#if canImport(SwiftUI)
import AVFoundation
import SwiftUI
import TelepromptKit

/// The app's state: the server it launched, the session with it, the
/// microphone, and what the prompter shows.
@MainActor
final class AppModel: ObservableObject {
    enum Phase: Equatable {
        /// Waiting for a script, or for settings to be filled in.
        case idle
        case launching(script: URL)
        case ready
        /// The server stopped or never started, and why.
        case failed([String])
    }

    @Published private(set) var phase = Phase.idle
    @Published var state = PrompterState()
    @Published private(set) var paused = false
    @Published var chooseScript = false
    @Published var mirrored = false
    @Published var showsScreen = true
    @Published var textSize: CGFloat = 44

    @Published var binaryPath: String { didSet { defaults.set(binaryPath, forKey: "binary") } }
    @Published var modelPath: String { didSet { defaults.set(modelPath, forKey: "model") } }
    @Published var locale: String { didSet { defaults.set(locale, forKey: "locale") } }
    @Published private var lastScript: String { didSet { defaults.set(lastScript, forKey: "script") } }

    let player = AVPlayer()
    /// Where the microphone's audio goes, from the audio thread.
    private nonisolated let outlet = Outlet()
    private let defaults = UserDefaults.standard
    private var server: ServerProcess?
    private var client: SessionClient?
    private var mic: MicCapture?
    private var clipObserver: NSObjectProtocol?

    init() {
        binaryPath = defaults.string(forKey: "binary") ?? Self.defaultBinary() ?? ""
        modelPath = defaults.string(forKey: "model") ?? ""
        locale = defaults.string(forKey: "locale") ?? "en"
        lastScript = defaults.string(forKey: "script") ?? ""
    }

    var isReady: Bool { phase == .ready }
    var isTaking: Bool { isReady && (state.listening || paused) }
    var lastScriptURL: URL? { lastScript.isEmpty ? nil : URL(fileURLWithPath: lastScript) }

    /// Where `teleprompt` usually is. An app does not get the shell's PATH.
    static func defaultBinary() -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        return ["\(home)/.cargo/bin/teleprompt", "/opt/homebrew/bin/teleprompt", "/usr/local/bin/teleprompt"]
            .first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// Launches `teleprompt prompt` for `script`, replacing any server
    /// already running.
    func open(_ script: URL) {
        shutdown()
        lastScript = script.path
        guard FileManager.default.isExecutableFile(atPath: binaryPath) else {
            phase = .failed(["Set the teleprompt binary in Settings (⌘,). It must be built with --features listen."])
            return
        }
        guard !modelPath.isEmpty else {
            phase = .failed(["Set the speech model directory in Settings (⌘,)."])
            return
        }
        let request = LaunchRequest(
            binary: URL(fileURLWithPath: binaryPath),
            script: script,
            model: URL(fileURLWithPath: modelPath),
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
            Task { await connect(to: origin) }
        case let .ended(reasons):
            closeSession()
            self.server = nil
            phase = .failed(reasons)
        }
    }

    private func connect(to origin: URL) async {
        let client = SessionClient(origin: origin)
        self.client = client
        outlet.client = client
        do {
            state = PrompterState()
            state.load(try await client.script())
        } catch {
            phase = .failed(["The server did not send its script: \(error.localizedDescription)"])
            return
        }
        client.open(
            onMessage: { [weak self] message in
                Task { @MainActor in self?.received(message) }
            },
            onClose: { [weak self] reason in
                Task { @MainActor in
                    guard let self, let reason, self.client === client else { return }
                    self.state.listening = false
                    self.state.status = .init(reason, isError: true)
                }
            }
        )
        phase = .ready
    }

    private func received(_ message: ServerMessage) {
        let before = state.playing
        state.apply(message)
        if state.playing != before { play(state.playing) }
        if case .stopped = message { Task { await refreshRecorded() } }
    }

    private func refreshRecorded() async {
        guard let script = try? await client?.script() else { return }
        state.script = script
    }

    /// Starts a take at `line`, opening the microphone the first time.
    func take(from line: Int) {
        guard isReady, let client else { return }
        Task {
            guard let mic = await openMic() else { return }
            mic.sending = false
            _ = mic.flush()
            player.pause()
            state.startTake(from: line)
            paused = false
            client.send(.start(from: line, rate: mic.rate))
            mic.sending = true
        }
    }

    /// Ends the take; the server keeps the lines read in full.
    func keep() {
        guard let client, let mic else { return }
        mic.sending = false
        client.sendAudio(mic.flush())
        client.send(.stop)
        state.listening = false
        paused = false
    }

    func togglePause() {
        guard let mic, isTaking else { return }
        paused.toggle()
        mic.sending = !paused
        state.listening = !paused
        state.status = .init(paused ? "paused" : "listening")
    }

    private func openMic() async -> MicCapture? {
        if let mic { return mic }
        guard await AVCaptureDevice.requestAccess(for: .audio) else {
            state.status = .init("Teleprompt may not use the microphone: allow it in System Settings › Privacy & Security.", isError: true)
            return nil
        }
        do {
            let mic = try MicCapture { [outlet] samples in
                outlet.client?.sendAudio(samples)
            }
            self.mic = mic
            return mic
        } catch {
            state.status = .init("The microphone did not start: \(error.localizedDescription)", isError: true)
            return nil
        }
    }

    /// Shows `shot`'s clip, or nothing.
    private func play(_ shot: String?) {
        if let clipObserver { NotificationCenter.default.removeObserver(clipObserver) }
        clipObserver = nil
        guard let shot, let path = state.script.shots.first(where: { $0.shot == shot })?.clip,
              let client
        else {
            player.replaceCurrentItem(with: nil)
            return
        }
        let item = AVPlayerItem(url: client.url(path))
        clipObserver = NotificationCenter.default.addObserver(
            forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.clipEnded() }
        }
        player.replaceCurrentItem(with: item)
        player.play()
    }

    func clipEnded() {
        state.clipEnded()
        play(state.playing)
    }

    private func closeSession() {
        mic?.sending = false
        client?.close()
        client = nil
        outlet.client = nil
        player.replaceCurrentItem(with: nil)
    }

    func shutdown() {
        closeSession()
        server?.stop()
        server = nil
        phase = .idle
    }
}

/// The session the microphone sends to, readable from any thread.
private final class Outlet: @unchecked Sendable {
    private let lock = NSLock()
    private var _client: SessionClient?

    var client: SessionClient? {
        get { lock.withLock { _client } }
        set { lock.withLock { _client = newValue } }
    }
}
#endif
