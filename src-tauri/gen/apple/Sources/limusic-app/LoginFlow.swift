import UIKit
import WebKit

/// Google sign-in, the iOS half of `src-tauri/src/mobile/session.rs`.
///
/// `LoginViewController` is the desktop login window as a full screen: Google's own sign-in page in
/// a webview, and once it lands on music.youtube.com the youtube.com cookie jar goes to Rust. The
/// session refresh is the desktop's hidden refresh window: music.youtube.com loaded off screen so
/// Google can roll the short-lived cookies, then the jar is read again.
///
/// Both use the default data store, which persists, so the Google session survives restarts
/// exactly like the desktop login webview's profile.
enum LoginFlow {
    /// Google's login page refuses what it recognises as an embedded webview. A WKWebView's own
    /// user agent is Safari's minus the `Version/… Safari/…` tail, so this is Safari's in full.
    static let userAgent =
        "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 "
        + "(KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1"

    fileprivate static weak var current: LoginViewController?
    private static var refresher: WKWebView?
    private static var refreshDelegate: RefreshDelegate?

    static func command(_ cmd: String, _ o: Json) {
        switch cmd {
        case "login.open":
            let open = { present(url: o.str("url") ?? "https://accounts.google.com/") }
            if o.flag("clear") { clearCookies(then: open) } else { open() }
        case "login.close":
            current?.finishQuietly()
        case "login.refresh":
            refresh(url: o.str("url") ?? "")
        case "login.refreshDone":
            refresher?.removeFromSuperview()
            refresher = nil
            refreshDelegate = nil
        default:
            NSLog("LoginFlow: unknown command %@", cmd)
        }
    }

    /// The cookies only: wiping the store's other data would take the app UI's own storage with it.
    private static func clearCookies(then done: @escaping () -> Void) {
        let store = WKWebsiteDataStore.default().httpCookieStore
        store.getAllCookies { cookies in
            let group = DispatchGroup()
            for cookie in cookies {
                group.enter()
                store.delete(cookie) { group.leave() }
            }
            group.notify(queue: .main, execute: done)
        }
    }

    private static func present(url: String) {
        let login = LoginViewController(url: URL(string: url))
        current = login
        let nav = UINavigationController(rootViewController: login)
        nav.modalPresentationStyle = .fullScreen
        AppChrome.shared.topController?.present(nav, animated: true)
    }

    /// The `name=value; …` header for music.youtube.com, HttpOnly cookies included.
    static func youtubeCookies(_ done: @escaping (String) -> Void) {
        WKWebsiteDataStore.default().httpCookieStore.getAllCookies { cookies in
            let header = cookies.filter { cookie in
                let domain = cookie.domain.hasPrefix(".")
                    ? String(cookie.domain.dropFirst()) : cookie.domain
                return (domain == "music.youtube.com" || domain == "youtube.com")
                    && (cookie.path.isEmpty || cookie.path == "/")
            }
            .map { "\($0.name)=\($0.value)" }
            .joined(separator: "; ")
            done(header)
        }
    }

    private final class RefreshDelegate: NSObject, WKNavigationDelegate {
        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            guard webView.url?.host == "music.youtube.com" else { return }
            // The rolled cookies can land a moment after the page; offer the jar a few times and
            // let the Rust side pick the first one that changed (session.rs).
            for i in 0..<8 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5 * Double(i)) {
                    guard LoginFlow.refresher === webView else { return }
                    LoginFlow.youtubeCookies { cookie in
                        LimusicBridge.emit("login", ["ev": "refresh", "cookie": cookie])
                    }
                }
            }
        }
    }

    private static func refresh(url: String) {
        refresher?.removeFromSuperview()
        let web = WKWebView(frame: CGRect(x: 0, y: 0, width: 1, height: 1))
        web.customUserAgent = userAgent
        web.tag = AppChrome.hiddenTag
        web.alpha = 0.01
        web.isUserInteractionEnabled = false
        AppChrome.shared.window?.addSubview(web)
        let delegate = RefreshDelegate()
        web.navigationDelegate = delegate
        refresher = web
        refreshDelegate = delegate
        if let target = URL(string: url) { web.load(URLRequest(url: target)) }
    }
}

final class LoginViewController: UIViewController, WKNavigationDelegate {
    private let url: URL?
    private let web = WKWebView()
    private var reported = false

    init(url: URL?) {
        self.url = url
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a storyboard") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        navigationItem.leftBarButtonItem = UIBarButtonItem(
            barButtonSystemItem: .cancel, target: self, action: #selector(cancel))
        web.customUserAgent = LoginFlow.userAgent
        web.navigationDelegate = self
        web.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(web)
        NSLayoutConstraint.activate([
            web.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            web.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            web.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            web.trailingAnchor.constraint(equalTo: view.trailingAnchor),
        ])
        if let url { web.load(URLRequest(url: url)) }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        if webView.url?.host == "music.youtube.com" { pollCookies(attempt: 0) }
    }

    /// The session cookies can land a moment after music.youtube.com finishes loading, so the jar
    /// is read a few times (the desktop login does the same, session.rs).
    private func pollCookies(attempt: Int) {
        guard !reported else { return }
        LoginFlow.youtubeCookies { [weak self] cookie in
            DispatchQueue.main.async {
                guard let self, !self.reported else { return }
                if cookie.contains("SAPISID=") {
                    self.reported = true
                    LimusicBridge.emit("login", ["ev": "cookies", "cookie": cookie])
                } else if attempt < 8 {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                        self.pollCookies(attempt: attempt + 1)
                    }
                }
            }
        }
    }

    @objc private func cancel() {
        if !reported { LimusicBridge.emit("login", ["ev": "closed"]) }
        reported = true
        dismiss(animated: true)
    }

    func finishQuietly() {
        reported = true
        dismiss(animated: true)
    }
}
