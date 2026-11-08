import AuthenticationServices
import UIKit
import WebKit

/// The C ABI seam between the Rust core and the iOS side (Rust half: crates/player/src/bridge.rs).
/// Rust calls the receiver from any thread with one JSON object; the work is posted to the main
/// queue, in order, and the call returns at once. Events go back through `limusic_bridge_event`,
/// tagged with a `topic` the Rust side routes on.
@_silgen_name("limusic_bridge_connect")
private func limusic_bridge_connect(_ receiver: @convention(c) (UnsafePointer<CChar>?) -> Void)

@_silgen_name("limusic_bridge_event")
private func limusic_bridge_event(_ json: UnsafePointer<CChar>?)

/// Called from main.mm, before `start_app`.
@_cdecl("limusic_native_start")
public func limusicNativeStart() {
    limusic_bridge_connect { json in
        guard let json else { return }
        let text = String(cString: json)
        DispatchQueue.main.async { LimusicBridge.dispatch(text) }
    }
    DispatchQueue.main.async { AppChrome.shared.install() }
}

typealias Json = [String: Any]

extension Dictionary where Key == String, Value == Any {
    func str(_ key: String) -> String? { self[key] as? String }
    func num(_ key: String) -> Double? { (self[key] as? NSNumber)?.doubleValue }
    func flag(_ key: String) -> Bool { (self[key] as? NSNumber)?.boolValue ?? false }
}

enum LimusicBridge {
    static func emit(_ topic: String, _ object: Json) {
        var o = object
        o["topic"] = topic
        guard JSONSerialization.isValidJSONObject(o),
            let data = try? JSONSerialization.data(withJSONObject: o),
            let text = String(data: data, encoding: .utf8)
        else {
            NSLog("LimusicBridge: unserializable %@ event", topic)
            return
        }
        text.withCString { limusic_bridge_event($0) }
    }

    fileprivate static func dispatch(_ text: String) {
        guard let data = text.data(using: .utf8),
            let o = (try? JSONSerialization.jsonObject(with: data)) as? Json,
            let cmd = o.str("cmd")
        else {
            NSLog("LimusicBridge: bad command json")
            return
        }
        if cmd.hasPrefix("player.") || cmd.hasPrefix("media.") {
            PlayerEngine.shared.command(cmd, o)
        } else if cmd.hasPrefix("webview.") {
            HiddenWebViews.command(cmd, o)
        } else if cmd.hasPrefix("login.") {
            LoginFlow.command(cmd, o)
        } else if cmd.hasPrefix("app.") {
            AppChrome.shared.command(cmd, o)
        } else {
            NSLog("LimusicBridge: unknown command %@", cmd)
        }
    }
}

/// The app's window and everything hanging off it: the `app.*` commands, and the two things the
/// webview needs from UIKit (edge-to-edge drawing, and making room for the keyboard).
final class AppChrome: NSObject, ASWebAuthenticationPresentationContextProviding {
    static let shared = AppChrome()

    /// Marks the off-screen webviews, so the search for the UI's own one skips them.
    static let hiddenTag = 7_227

    private var webConfigured = false
    private var authSession: ASWebAuthenticationSession?

    var window: UIWindow? {
        let windows = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
        return windows.first(where: \.isKeyWindow) ?? windows.first
    }

    var topController: UIViewController? {
        var controller = window?.rootViewController
        while let presented = controller?.presentedViewController { controller = presented }
        return controller
    }

    func install() {
        NotificationCenter.default.addObserver(
            self, selector: #selector(keyboardChanged(_:)),
            name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        configureWebViewWhenReady(attempt: 0)
        #if DEBUG
            runSmokeSteps()
        #endif
    }

    #if DEBUG
        /// The iOS workflow's Simulator run (`LIMUSIC_SMOKE=1`): nobody is there to tap, so this
        /// measures the layout, starts a song and walks the tabs, logging what it finds.
        private func runSmokeSteps() {
            guard ProcessInfo.processInfo.environment["LIMUSIC_SMOKE"] != nil else { return }
            let go = { (path: String) in
                "(()=>{const a=document.createElement('a');a.href='\(path)';document.body.appendChild(a);a.click();a.remove();return location.pathname})()"
            }
            let layout = """
                JSON.stringify((()=>{const r=e=>{if(!e)return null;const b=e.getBoundingClientRect();return [Math.round(b.top),Math.round(b.bottom)]};\
                const p=document.createElement('div');p.style.cssText='position:fixed;top:0;left:0;width:1px;height:100dvh;padding-bottom:env(safe-area-inset-bottom);padding-top:env(safe-area-inset-top);box-sizing:content-box';\
                document.body.appendChild(p);const c=getComputedStyle(p);const o={dvh:p.clientHeight,top:c.paddingTop,bottom:c.paddingBottom};p.remove();\
                return {ih:innerHeight,vv:visualViewport.height,ch:document.documentElement.clientHeight,probe:o,shell:r(document.querySelector('.h-dvh')),\
                main:r(document.querySelector('main')),nav:r(document.querySelector('nav')),body:r(document.body),meta:document.querySelector('meta[name=viewport]').content,cls:document.documentElement.className}})())
                """
            let state = "JSON.stringify({path:location.pathname,text:document.body.innerText.slice(0,300)})"
            let steps: [(Double, String)] = [
                (20, layout),
                (22, "(()=>{const r=document.querySelector('main [data-ctx][role=button]');r&&r.click();return r?r.innerText.slice(0,60):'no row'})()"),
                (36, state),
                (46, state),
                (47, "(()=>{const b=document.querySelector('button[aria-label=\"Minimize player\"]');b&&b.click();return !!b})()"),
                (50, go("/search")),
                (55, go("/library")),
                (60, state),
            ]
            for (delay, js) in steps {
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                    guard let web = self?.mainWebView() else {
                        NSLog("SMOKE no webview")
                        return
                    }
                    NSLog(
                        "SMOKE t=%.0f frame=%@ super=%@ safe=%@", delay,
                        NSCoder.string(for: web.frame),
                        NSCoder.string(for: web.superview?.bounds ?? .zero),
                        NSCoder.string(for: web.safeAreaInsets))
                    web.evaluateJavaScript(js) { value, error in
                        NSLog(
                            "SMOKE t=%.0f result=%@ error=%@", delay,
                            String(describing: value), String(describing: error))
                    }
                }
            }
        }
    #endif

    /// The webview arrives from wry some time after launch; there is nothing to be notified by.
    private func configureWebViewWhenReady(attempt: Int) {
        if let web = mainWebView() {
            guard !webConfigured else { return }
            webConfigured = true
            // Edge to edge: the page pads itself with `env(safe-area-inset-*)`. The shell is
            // exactly one screen tall with its own scrollers inside, so the page itself never
            // scrolls or rubber-bands.
            web.scrollView.contentInsetAdjustmentBehavior = .never
            web.scrollView.isScrollEnabled = false
            web.scrollView.bounces = false
            web.allowsBackForwardNavigationGestures = true
            web.allowsLinkPreview = false
        } else if attempt < 100 {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                self.configureWebViewWhenReady(attempt: attempt + 1)
            }
        }
    }

    private func mainWebView() -> WKWebView? {
        func search(_ view: UIView) -> WKWebView? {
            if let web = view as? WKWebView, web.tag != AppChrome.hiddenTag { return web }
            for child in view.subviews {
                if let found = search(child) { return found }
            }
            return nil
        }
        return window.flatMap { search($0) }
    }

    /// The webview shrinks to what the keyboard leaves, so a focused field and the sheet or dialog
    /// it sits in stay above it: `100dvh` alone doesn't follow the keyboard on iOS.
    @objc private func keyboardChanged(_ note: Notification) {
        guard topController === window?.rootViewController,
            let web = mainWebView(), let host = web.superview, let window = web.window,
            let end = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect
        else { return }
        let covered = max(0, window.bounds.maxY - window.screen.coordinateSpace.convert(end, to: window).minY)
        var frame = host.bounds
        frame.size.height -= covered
        web.frame = frame
        web.evaluateJavaScript(
            "document.documentElement.classList.toggle('keyboard-open',\(covered > 0));")
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        window ?? ASPresentationAnchor()
    }

    func command(_ cmd: String, _ o: Json) {
        switch cmd {
        case "app.openUrl":
            guard let url = URL(string: o.str("url") ?? "") else { return }
            if url.absoluteString.contains("redirect_uri=limusicdiscord") {
                discordSignIn(url)
            } else {
                UIApplication.shared.open(url)
            }
        case "app.share":
            let sheet = UIActivityViewController(
                activityItems: [o.str("text") ?? ""], applicationActivities: nil)
            if let top = topController {
                sheet.popoverPresentationController?.sourceView = top.view
                sheet.popoverPresentationController?.sourceRect = CGRect(
                    x: top.view.bounds.midX, y: top.view.bounds.maxY, width: 0, height: 0)
                top.present(sheet, animated: true)
            }
        case "app.equalizer":
            LimusicBridge.emit("app", ["ev": "equalizer", "ok": false])
        case "app.keepScreenOn":
            UIApplication.shared.isIdleTimerDisabled = o.flag("on")
        case "app.statusBar":
            // The bar's text follows the window's style: the app's own theme, not the phone's.
            window?.overrideUserInterfaceStyle =
                (o["dark"] as? NSNumber)?.boolValue ?? true ? .dark : .light
        case "app.haptic":
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        case "app.audioPermission":
            LimusicBridge.emit(
                "app", ["ev": "audioPermission", "granted": true, "music": musicFolder()])
        case "app.immersive", "app.bringToFront", "app.background", "app.insets":
            break  // Android's. An iPhone app has no back button to leave by or bars to hide.
        default:
            NSLog("LimusicBridge: unknown app command %@", cmd)
        }
    }

    /// iOS has no shared Music folder. The app's own Documents/Music is what Local Music watches,
    /// and it shows up in the Files app (`UIFileSharingEnabled`) for songs to be copied into.
    private func musicFolder() -> String {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let music = docs.appendingPathComponent("Music", isDirectory: true)
        try? FileManager.default.createDirectory(at: music, withIntermediateDirectories: true)
        return music.path
    }

    /// Discord's page in a sheet that hands the redirect straight back, in place of the Android
    /// build's browser plus intent filter. Answers the same `discord` callback event.
    private func discordSignIn(_ url: URL) {
        let session = ASWebAuthenticationSession(url: url, callbackURLScheme: "limusicdiscord") {
            [weak self] callback, error in
            DispatchQueue.main.async {
                self?.authSession = nil
                let items =
                    callback.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }?
                    .queryItems ?? []
                let value = { (name: String) in items.first { $0.name == name }?.value ?? "" }
                let failure = error == nil ? value("error") : "cancelled"
                LimusicBridge.emit(
                    "discord",
                    [
                        "ev": "callback", "code": value("code"), "state": value("state"),
                        "error": failure,
                    ])
            }
        }
        session.presentationContextProvider = self
        authSession = session
        session.start()
    }
}
