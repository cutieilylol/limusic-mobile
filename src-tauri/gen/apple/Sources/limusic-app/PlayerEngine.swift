import AVFoundation
import MediaPlayer
import UIKit

/// One queued track: an AVPlayerItem that remembers the path Rust knows it by.
private final class TrackItem: AVPlayerItem {
    let path: String
    var watchers: [NSKeyValueObservation] = []

    init(path: String, asset: AVAsset) {
        self.path = path
        super.init(asset: asset, automaticallyLoadedAssetKeys: ["duration"])
    }
}

/// The iOS audio engine: AVQueuePlayer decks behind the same contract
/// `crates/player/src/mpv.rs` keeps with libmpv, so the Rust state machine above it runs unchanged.
///
/// - **Gapless**: the lookahead is appended to the playing deck's own queue.
/// - **Crossfade**: a second deck holds the next track paused and silent; `startCrossfade` swaps
///   which deck is live and ramps both with an equal-power (cos/sin) curve. `ended` goes out when
///   the overlap *starts*, as on desktop.
/// - **Pause survives a load**, like mpv's `pause` property: `load` never touches `wantsPlay`.
/// - Only the live deck's events reach Rust.
///
/// Loudness gain only turns tracks down for now: an AVPlayer's volume stops at 1, so a track that
/// wants boosting plays at its own level. Pitch and skip silence need an audio tap and aren't here
/// yet (`native.rs` says so; the setting is hidden).
final class PlayerEngine: NSObject {
    static let shared = PlayerEngine()

    private static let fadeStep = 0.05

    private var decks: [Deck] = []
    private var active = 0

    private var crossfadeSecs = 0.0
    /// The user's volume as a linear amplitude (the Rust side does the perceptual curve).
    fileprivate var userAmp: Float = 1
    fileprivate var speed: Float = 1
    private var gainDb = 0.0
    private var loopFile = false
    private var headers: [String: String] = [:]

    // Crossfade bookkeeping, mirroring `Decks` in mpv.rs.
    fileprivate var preloaded = false
    fileprivate var preloading = false
    fileprivate var preloadGen = 0
    fileprivate var preloadArmed = -1
    fileprivate var fading = false
    private var fadeGen = 0
    private var pending: String?

    private var sessionActive = false
    private var resumeAfterInterruption = false

    private var nowPlaying: [String: Any] = [:]
    private var artworkGen = 0

    private var live: Deck { decks[active] }

    private override init() {
        super.init()
        decks = [Deck(index: 0, engine: self), Deck(index: 1, engine: self)]
        observeSession()
        setUpRemoteCommands()
    }

    #if DEBUG
        func smokeDescription() -> String {
            let d = live
            let item = d.player.currentItem
            return "items=\(d.player.items().count) status=\(item?.status.rawValue ?? -1)"
                + " time=\(d.player.currentTime().seconds) dur=\(d.durationSecs)"
                + " rate=\(d.player.rate) tc=\(d.player.timeControlStatus.rawValue)"
                + " wait=\(d.player.reasonForWaitingToPlay?.rawValue ?? "-")"
                + " playing=\(d.playing) error=\(String(describing: item?.error))"
        }
    #endif

    // MARK: Commands from Rust

    func command(_ cmd: String, _ o: Json) {
        switch cmd {
        case "player.init":
            break
        case "player.load":
            load(
                o.str("url") ?? "", headers: o["headers"] as? [String: String] ?? [:],
                gain: o.num("gain"), start: o.num("start"))
        case "player.enqueue":
            enqueue(o.str("url") ?? "")
        case "player.crossfade":
            crossfadeSecs = o.num("secs") ?? 0
            if crossfadeSecs == 0 { dropPreload(cutFade: false) }
        case "player.clearPlaylist":
            live.player.items().dropFirst().forEach(live.player.remove)
            dropPreload(cutFade: false)
        case "player.stop":
            stop()
        case "player.play":
            play()
        case "player.pause":
            pause()
        case "player.toggle":
            if live.wantsPlay { pause() } else { play() }
        case "player.loop":
            loopFile = o.flag("on")
            decks.forEach { $0.player.actionAtItemEnd = loopFile ? .none : .advance }
        case "player.seek":
            seek(o.num("pos") ?? 0)
        case "player.volume":
            userAmp = Float(min(max(o.num("gain") ?? 1, 0), 1))
            if !fading { live.applyVolume() }
        case "player.gain":
            setGain(o.num("db") ?? 0)
        case "player.speed":
            speed = Float(min(max(o.num("speed") ?? 1, 0.25), 2))
            decks.forEach { $0.applySpeed() }
        case "player.pitch", "player.skipSilence":
            break  // See the class comment.
        case "media.metadata":
            setMetadata(o)
        case "media.album":
            nowPlaying[MPMediaItemPropertyAlbumTitle] = o.str("album") ?? ""
            publishNowPlaying()
        default:
            NSLog("PlayerEngine: unknown command %@", cmd)
        }
    }

    private func load(_ url: String, headers: [String: String], gain: Double?, start: Double?) {
        dropPreload(cutFade: true)
        self.headers = headers
        let d = live
        d.lastDuration = -1
        setGain(gain ?? 0)
        d.pendingSeek = start
        d.clear()
        d.append(makeItem(url))
        if d.wantsPlay {
            activateSession()
            d.applyRate()
        }
        d.reportPlaying()
    }

    private func enqueue(_ url: String) {
        if crossfadeSecs == 0 {
            live.append(makeItem(url))
        } else if fading {
            pending = url
        } else {
            preload(1 - active, url)
        }
    }

    private func makeItem(_ url: String) -> TrackItem {
        let target = url.contains("://")
            ? URL(string: url) ?? URL(fileURLWithPath: url) : URL(fileURLWithPath: url)
        var options: [String: Any] = [:]
        if !headers.isEmpty, target.scheme?.hasPrefix("http") == true {
            options["AVURLAssetHTTPHeaderFieldsKey"] = headers
        }
        let item = TrackItem(path: url, asset: AVURLAsset(url: target, options: options))
        // Tempo changes keep the pitch, like the desktop's rubberband and ExoPlayer's Sonic.
        item.audioTimePitchAlgorithm = .spectral
        return item
    }

    private func stop() {
        dropPreload(cutFade: true)
        let d = live
        d.player.pause()
        d.clear()
        d.reportPlaying()
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
        if sessionActive {
            sessionActive = false
            try? AVAudioSession.sharedInstance().setActive(
                false, options: .notifyOthersOnDeactivation)
        }
    }

    private func play() {
        activateSession()
        live.setWantsPlay(true)
    }

    private func pause() {
        cancelFade()
        live.setWantsPlay(false)
        resumeAfterInterruption = false
    }

    private func seek(_ pos: Double) {
        cancelFade()
        let d = live
        let time = CMTime(seconds: max(pos, 0), preferredTimescale: 1000)
        d.player.seek(to: time, toleranceBefore: .zero, toleranceAfter: .zero) { [weak self] _ in
            DispatchQueue.main.async {
                guard let self else { return }
                let now = d.player.currentTime().seconds
                if now.isFinite, self.isLive(d) { self.emit("position", d, now) }
                self.publishNowPlaying()
            }
        }
    }

    private func setGain(_ db: Double) {
        gainDb = db
        live.setGain(db)
    }

    // MARK: Decks

    fileprivate func isLive(_ d: Deck) -> Bool { decks[active] === d }

    private func preload(_ i: Int, _ url: String) {
        let d = decks[i]
        d.wantsPlay = false
        d.player.pause()
        d.player.volume = 0
        d.setGainQuietly(gainDb)
        d.lastDuration = -1
        preloadArmed = preloadGen
        preloaded = false
        preloading = true
        d.clear()
        d.append(makeItem(url))
    }

    private func dropPreload(cutFade: Bool) {
        pending = nil
        preloadGen += 1
        if cutFade { self.cancelFade() }
        if (preloaded || preloading) && !fading {
            let d = decks[1 - active]
            d.player.pause()
            d.clear()
        }
        preloaded = false
        preloading = false
    }

    private func cancelFade() {
        guard fading else { return }
        fadeGen += 1
        decks[1 - active].player.pause()
    }

    /// See `fade_due` in mpv.rs: never longer than half the track, nor than what is left.
    private func fadeDue(_ setting: Double, pos: Double, dur: Double) -> Double? {
        guard setting > 0, pos.isFinite, dur.isFinite, dur > 1 else { return nil }
        let left = dur - pos
        let fade = min(setting, dur / 2)
        return left <= fade ? max(0, min(fade, left)) : nil
    }

    fileprivate func tick(_ d: Deck, position pos: Double) {
        guard isLive(d) else { return }
        emit("position", d, pos)
        guard !loopFile, !fading, preloaded, crossfadeSecs > 0,
            let fade = fadeDue(crossfadeSecs, pos: pos, dur: d.durationSecs)
        else { return }
        startCrossfade(from: d.index, fadeIn: fade)
    }

    private func startCrossfade(from: Int, fadeIn: Double) {
        let out = decks[from]
        let inc = decks[1 - from]
        preloaded = false
        preloading = false
        let gen = fadeGen
        fading = true
        active = 1 - from
        inc.setWantsPlay(true)
        inc.player.volume = 0
        inc.lastDuration = -1
        emit("ended", inc)
        var fade = fadeIn
        let incoming = inc.durationSecs
        if incoming > 0 {
            inc.lastDuration = incoming
            emit("duration", inc, incoming)
            fade = min(fade, incoming / 2)
        }
        publishNowPlaying()
        let steps = max(1, Int((fade / PlayerEngine.fadeStep).rounded()))
        var step = 0
        func ramp() {
            guard fadeGen == gen else {
                finishFade(out, inc, cancelled: true)
                return
            }
            step += 1
            let turn = Double(step) / Double(steps) * (Double.pi / 2)
            out.player.volume = out.volumeTarget * Float(max(cos(turn), 0))
            inc.player.volume = inc.volumeTarget * Float(max(sin(turn), 0))
            if step >= steps {
                finishFade(out, inc, cancelled: false)
            } else {
                DispatchQueue.main.asyncAfter(deadline: .now() + PlayerEngine.fadeStep) { ramp() }
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + PlayerEngine.fadeStep) { ramp() }
    }

    private func finishFade(_ out: Deck, _ inc: Deck, cancelled: Bool) {
        out.player.pause()
        out.clear()
        out.wantsPlay = false
        out.playing = false
        out.applyVolume()
        if cancelled { inc.applyVolume() }
        fading = false
        let next = pending
        pending = nil
        if let next, crossfadeSecs > 0 { preload(1 - active, next) }
    }

    // MARK: Events to Rust

    fileprivate func emit(_ ev: String, _ d: Deck, _ value: Any? = nil, msg: String? = nil) {
        var o: Json = [
            "ev": ev, "idle": d.isIdle,
            "path": (d.player.currentItem as? TrackItem)?.path ?? NSNull(),
        ]
        if let value { o["v"] = value }
        if let msg { o["msg"] = msg }
        LimusicBridge.emit("player", o)
    }

    fileprivate func itemFinished(_ d: Deck) {
        if loopFile && isLive(d) {
            d.player.seek(to: .zero, toleranceBefore: .zero, toleranceAfter: .zero)
            d.applyRate()
            return
        }
        guard isLive(d) else { return }
        d.lastDuration = -1
        emit("ended", d)
        d.reportDuration()
        d.reportPlaying()
        publishNowPlaying()
    }

    fileprivate func itemFailed(_ d: Deck, _ item: TrackItem, _ error: Error?) {
        let msg = friendly(error)
        NSLog("PlayerEngine: item failed: %@", error?.localizedDescription ?? "unknown")
        if isLive(d) {
            if d.player.currentItem === item {
                emit("failed", d, msg: msg)
            } else {
                d.player.remove(item)
                emit("lookaheadFailed", d, msg: msg)
            }
        } else if !fading {
            preloaded = false
            preloading = false
            emit("lookaheadFailed", d, msg: msg)
        }
        d.reportPlaying()
    }

    private func friendly(_ error: Error?) -> String {
        let e = error as NSError?
        if e?.domain == NSURLErrorDomain {
            return "Couldn't load this track. The stream link was refused."
        }
        if e?.domain == AVFoundationErrorDomain,
            e?.code == AVError.fileFormatNotRecognized.rawValue
                || e?.code == AVError.decodeFailed.rawValue
                || e?.code == AVError.failedToLoadMediaData.rawValue
        {
            return "Unrecognized audio format"
        }
        return "Playback failed (\(e?.domain ?? "unknown") \(e?.code ?? 0))"
    }

    // MARK: Audio session

    fileprivate func activateSession() {
        guard !sessionActive else { return }
        do {
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.playback, mode: .default)
            try session.setActive(true)
            sessionActive = true
            UIApplication.shared.beginReceivingRemoteControlEvents()
        } catch {
            NSLog("PlayerEngine: audio session failed: %@", error.localizedDescription)
        }
    }

    private func observeSession() {
        let center = NotificationCenter.default
        center.addObserver(
            self, selector: #selector(interrupted(_:)),
            name: AVAudioSession.interruptionNotification, object: nil)
        center.addObserver(
            self, selector: #selector(routeChanged(_:)),
            name: AVAudioSession.routeChangeNotification, object: nil)
    }

    /// A call, a voice assistant: pause, and pick up again when it is over.
    @objc private func interrupted(_ note: Notification) {
        guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
            let type = AVAudioSession.InterruptionType(rawValue: raw)
        else { return }
        let options = note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0
        DispatchQueue.main.async { [self] in
            switch type {
            case .began:
                resumeAfterInterruption = live.wantsPlay
                cancelFade()
                live.setWantsPlay(false)
                sessionActive = false
            case .ended:
                if resumeAfterInterruption,
                    AVAudioSession.InterruptionOptions(rawValue: options).contains(.shouldResume)
                {
                    play()
                }
                resumeAfterInterruption = false
            @unknown default:
                break
            }
        }
    }

    /// Headphones out: pause, as every music app does.
    @objc private func routeChanged(_ note: Notification) {
        guard let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
            AVAudioSession.RouteChangeReason(rawValue: raw) == .oldDeviceUnavailable
        else { return }
        DispatchQueue.main.async { [self] in
            cancelFade()
            live.setWantsPlay(false)
        }
    }

    // MARK: Lock screen and Control Center

    /// Every button goes to the Rust queue, not the deck: the deck only ever holds "current +
    /// next", so "next" has to mean the app's next, with shuffle, radio and autoplay included.
    private func setUpRemoteCommands() {
        let center = MPRemoteCommandCenter.shared()
        func bind(_ command: MPRemoteCommand, _ action: String) {
            command.addTarget { _ in
                LimusicBridge.emit("remote", ["action": action])
                return .success
            }
        }
        bind(center.playCommand, "play")
        bind(center.pauseCommand, "pause")
        bind(center.togglePlayPauseCommand, "toggle")
        bind(center.nextTrackCommand, "next")
        bind(center.previousTrackCommand, "prev")
        center.changePlaybackPositionCommand.addTarget { event in
            guard let event = event as? MPChangePlaybackPositionCommandEvent else {
                return .commandFailed
            }
            LimusicBridge.emit("remote", ["action": "seek", "pos": event.positionTime])
            return .success
        }
    }

    private func setMetadata(_ o: Json) {
        artworkGen += 1
        let gen = artworkGen
        nowPlaying = [
            MPMediaItemPropertyTitle: o.str("title") ?? "",
            MPMediaItemPropertyArtist: o.str("artist") ?? "",
        ]
        if let album = o.str("album") { nowPlaying[MPMediaItemPropertyAlbumTitle] = album }
        publishNowPlaying()
        guard let cover = o.str("cover"), cover.hasPrefix("http"), let url = URL(string: cover)
        else { return }
        URLSession.shared.dataTask(with: url) { [weak self] data, _, _ in
            guard let data, let image = UIImage(data: data) else { return }
            DispatchQueue.main.async {
                guard let self, self.artworkGen == gen else { return }
                self.nowPlaying[MPMediaItemPropertyArtwork] = MPMediaItemArtwork(
                    boundsSize: image.size, requestHandler: { _ in image })
                self.publishNowPlaying()
            }
        }.resume()
    }

    fileprivate func publishNowPlaying() {
        guard !nowPlaying.isEmpty else { return }
        let d = live
        var info = nowPlaying
        let position = d.player.currentTime().seconds
        info[MPNowPlayingInfoPropertyElapsedPlaybackTime] = position.isFinite ? position : 0
        info[MPNowPlayingInfoPropertyPlaybackRate] = d.playing ? Double(speed) : 0
        if d.durationSecs > 0 { info[MPMediaItemPropertyPlaybackDuration] = d.durationSecs }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
        MPNowPlayingInfoCenter.default().playbackState = d.playing ? .playing : .paused
    }

    // MARK: One deck

    fileprivate final class Deck: NSObject {
        let index: Int
        let player = AVQueuePlayer()
        unowned let engine: PlayerEngine

        var lastDuration = -1.0
        var wantsPlay = false
        var playing = false
        var pendingSeek: Double?
        /// Loudness gain as an amplitude, capped at 1: see the class comment.
        var gainAmp: Float = 1

        private var timeObserver: Any?

        init(index: Int, engine: PlayerEngine) {
            self.index = index
            self.engine = engine
            super.init()
            player.defaultRate = engine.speed
            player.actionAtItemEnd = .advance
            timeObserver = player.addPeriodicTimeObserver(
                forInterval: CMTime(seconds: 0.25, preferredTimescale: 600), queue: .main
            ) { [weak self] time in
                guard let self, self.playing, time.seconds.isFinite else { return }
                self.engine.tick(self, position: time.seconds)
            }
        }

        var isIdle: Bool { player.currentItem == nil }

        var durationSecs: Double {
            let d = player.currentItem?.duration.seconds ?? -1
            return d.isFinite && d > 0 ? d : -1
        }

        var volumeTarget: Float { engine.userAmp * gainAmp }

        func applyVolume() { player.volume = volumeTarget }

        func setGainQuietly(_ db: Double) {
            gainAmp = db.isFinite ? Float(min(pow(10, db / 20), 1)) : 1
        }

        func setGain(_ db: Double) {
            setGainQuietly(db)
            if !engine.fading { applyVolume() }
        }

        func applySpeed() {
            player.defaultRate = engine.speed
            if wantsPlay && player.rate != 0 { player.rate = engine.speed }
        }

        func applyRate() {
            if wantsPlay && !isIdle { player.play() }
        }

        func setWantsPlay(_ want: Bool) {
            if want != wantsPlay {
                wantsPlay = want
                if engine.isLive(self) { engine.emit("paused", self, !want) }
            }
            if want {
                engine.activateSession()
                applyVolume()
                applyRate()
            } else {
                player.pause()
            }
            reportPlaying()
        }

        func reportPlaying() {
            let now = wantsPlay && !isIdle
            if now != playing {
                playing = now
                if engine.isLive(self) {
                    engine.emit("playing", self, now)
                    engine.publishNowPlaying()
                }
            }
        }

        func reportDuration() {
            let d = durationSecs
            if d > 0, d != lastDuration, engine.isLive(self) {
                lastDuration = d
                engine.emit("duration", self, d)
                engine.publishNowPlaying()
            }
        }

        func append(_ item: TrackItem) {
            item.watchers.append(
                item.observe(\.status) { [weak self] item, _ in
                    DispatchQueue.main.async { self?.statusChanged(item) }
                })
            item.watchers.append(
                item.observe(\.duration) { [weak self] _, _ in
                    DispatchQueue.main.async { self?.reportDuration() }
                })
            let center = NotificationCenter.default
            center.addObserver(
                self, selector: #selector(finished(_:)),
                name: .AVPlayerItemDidPlayToEndTime, object: item)
            center.addObserver(
                self, selector: #selector(failedMidway(_:)),
                name: .AVPlayerItemFailedToPlayToEndTime, object: item)
            player.insert(item, after: nil)
        }

        func clear() {
            player.items().compactMap { $0 as? TrackItem }.forEach(forget)
            player.removeAllItems()
        }

        private func forget(_ item: TrackItem) {
            item.watchers.removeAll()
            let center = NotificationCenter.default
            center.removeObserver(self, name: .AVPlayerItemDidPlayToEndTime, object: item)
            center.removeObserver(self, name: .AVPlayerItemFailedToPlayToEndTime, object: item)
        }

        private func statusChanged(_ item: TrackItem) {
            switch item.status {
            case .readyToPlay:
                if !engine.isLive(self), engine.preloading,
                    engine.preloadArmed == engine.preloadGen
                {
                    engine.preloading = false
                    engine.preloaded = true
                }
                if let start = pendingSeek, player.currentItem === item {
                    pendingSeek = nil
                    item.seek(
                        to: CMTime(seconds: start, preferredTimescale: 1000),
                        completionHandler: nil)
                }
                reportDuration()
            case .failed:
                forget(item)
                engine.itemFailed(self, item, item.error)
            default:
                break
            }
        }

        @objc private func finished(_ note: Notification) {
            guard let item = note.object as? TrackItem else { return }
            DispatchQueue.main.async { [self] in settle(item, tries: 0) }
        }

        /// The notification comes before the queue has moved on, and not always on the main
        /// thread. Waits that out, so the idle flag and path in the `ended` event describe what
        /// plays next.
        private func settle(_ item: TrackItem, tries: Int) {
            if player.currentItem === item, player.actionAtItemEnd == .advance, tries < 20 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.01) {
                    self.settle(item, tries: tries + 1)
                }
                return
            }
            if player.currentItem !== item { forget(item) }
            engine.itemFinished(self)
        }

        @objc private func failedMidway(_ note: Notification) {
            guard let item = note.object as? TrackItem else { return }
            let error = note.userInfo?[AVPlayerItemFailedToPlayToEndTimeErrorKey] as? Error
            DispatchQueue.main.async { [self] in
                forget(item)
                engine.itemFailed(self, item, error)
            }
        }
    }
}
