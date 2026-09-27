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
    /// The beat of the count before a take, while it counts.
    @Published private(set) var counting: Int?
    @Published var chooseScript = false
    @Published var mirrored = false
    @Published var showsScreen = true
    @Published var textSize: CGFloat = 48
    /// The microphone's loudness, 0 to 1, while a take listens.
    @Published private(set) var level: Double = 0
    /// How far the clip on screen has played, and its time as text.
    @Published private(set) var clipFraction: Double = 0
    @Published private(set) var clipTime = ""
    @Published private(set) var toast: String?
    /// The line whose rewording is being reviewed.
    @Published var reviewing: Int?
    /// Rewordings a toast has told of, as line id and what was said.
    private var saidOffered: Set<[String]> = []
    @Published private(set) var scriptName = ""
    @Published private(set) var projectName = ""
    /// The take's running time: what ran before a pause, and since when.
    @Published private(set) var takeTime: TimeInterval = 0
    @Published private(set) var takeSince: Date?

    @Published var binaryPath: String { didSet { defaults.set(binaryPath, forKey: "binary") } }
    @Published var modelPath: String { didSet { defaults.set(modelPath, forKey: "model") } }
    @Published var locale: String { didSet { defaults.set(locale, forKey: "locale") } }
    @Published var countdownOn: Bool { didSet { defaults.set(countdownOn, forKey: "countdown") } }
    @Published private var lastScript: String { didSet { defaults.set(lastScript, forKey: "script") } }

    let player = AVPlayer()
    /// Where the microphone's audio goes, from the audio thread.
    private nonisolated let outlet = Outlet()
    private let defaults = UserDefaults.standard
    private var server: ServerProcess?
    private var client: SessionClient?
    private var mic: MicCapture?
    private var clipObserver: NSObjectProtocol?
    private var timeObserver: Any?

    init() {
        binaryPath = defaults.string(forKey: "binary") ?? Self.defaultBinary() ?? ""
        modelPath = defaults.string(forKey: "model") ?? ""
        locale = defaults.string(forKey: "locale") ?? "en"
        countdownOn = defaults.object(forKey: "countdown") as? Bool ?? true
        lastScript = defaults.string(forKey: "script") ?? ""
        timeObserver = player.addPeriodicTimeObserver(
            forInterval: CMTime(value: 1, timescale: 10), queue: .main
        ) { [weak self] time in
            MainActor.assumeIsolated { self?.showProgress(time) }
        }
    }

    var isReady: Bool { phase == .ready }
    var isTaking: Bool { isReady && (state.listening || paused || counting != nil) }
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
        scriptName = script.lastPathComponent
        let dir = script.deletingLastPathComponent()
        projectName = (dir.lastPathComponent == "scripts" ? dir.deletingLastPathComponent() : dir).lastPathComponent
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
        switch message {
        case .stopped:
            show(toast: state.status.text)
            Task {
                await refreshScript()
                offerSaid()
            }
        case .keptSaid:
            show(toast: state.status.text)
            Task { await refreshScript() }
        default:
            break
        }
    }

    private func refreshScript() async {
        guard let script = try? await client?.script() else { return }
        state.script = script
    }

    /// Tells of the lines newly heard saying other words, once.
    private func offerSaid() {
        let fresh = state.script.lines.enumerated().filter { _, line in
            line.said.map { !saidOffered.contains([line.id, $0]) } ?? false
        }
        guard let first = fresh.first else { return }
        for (_, line) in fresh { saidOffered.insert([line.id, line.said ?? ""]) }
        show(toast: fresh.count == 1
            ? "Line \(first.offset + 1) was said in other words. Press W to review"
            : "\(fresh.count) lines were said in other words. Press W to review")
    }

    /// Shows the next line said in other words, to keep what was said or
    /// the script.
    func reviewSaid() {
        guard isReady, !isTaking else { return }
        guard let line = state.saidOtherwise else {
            return show(toast: "Every line reads as it was said")
        }
        reviewing = line
    }

    /// Rewords the line under review to what was said, as `teleprompt edit
    /// <script> said <line>` does.
    func keepSaid() {
        guard let line = reviewing, let client else { return }
        reviewing = nil
        client.send(.keepSaid(line: state.script.lines[line].id))
    }

    /// Leaves the line under review as written.
    func keepScript() {
        guard let line = reviewing else { return }
        reviewing = nil
        state.keepScript(line: line)
    }

    /// Starts a take at `line`, after a count of three if that is on,
    /// opening the microphone the first time.
    func take(from line: Int) {
        guard isReady, client != nil, counting == nil else { return }
        Task {
            guard let mic = await openMic() else { return }
            mic.sending = false
            _ = mic.flush()
            player.pause()
            paused = false
            state.listening = false
            state.at = Position(line: line, word: 0)
            if countdownOn {
                state.status = .init("Recording from line \(line + 1) in…")
                for beat in [3, 2, 1] {
                    counting = beat
                    try? await Task.sleep(nanoseconds: 650_000_000)
                    if counting == nil { return }
                }
                counting = nil
            }
            begin(line, mic)
        }
    }

    private func begin(_ line: Int, _ mic: MicCapture) {
        guard let client else { return }
        state.startTake(from: line)
        takeTime = 0
        takeSince = .now
        client.send(.start(from: line, rate: mic.rate))
        mic.sending = true
    }

    /// Ends the take, or the count before it; the server keeps the lines
    /// read in full.
    func keep() {
        if counting != nil {
            counting = nil
            state.status = .init("Press ⌘⇧Space to record from here, or click a line")
            return
        }
        guard let client, let mic, state.listening || paused else { return }
        mic.sending = false
        client.sendAudio(mic.flush())
        client.send(.stop)
        state.listening = false
        paused = false
        stopClock()
        level = 0
        state.status = .init("Keeping the take…")
    }

    /// The record key (⌘⇧Space) and the Record button: a take from the line
    /// you are on, or keep the one under way.
    func recordOrKeep() {
        if isTaking { return keep() }
        let at = state.at.line
        take(from: at < state.script.lines.count ? at : 0)
    }

    func togglePause() {
        guard let mic, state.listening || paused else { return }
        paused.toggle()
        mic.sending = !paused
        state.listening = !paused
        state.status = .init(paused ? "Paused" : "Recording")
        if paused { stopClock() } else { takeSince = .now }
    }

    private func stopClock() {
        if let since = takeSince { takeTime += Date.now.timeIntervalSince(since) }
        takeSince = nil
    }

    /// The take's running time at `date`, as timecode.
    func timecode(at date: Date) -> String {
        let seconds = takeTime + (takeSince.map { date.timeIntervalSince($0) } ?? 0)
        let tenths = Int(seconds * 10)
        return String(format: "%02d:%02d.%d", tenths / 600, tenths / 10 % 60, tenths % 10)
    }

    private func openMic() async -> MicCapture? {
        if let mic { return mic }
        guard await AVCaptureDevice.requestAccess(for: .audio) else {
            state.status = .init("Teleprompt may not use the microphone: allow it in System Settings › Privacy & Security.", isError: true)
            return nil
        }
        do {
            let mic = try MicCapture(
                sink: { [outlet] samples in outlet.client?.sendAudio(samples) },
                level: { [weak self] rms in
                    Task { @MainActor in
                        guard let self, self.state.listening else { return }
                        self.level = min(1, Double(rms).squareRoot())
                    }
                }
            )
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
        clipFraction = 0
        clipTime = ""
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
        player.isMuted = true
        player.play()
    }

    private func showProgress(_ time: CMTime) {
        guard let duration = player.currentItem?.duration.seconds, duration.isFinite, duration > 0 else { return }
        clipFraction = min(1, time.seconds / duration)
        clipTime = String(format: "%.1f s / %.1f s", time.seconds, duration)
    }

    func clipEnded() {
        state.clipEnded()
        play(state.playing)
    }

    /// The next captured shot the reader has yet to reach, and its line.
    var nextShot: (name: String, line: Int)? {
        state.script.shots
            .first { $0.clip != nil && !state.started.contains($0.shot) && $0.at >= state.at }
            .map { (displayName($0.shot), $0.at.line + 1) }
    }

    private func show(toast text: String) {
        toast = text
        Task {
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            if toast == text { toast = nil }
        }
    }

    private func closeSession() {
        mic?.sending = false
        counting = nil
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

/// A shot's name as a person reads it: its block, without the index when
/// there is only one.
func displayName(_ shot: String) -> String {
    shot.hasSuffix("#0") ? String(shot.dropLast(2)) : shot
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
