import UIKit
import WebKit

/// Off-screen webviews for the Rust side's JS harness (`src-tauri/src/mobile/webview.rs`): the
/// cipher deobfuscator runs YouTube's own player code in one. Tauri gives a phone exactly one
/// webview, the UI, so these are plain WKWebViews. Each sits in the window as an invisible speck,
/// because WebKit throttles a webview that isn't in one.
enum HiddenWebViews {
    private static var views: [String: WKWebView] = [:]

    static func command(_ cmd: String, _ o: Json) {
        let label = o.str("label") ?? ""
        switch cmd {
        case "webview.create":
            create(label, html: o.str("html") ?? "")
        case "webview.eval":
            guard let web = views[label] else {
                gone(label)
                return
            }
            web.evaluateJavaScript(o.str("js") ?? "", completionHandler: nil)
        case "webview.evalJson":
            let id = o["id"] as? NSNumber ?? 0
            guard let web = views[label] else {
                gone(label)
                result(id, "null")
                return
            }
            web.evaluateJavaScript(jsonEval(o.str("js") ?? "")) { value, _ in
                result(id, value as? String ?? "null")
            }
        case "webview.destroy":
            destroy(label)
        default:
            NSLog("HiddenWebViews: unknown command %@", cmd)
        }
    }

    private static func create(_ label: String, html: String) {
        destroy(label)
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        let web = WKWebView(frame: CGRect(x: 0, y: 0, width: 1, height: 1), configuration: config)
        web.tag = AppChrome.hiddenTag
        web.alpha = 0.01
        web.isUserInteractionEnabled = false
        AppChrome.shared.window?.addSubview(web)
        web.loadHTMLString(html, baseURL: URL(string: "https://localhost/"))
        views[label] = web
    }

    private static func destroy(_ label: String) {
        guard let web = views.removeValue(forKey: label) else { return }
        web.stopLoading()
        web.removeFromSuperview()
    }

    /// Runs `js` as a script and answers the JSON encoding of its completion value, the contract
    /// the Rust side has from Android's `evaluateJavascript`. WebKit can't hand back an undefined
    /// or a function at all, so the encoding happens on the page.
    private static func jsonEval(_ js: String) -> String {
        let quoted = (try? JSONSerialization.data(withJSONObject: [js]))
            .flatMap { String(data: $0, encoding: .utf8) }
            .map { String($0.dropFirst().dropLast()) } ?? "\"null\""
        return "(function(){var s=JSON.stringify((0,eval)(\(quoted)));return s===undefined?'null':s;})()"
    }

    private static func result(_ id: NSNumber, _ json: String) {
        LimusicBridge.emit("webview", ["ev": "result", "id": id, "json": json])
    }

    private static func gone(_ label: String) {
        LimusicBridge.emit("webview", ["ev": "gone", "label": label])
    }
}
