#if canImport(SwiftUI)
import AppKit
import SwiftUI
import TelepromptKit

/// The app's state: the server it launched, and where its page is. The
/// prompter is that page (`PrompterPage`), here as in a browser.
@MainActor
final class AppModel: ObservableObject {
    enum Phase: Equatable {
        /// Waiting for a script, or for settings to be filled in.
        case idle
        case launching(script: URL)
        /// The server listens: its page, to show.
        case ready(page: URL)
        /// The server stopped or never started, and why.
        case failed([String])
    }

    @Published private(set) var phase = Phase.idle
    @Published var chooseScript = false
    @Published private(set) var scriptName = ""
    @Published private(set) var projectName = ""

    @Published var binaryPath: String { didSet { defaults.set(binaryPath, forKey: "binary") } }
    @Published var modelPath: String { didSet { defaults.set(modelPath, forKey: "model") } }
    @Published var locale: String { didSet { defaults.set(locale, forKey: "locale") } }
    @Published var countdownOn: Bool { didSet { defaults.set(countdownOn, forKey: "countdown") } }
    @Published private var lastScript: String { didSet { defaults.set(lastScript, forKey: "script") } }
    /// Whether the script's voice reads it, rather than the author.
    @Published var voiceReads: Bool { didSet { defaults.set(voiceReads, forKey: "voiceReads") } }
    /// Setup, open: what to tick, why, and what to carry on with.
    @Published var setup: SetupRequest?

    /// The page's screen, in windows of its own, which it opened.
    var screens: [NSWindow] = []
    private let defaults = UserDefaults.standard
    private var server: ServerProcess?

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

    /// Opens setup with `wanted` ticked, saying `why`, and `then` once
    /// something is installed. Setup is teleprompt's to do, so without the
    /// binary it says where to set that.
    func offerSetup(_ wanted: [String] = [], why: String? = nil, then: (@MainActor () -> Void)? = nil) {
        guard FileManager.default.isExecutableFile(atPath: binaryPath) else {
            phase = .failed(["Set the teleprompt binary in Settings (⌘,): it is what sets up the rest."])
            return
        }
        setup = SetupRequest(wanted: wanted, why: why, then: then)
    }

    /// Launches `teleprompt prompt` for `script`, replacing any server
    /// already running, and shows its page once it listens.
    func open(_ script: URL) {
        shutdown()
        lastScript = script.path
        scriptName = script.lastPathComponent
        let dir = script.deletingLastPathComponent()
        projectName = (dir.lastPathComponent == "scripts" ? dir.deletingLastPathComponent() : dir).lastPathComponent
        guard FileManager.default.isExecutableFile(atPath: binaryPath) else {
            phase = .failed([voiceReads
                ? "Set the teleprompt binary in Settings (⌘,)."
                : "Set the teleprompt binary in Settings (⌘,). It must be built with --features listen."])
            return
        }
        // A voice reads without a speech model; the author, with one: the
        // one chosen in Settings, or the one `teleprompt setup` installed.
        let speech = modelPath.isEmpty ? installedSpeechModel() : URL(fileURLWithPath: modelPath)
        guard voiceReads || speech != nil else {
            phase = .idle
            offerSetup(
                ["prompt"],
                why: "Following your voice needs the speech model. The script opens once it is installed. Or let a voice read it: choose \u{201C}A voice reads\u{201D} on the welcome page.",
                then: { [weak self] in self?.open(script) }
            )
            return
        }
        let request = LaunchRequest(
            binary: URL(fileURLWithPath: binaryPath),
            script: script,
            model: voiceReads ? nil : speech,
            locale: locale
        )
        let server = ServerProcess(request)
        self.server = server
        phase = .launching(script: script)
        do {
            try server.start { [weak self] event in
                Task { @MainActor in self?.handle(event, from: server) }
            }
        } catch {
            phase = .failed(["Could not run \(binaryPath): \(error.localizedDescription)"])
        }
    }

    private func handle(_ event: LaunchEvent, from server: ServerProcess) {
        guard server === self.server else { return }
        switch event {
        case let .listening(origin):
            // Settings the page takes in its address: it is in an app,
            // whose title bar names the script, and whether a take counts down.
            var page = URLComponents(url: origin, resolvingAgainstBaseURL: false)
            page?.path = "/"
            page?.queryItems = [URLQueryItem(name: "shell", value: "1")]
                + (countdownOn ? [] : [URLQueryItem(name: "countdown", value: "0")])
            phase = page?.url.map { .ready(page: $0) } ?? .failed(["The server's address is not one: \(origin)"])
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
        for screen in screens { screen.close() }
        screens = []
    }
}
#endif
