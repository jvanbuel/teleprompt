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
    /// Whether the script's voice reads it, rather than the author.
    @Published var voiceReads: Bool { didSet { defaults.set(voiceReads, forKey: "voiceReads") } }
    /// Setup, open: what to tick, why, and what to carry on with.
    @Published var setup: SetupRequest?

    /// The voice reading the script aloud, while it does.
    @Published private(set) var reading: Reading?
    /// The line whose panel is open, read by a voice.
    @Published var panelLine: Int?
    /// The line whose words are being edited on the glass, and the words.
    @Published private(set) var rewording: Int?
    @Published var rewordText = ""
    /// What undoes each edit written, latest last; and the edit sent, until
    /// the server answers it.
    private var edits: [ClientMessage] = []
    private var pendingEdit: (back: ClientMessage?, said: String)?
    private var voicePlayer: AVAudioPlayer?
    private var readingTimer: Timer?
    /// Bumped whenever a reading ends, so audio fetched for it is dropped.
    private var readingGeneration = 0
    private var voicing = false

    struct Reading: Equatable {
        /// The line being read.
        var line: Int
        /// Only this line, as Listen reads it.
        var only: Bool
        /// Where it began: shots cued before it do not play.
        var from: Position
    }

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
        voiceReads = defaults.bool(forKey: "voiceReads")
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
    /// already running.
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
        if state.script.voiced {
            showVoiceStatus()
            voiceUnmade()
        }
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
        case .edited:
            let done = pendingEdit
            pendingEdit = nil
            if let back = done?.back { edits.append(back) }
            show(toast: done?.said ?? "Edited")
            Task {
                await refreshScript()
                showVoiceStatus()
                voiceUnmade()
            }
        case .error:
            pendingEdit = nil
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
        if voiced { return read(from: line) }
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
        if voiced { return playOrStop() }
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
        halt()
        panelLine = nil
        rewording = nil
        edits = []
        pendingEdit = nil
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

// Read by a voice: Play reads from the line you are on, lighting each word
// as it is said and starting the shots as it reaches them, as a reader's
// voice would. And changing a line where it is read: its words (F2), and,
// read by a voice, how to say it; each through `teleprompt edit` on the
// server, which writes nothing that would not compile.
extension AppModel {
    var voiced: Bool { isReady && state.script.voiced }
    var isReading: Bool { reading != nil }
    var canUndoEdit: Bool { !edits.isEmpty && rewording == nil && !isTaking }

    /// Space and the Play button: read from the line you are on, or stop.
    func playOrStop() {
        guard voiced, rewording == nil else { return }
        if isReading { return stopReading() }
        let at = state.at.line
        read(from: at < state.script.lines.count ? at : 0)
    }

    /// Reads from `line` to the end of the script, or only `line`; with
    /// `fresh`, has the voice make the line anew first.
    func read(from line: Int, only: Bool = false, fresh: Bool = false) {
        guard voiced, client != nil, rewording == nil else { return }
        halt()
        panelLine = nil
        state.playing = nil
        state.queue = []
        state.started = []
        play(nil)
        let start = Position(line: line, word: 0)
        reading = Reading(line: line, only: only, from: start)
        takeTime = 0
        takeSince = .now
        reach(start)
        state.status = .init(only ? "Reading line \(line + 1)" : "Reading from line \(line + 1)")
        readLine(line, fresh: fresh)
    }

    /// Moves the reading to `position`, starting the shots it reaches.
    private func reach(_ position: Position) {
        guard let reading else { return }
        let due = Voice.due(state.script, from: reading.from, to: position, started: state.started)
        let before = state.playing
        state.apply(.reached(at: position, play: due))
        if state.playing != before { play(state.playing) }
    }

    private func readLine(_ line: Int, fresh: Bool) {
        guard state.script.lines.indices.contains(line), let audio = state.script.lines[line].audio,
              let client
        else { return stopReading("Line \(line + 1) has no voice to read it") }
        let generation = readingGeneration
        Task {
            do {
                let wav = try await client.data(audio.url + (fresh ? "?fresh=1" : ""))
                // A line not made before now knows its words.
                if !audio.ready || fresh { await refreshScript() }
                guard readingGeneration == generation, reading?.line == line else { return }
                let player = try AVAudioPlayer(data: wav)
                voicePlayer = player
                player.play()
                readingTimer = Timer.scheduledTimer(withTimeInterval: 0.03, repeats: true) { [weak self] _ in
                    MainActor.assumeIsolated { self?.voiceTick() }
                }
            } catch {
                guard readingGeneration == generation else { return }
                stopReading("The voice failed: \(error.localizedDescription)", isError: true)
            }
        }
    }

    /// Lights the word the voice is saying, or moves on once it is done.
    private func voiceTick() {
        guard let reading, let player = voicePlayer else { return }
        if !player.isPlaying { return lineRead() }
        guard state.script.lines.indices.contains(reading.line) else { return }
        let starts = state.script.lines[reading.line].audio?.words ?? []
        let word = Voice.word(at: Int(player.currentTime * 1000), starts: starts)
        let position = Position(line: reading.line, word: word)
        if position != state.at { reach(position) }
    }

    /// The voice finished a line: on to the next, or done.
    private func lineRead() {
        guard let current = reading else { return }
        dropAudio()
        let next = current.line + 1
        let count = state.script.lines.count
        if current.only || next >= count {
            halt()
            state.at = Position(line: min(next, max(0, count - 1)), word: 0)
            state.status = .init(current.only ? "Read line \(current.line + 1)" : "Read to the end")
            return
        }
        reading?.line = next
        reach(Position(line: next, word: 0))
        readLine(next, fresh: false)
    }

    private func dropAudio() {
        readingTimer?.invalidate()
        readingTimer = nil
        voicePlayer?.stop()
        voicePlayer = nil
    }

    /// Ends any reading, quietly.
    private func halt() {
        dropAudio()
        if reading != nil { stopClock() }
        reading = nil
        readingGeneration += 1
    }

    /// Stops the voice where it is, saying `why` or that it stopped.
    func stopReading(_ why: String? = nil, isError: Bool = false) {
        guard isReading else { return }
        halt()
        state.status = .init(why ?? "Stopped. Space reads on from here", isError: isError)
    }

    /// Has the voice make the lines it has yet to, one after another, in
    /// the background; once at a time.
    private func voiceUnmade() {
        guard voiced, !voicing, let client else { return }
        voicing = true
        Task {
            var failed: Set<Int> = []
            while self.client === client {
                guard let line = Voice.unmade(state.script).first(where: { !failed.contains($0) }),
                      let audio = state.script.lines[line].audio
                else { break }
                do {
                    _ = try await client.data(audio.url)
                } catch {
                    failed.insert(line)
                    state.status = .init(
                        "The voice could not read line \(line + 1): \(error.localizedDescription)", isError: true
                    )
                    continue
                }
                await refreshScript()
                showVoiceStatus()
            }
            voicing = false
        }
    }

    /// Who reads, how long the video runs, and how far the voice has got;
    /// not while it reads, or a line is being reworded.
    private func showVoiceStatus() {
        guard voiced, reading == nil, rewording == nil else { return }
        let name = state.script.voice?.name ?? ""
        let (made, of) = Voice.made(state.script)
        if made < of {
            state.status = PrompterState.Status("\(name) is reading the lines: \(made) of \(of)")
        } else {
            let length = Voice.clock(state.script.lengthMs ?? 0)
            state.status = PrompterState.Status("\(name) · \(length) long. Space reads from here; click a line to direct it")
        }
    }

    /// A line clicked: read by a voice, it reads on from there while the
    /// voice reads, and otherwise opens the line's panel; read by its
    /// author, a take starts there.
    func lineTapped(_ line: Int) {
        guard rewording == nil else { return }
        guard voiced else { return take(from: line) }
        if isReading { return read(from: line) }
        state.at = Position(line: line, word: 0)
        panelLine = line
    }

    /// Tells the voice how to say line `line`, or, empty, stops telling it.
    func instruct(_ line: Int, _ text: String) {
        panelLine = nil
        guard state.script.lines.indices.contains(line) else { return }
        let l = state.script.lines[line]
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard text != (l.instruct ?? "") else { return }
        sendEdit(
            .instruct(line: l.id, text: text.isEmpty ? nil : text),
            back: .instruct(line: l.id, text: l.instruct),
            said: text.isEmpty ? "Line \(line + 1) said as the voice would" : "Line \(line + 1) said \(text)"
        )
    }

    /// F2: reword the line you are on.
    func rewordCurrent() {
        beginReword(min(state.at.line, max(0, state.script.lines.count - 1)))
    }

    /// Line `line`'s words become editable where they stand: Return keeps
    /// them, Escape leaves the line as it was. Not during a take, nor on
    /// mirrored glass, which reads backwards.
    func beginReword(_ line: Int) {
        guard isReady, !isTaking, rewording == nil, state.script.lines.indices.contains(line) else { return }
        if mirrored {
            state.status = .init("Mirrored text cannot be edited: M turns mirroring off")
            return
        }
        halt()
        panelLine = nil
        state.at = Position(line: line, word: 0)
        rewordText = state.script.lines[line].text
        rewording = line
        state.status = .init("Rewording line \(line + 1): Return keeps it, Esc leaves it")
    }

    /// Ends the rewording, writing the new words if `keep`.
    func endReword(keep: Bool) {
        guard let line = rewording else { return }
        rewording = nil
        guard state.script.lines.indices.contains(line) else { return }
        let l = state.script.lines[line]
        // A paragraph is one line: what was typed on several is one.
        let text = rewordText.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        if keep, !text.isEmpty, text != l.text {
            sendEdit(.reword(line: l.id, text: text), back: .reword(line: l.id, text: l.text), said: "Reworded line \(line + 1)")
        }
        if voiced {
            showVoiceStatus()
        } else {
            state.status = .init("Press ⌘⇧Space to record from here, or click a line")
        }
    }

    /// Puts back what the last edit changed.
    func undoEdit() {
        guard canUndoEdit, let back = edits.popLast() else { return }
        sendEdit(back, back: nil, said: "Undone")
    }

    private func sendEdit(_ edit: ClientMessage, back: ClientMessage?, said: String) {
        guard let client else { return }
        pendingEdit = (back, said)
        client.send(edit)
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
