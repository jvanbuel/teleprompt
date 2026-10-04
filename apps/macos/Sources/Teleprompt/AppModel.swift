#if canImport(SwiftUI)
import AppKit
import SwiftUI
import TelepromptKit
import WebKit

/// The app's state: the server it launched, and where its page is. The
/// welcome, setup and the prompter are that page (`PrompterPage`), here as
/// in a browser.
@MainActor
final class AppModel: ObservableObject {
    enum Phase: Equatable {
        /// The server is starting.
        case launching
        /// The server listens: its page, to show.
        case ready(page: URL)
        /// The server stopped or never started, and why.
        case failed([String])
    }

    @Published private(set) var phase = Phase.launching
    @Published var chooseScript = false
    @Published private(set) var scriptName = ""
    @Published private(set) var projectName = ""

    @Published var binaryPath: String { didSet { defaults.set(binaryPath, forKey: "binary") } }
    @Published var modelPath: String { didSet { defaults.set(modelPath, forKey: "model") } }
    @Published var locale: String { didSet { defaults.set(locale, forKey: "locale") } }
    @Published var countdownOn: Bool { didSet { defaults.set(countdownOn, forKey: "countdown") } }
    @Published private var lastScript: String { didSet { defaults.set(lastScript, forKey: "script") } }
    /// Whether the script's voice reads it, rather than the author: as the
    /// page's welcome was last told.
    @Published var voiceReads: Bool { didSet { defaults.set(voiceReads, forKey: "voiceReads") } }

    /// The page's screen, in windows of its own, which it opened.
    var screens: [NSWindow] = []
    /// The page, once shown: what the app's commands run script in.
    weak var webView: WKWebView?
    private let defaults = UserDefaults.standard
    private var server: ServerProcess?
    /// Where the server runs, and its origin once it listens.
    private var dir: URL?
    private var origin: URL?
    /// The page's address to load once the server listens.
    private var pending: [URLQueryItem] = []

    init() {
        binaryPath = defaults.string(forKey: "binary") ?? Self.defaultBinary() ?? ""
        modelPath = defaults.string(forKey: "model") ?? ""
        locale = defaults.string(forKey: "locale") ?? "en"
        countdownOn = defaults.object(forKey: "countdown") as? Bool ?? true
        lastScript = defaults.string(forKey: "script") ?? ""
        voiceReads = defaults.bool(forKey: "voiceReads")
    }

    var lastScriptURL: URL? { lastScript.isEmpty ? nil : URL(fileURLWithPath: lastScript) }

    /// Where `teleprompt` usually is. An app does not get the shell's PATH.
    static func defaultBinary() -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        return ["\(home)/.cargo/bin/teleprompt", "/opt/homebrew/bin/teleprompt", "/usr/local/bin/teleprompt"]
            .first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// The page's welcome: from the server running, or one started in the
    /// last script's project.
    func showHome() {
        if case .ready = phase, webView != nil { return run("showHome()") }
        serve(homeDir, [])
    }

    private var homeDir: URL {
        dir ?? lastScriptURL.map(projectDir(of:)) ?? FileManager.default.homeDirectoryForCurrentUser
    }

    /// Opens `script` in the page, from a server running in its project.
    func open(_ script: URL) {
        named(script)
        serve(projectDir(of: script), [URLQueryItem(name: "open", value: script.path)])
    }

    /// The page's setup, with `wanted` ticked, saying `why`.
    func offerSetup(_ wanted: [String] = [], why: String? = nil) {
        if case .ready = phase, webView != nil {
            let args = (try? JSONSerialization.data(withJSONObject: [wanted, why ?? NSNull()]))
                .map { String(decoding: $0, as: UTF8.self) } ?? "[[], null]"
            return run("openSetup(...\(args))")
        }
        var extra = [URLQueryItem(name: "setup", value: wanted.joined(separator: ","))]
        if let why { extra.append(URLQueryItem(name: "why", value: why)) }
        serve(homeDir, extra)
    }

    /// What the page did, as it tells the app: a script opened, which the
    /// app reopens next time; the welcome shown.
    func told(_ message: String) {
        guard let data = message.data(using: .utf8),
              let message = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }
        switch message["event"] as? String {
        case "opened":
            guard let path = message["path"] as? String else { return }
            voiceReads = message["voice"] as? Bool ?? false
            named(URL(fileURLWithPath: path))
        case "home":
            scriptName = ""
            projectName = ""
        default:
            break
        }
    }

    private func named(_ script: URL) {
        lastScript = script.path
        scriptName = script.lastPathComponent
        projectName = projectDir(of: script).lastPathComponent
    }

    /// The page with `extra` in its address, served from `dir`: by the
    /// server running there, or one started there in place of any other.
    private func serve(_ dir: URL, _ extra: [URLQueryItem]) {
        if server != nil, self.dir == dir {
            if let origin { phase = page(origin, extra).map { .ready(page: $0) } ?? phase }
            else { pending = extra }
            return
        }
        shutdown()
        guard FileManager.default.isExecutableFile(atPath: binaryPath) else {
            phase = .failed(["Set the teleprompt binary in Settings (⌘,). It must be built with --features listen to follow your voice."])
            return
        }
        let request = LaunchRequest(
            binary: URL(fileURLWithPath: binaryPath),
            dir: dir,
            model: modelPath.isEmpty ? nil : URL(fileURLWithPath: modelPath),
            locale: locale
        )
        let server = ServerProcess(request)
        self.server = server
        self.dir = dir
        pending = extra
        phase = .launching
        do {
            try server.start { [weak self] event in
                Task { @MainActor in self?.handle(event, from: server) }
            }
        } catch {
            phase = .failed(["Could not run \(binaryPath): \(error.localizedDescription)"])
        }
    }

    /// The page's address: in an app, whether a take counts down, who
    /// reads, and `extra`.
    private func page(_ origin: URL, _ extra: [URLQueryItem]) -> URL? {
        var page = URLComponents(url: origin, resolvingAgainstBaseURL: false)
        page?.path = "/"
        page?.queryItems = [URLQueryItem(name: "shell", value: "1")]
            + (countdownOn ? [] : [URLQueryItem(name: "countdown", value: "0")])
            + [URLQueryItem(name: "narrator", value: voiceReads ? "voice" : "you")]
            + extra
        return page?.url
    }

    private func run(_ script: String) {
        webView?.evaluateJavaScript(script)
    }

    private func handle(_ event: LaunchEvent, from server: ServerProcess) {
        guard server === self.server else { return }
        switch event {
        case let .listening(origin):
            self.origin = origin
            let extra = pending
            pending = []
            phase = page(origin, extra).map { .ready(page: $0) } ?? .failed(["The server's address is not one: \(origin)"])
        case let .ended(reasons):
            self.server = nil
            phase = .failed(reasons)
        }
    }

    /// The page could not be shown, and why.
    func pageFailed(_ why: String) {
        shutdown()
        phase = .failed(["Could not show the prompter: \(why)"])
    }

    /// The script in the author's own editor, for what the page does not
    /// edit: lines added, split or moved, and the shots' blocks.
    func openInEditor() {
        if let script = lastScriptURL { NSWorkspace.shared.open(script) }
    }

    /// Stops the server, and closes the screens its page opened.
    func shutdown() {
        server?.stop()
        server = nil
        dir = nil
        origin = nil
        for screen in screens { screen.close() }
        screens = []
    }
}
#endif
