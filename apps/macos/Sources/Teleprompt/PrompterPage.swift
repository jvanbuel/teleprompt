#if canImport(SwiftUI)
import AppKit
import SwiftUI
import WebKit

/// The prompter: the page `teleprompt prompt` serves, in WebKit. The page
/// does the prompting; this gives it the microphone, and nothing else, and
/// opens its screen in a window of its own when it asks.
struct PrompterPage: NSViewRepresentable {
    let page: URL
    let model: AppModel

    func makeCoordinator() -> Coordinator {
        Coordinator(page: page, model: model)
    }

    func makeNSView(context: Context) -> WKWebView {
        let view = WKWebView(frame: .zero, configuration: Self.configuration())
        view.uiDelegate = context.coordinator
        view.navigationDelegate = context.coordinator
        // Black while it loads, as the glass is.
        view.underPageBackgroundColor = .black
        view.load(URLRequest(url: page))
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        guard context.coordinator.page != page else { return }
        context.coordinator.page = page
        view.load(URLRequest(url: page))
    }

    static func dismantleNSView(_ view: WKWebView, coordinator: Coordinator) {
        // The page lets go of the microphone as it goes.
        view.load(URLRequest(url: URL(string: "about:blank")!))
    }

    /// Shot clips play without a click.
    static func configuration() -> WKWebViewConfiguration {
        let configuration = WKWebViewConfiguration()
        configuration.mediaTypesRequiringUserActionForPlayback = []
        return configuration
    }

    @MainActor
    final class Coordinator: NSObject, WKUIDelegate, WKNavigationDelegate {
        var page: URL
        let model: AppModel

        init(page: URL, model: AppModel) {
            self.page = page
            self.model = model
        }

        /// Whether `origin` is the server's: the only page the app shows.
        private func ours(_ origin: WKSecurityOrigin) -> Bool {
            origin.protocol == page.scheme && origin.host == page.host && origin.port == (page.port ?? 0)
        }

        func webView(
            _ webView: WKWebView,
            requestMediaCapturePermissionFor origin: WKSecurityOrigin,
            initiatedByFrame frame: WKFrameInfo,
            type: WKMediaCaptureType,
            decisionHandler: @escaping (WKPermissionDecision) -> Void
        ) {
            // The microphone, for the server's page; the app's own Info.plist
            // has macOS ask the author first.
            decisionHandler(ours(origin) && type == .microphone ? .grant : .deny)
        }

        /// The page's screen, which it opened: in a window of its own, for
        /// a second display.
        func webView(
            _ webView: WKWebView,
            createWebViewWith configuration: WKWebViewConfiguration,
            for navigationAction: WKNavigationAction,
            windowFeatures: WKWindowFeatures
        ) -> WKWebView? {
            let screen = WKWebView(frame: NSRect(x: 0, y: 0, width: 800, height: 520), configuration: configuration)
            screen.uiDelegate = self
            let window = NSWindow(
                contentRect: screen.frame,
                styleMask: [.titled, .closable, .resizable, .miniaturizable],
                backing: .buffered,
                defer: false
            )
            window.title = "Teleprompt · Screen"
            window.contentView = screen
            window.isReleasedWhenClosed = false
            window.center()
            window.makeKeyAndOrderFront(nil)
            model.screens.append(window)
            return screen
        }

        func webViewDidClose(_ webView: WKWebView) {
            webView.window?.close()
        }

        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
            model.pageFailed(error.localizedDescription)
        }

        /// Only the server's page: anything else opens in the browser.
        func webView(
            _ webView: WKWebView,
            decidePolicyFor navigationAction: WKNavigationAction,
            decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
        ) {
            guard let url = navigationAction.request.url else { return decisionHandler(.cancel) }
            if url.host == page.host && url.port == page.port || url.scheme == "about" {
                decisionHandler(.allow)
            } else {
                NSWorkspace.shared.open(url)
                decisionHandler(.cancel)
            }
        }
    }
}
#endif
