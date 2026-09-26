import Foundation

/// What the prompter shows, driven by the server's messages. No I/O: the
/// app applies messages and reads the state back.
public struct PrompterState: Equatable, Sendable {
    public var script = Script(lines: [], shots: [])
    /// The next word to be said.
    public var at = Position(line: 0, word: 0)
    /// Whether audio is being sent: a take is under way and not paused.
    public var listening = false
    /// The shot on screen, and the ones to play after it.
    public var playing: String?
    public var queue: [String] = []
    /// Shots that have started this take.
    public var started: Set<String> = []
    public var status = Status("choose a script")

    public struct Status: Equatable, Sendable {
        public var text: String
        public var isError: Bool

        public init(_ text: String, isError: Bool = false) {
            self.text = text
            self.isError = isError
        }
    }

    public init() {}

    public mutating func load(_ script: Script) {
        self.script = script
        at = Position(line: 0, word: 0)
        status = Status("click a line, or press return, to start a take")
    }

    /// A take is starting at line `from`: nothing has played in it yet.
    public mutating func startTake(from: Int) {
        at = Position(line: from, word: 0)
        playing = nil
        queue = []
        started = []
        listening = true
        status = Status("recording from line \(from + 1)")
    }

    public mutating func apply(_ message: ServerMessage) {
        switch message {
        case let .reached(at, play):
            self.at = at
            if !play.isEmpty {
                // Reading on cuts off the shot that is playing.
                playing = play.first
                queue = Array(play.dropFirst())
                started.formUnion(play)
            }
            if at.line >= script.lines.count, !script.lines.isEmpty {
                status = Status("end of script")
            }
        case let .stopped(saved):
            listening = false
            status = Status(saved.isEmpty
                ? "nothing read in full to keep"
                : "kept \(saved.count) line(s): \(saved.joined(separator: ", "))")
        case let .error(message):
            status = Status(message, isError: true)
        case .unknown:
            break
        }
    }

    /// The shot on screen ended; the next one queued plays.
    public mutating func clipEnded() {
        playing = queue.isEmpty ? nil : queue.removeFirst()
    }

    public enum Word: Equatable, Sendable { case said, next, ahead }

    public func word(line: Int, word: Int) -> Word {
        let here = Position(line: line, word: word)
        return here < at ? .said : here == at ? .next : .ahead
    }

    /// The shots that start before `word` of `line`, or at its end when
    /// `word` is the line's length.
    public func shots(at position: Position) -> [Script.Shot] {
        script.shots.filter { $0.at == position }
    }
}
