import Foundation

/// The prompter API, version 1 (`docs/design.md#prompter-api-version-1`).
/// The types decode what the server sends and ignore what they do not know,
/// as the API asks of its clients.
public enum API {
    public static let version = "/api/v1"
}

/// The next word the reader will say.
public struct Position: Codable, Equatable, Comparable, Sendable {
    public var line: Int
    public var word: Int

    public init(line: Int, word: Int) {
        self.line = line
        self.word = word
    }

    public static func < (a: Position, b: Position) -> Bool {
        (a.line, a.word) < (b.line, b.word)
    }
}

/// `GET /api/v1/script`.
public struct Script: Codable, Equatable, Sendable {
    /// The script's file name; servers before it was added send none.
    public var name: String?
    public var lines: [Line]
    public var shots: [Shot]
    /// Who reads the script, for a project's script; servers before voices
    /// were served send none.
    public var voice: Voice?
    /// The video's length as the timeline has it now.
    public var lengthMs: Int?

    enum CodingKeys: String, CodingKey {
        case name, lines, shots, voice
        case lengthMs = "length_ms"
    }

    public init(name: String? = nil, lines: [Line], shots: [Shot], voice: Voice? = nil, lengthMs: Int? = nil) {
        self.name = name
        self.lines = lines
        self.shots = shots
        self.voice = voice
        self.lengthMs = lengthMs
    }

    /// Whether the script's voice reads it, rather than a reader followed
    /// by ear.
    public var voiced: Bool { voice.map { !$0.listens } ?? false }

    public struct Voice: Codable, Equatable, Sendable {
        /// The voice, as its backend names it: "kokoro · af_heart".
        public var name: String
        /// Whether the server follows a reader by ear.
        public var listens: Bool

        public init(name: String, listens: Bool) {
            self.name = name
            self.listens = listens
        }
    }

    /// A line's audio, as its voice or its take says it.
    public struct Audio: Codable, Equatable, Sendable {
        public enum Source: String, Codable, Sendable { case take, voice }
        public var source: Source
        /// The WAV, under the server's origin; made when fetched.
        public var url: String
        /// Whether it is made yet.
        public var ready: Bool
        public var durationMs: Int?
        /// When each word starts, in milliseconds, once it is made.
        public var words: [Int]?

        enum CodingKeys: String, CodingKey {
            case source, url, ready, words
            case durationMs = "duration_ms"
        }

        public init(source: Source, url: String, ready: Bool, durationMs: Int? = nil, words: [Int]? = nil) {
            self.source = source
            self.url = url
            self.ready = ready
            self.durationMs = durationMs
            self.words = words
        }
    }

    public struct Line: Codable, Equatable, Sendable {
        public var id: String
        public var text: String
        /// Whether the line has a take read from it as it now reads.
        public var recorded: Bool
        /// The line as its take was heard to say it, where that is other
        /// words; servers before it was added send none.
        public var said: String?
        /// The line against `said`, word by word.
        public var saidDiff: [SaidChange]?
        /// How the line sounds read by the script's voice.
        public var audio: Audio?
        /// How the voice is told to say it.
        public var instruct: String?

        enum CodingKeys: String, CodingKey {
            case id, text, recorded, said, audio, instruct
            case saidDiff = "said_diff"
        }

        public init(
            id: String, text: String, recorded: Bool, said: String? = nil, saidDiff: [SaidChange]? = nil,
            audio: Audio? = nil, instruct: String? = nil
        ) {
            self.id = id
            self.text = text
            self.recorded = recorded
            self.said = said
            self.saidDiff = saidDiff
            self.audio = audio
            self.instruct = instruct
        }

        /// The words as the server counts them: split at whitespace.
        public var words: [String] {
            text.split(whereSeparator: { $0.isWhitespace }).map(String.init)
        }
    }

    public struct Shot: Codable, Equatable, Sendable {
        public var shot: String
        public var at: Position
        /// The clip's path under the server's origin, or nil if the shot was
        /// never captured.
        public var clip: String?

        public init(shot: String, at: Position, clip: String?) {
            self.shot = shot
            self.at = at
            self.clip = clip
        }
    }
}

/// A message from the server on the session socket.
public enum ServerMessage: Equatable, Sendable {
    /// Where the reader is, and the shots that just reached their cue.
    case reached(at: Position, play: [String])
    /// The ids of the lines kept from the take.
    case stopped(saved: [String])
    /// The line now reads as its take was heard to say it.
    case keptSaid(line: String)
    /// A `reword` or `instruct` was written into the script.
    case edited(line: String)
    case error(String)
    /// A message this client does not know; a later v1 server may send it.
    case unknown(type: String)

    public init(json: Data) throws {
        let header = try JSONDecoder().decode(Header.self, from: json)
        switch header.type {
        case "reached":
            let m = try JSONDecoder().decode(Reached.self, from: json)
            self = .reached(at: Position(line: m.line, word: m.word), play: m.play)
        case "stopped":
            self = .stopped(saved: try JSONDecoder().decode(Stopped.self, from: json).saved)
        case "kept_said":
            self = .keptSaid(line: try JSONDecoder().decode(Line.self, from: json).line)
        case "edited":
            self = .edited(line: try JSONDecoder().decode(Line.self, from: json).line)
        case "error":
            self = .error(try JSONDecoder().decode(Failure.self, from: json).message)
        default:
            self = .unknown(type: header.type)
        }
    }

    private struct Header: Decodable { var type: String }
    private struct Reached: Decodable { var line: Int; var word: Int; var play: [String] }
    private struct Stopped: Decodable { var saved: [String] }
    private struct Line: Decodable { var line: String }
    private struct Failure: Decodable { var message: String }
}

/// A command to the server on the session socket. Audio goes as binary
/// messages; see ``encodeSamples(_:)``.
public enum ClientMessage: Equatable, Sendable {
    /// A new take at line `from`, with audio at `rate` Hz.
    case start(from: Int, rate: Int)
    /// Keep the lines read in full.
    case stop
    /// Reword a line to what its take was heard to say.
    case keepSaid(line: String)
    /// Say the line as `text`.
    case reword(line: String, text: String)
    /// Tell the voice how to say the line; nil stops telling it.
    case instruct(line: String, text: String?)

    public func json() -> Data {
        let object: [String: Any] = switch self {
        case let .start(from, rate): ["type": "start", "from": from, "rate": rate]
        case .stop: ["type": "stop"]
        case let .keepSaid(line): ["type": "keep_said", "line": line]
        case let .reword(line, text): ["type": "reword", "line": line, "text": text]
        case let .instruct(line, text): ["type": "instruct", "line": line, "text": (text as Any?) ?? NSNull()]
        }
        // Sorted keys: the same command is the same bytes.
        return try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    }
}

/// Mono samples as the socket takes them: little-endian 32-bit floats.
public func encodeSamples(_ samples: [Float]) -> Data {
    var data = Data(capacity: samples.count * 4)
    for sample in samples {
        withUnsafeBytes(of: sample.bitPattern.littleEndian) { data.append(contentsOf: $0) }
    }
    return data
}
