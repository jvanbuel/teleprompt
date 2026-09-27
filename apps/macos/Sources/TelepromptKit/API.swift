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

    public init(name: String? = nil, lines: [Line], shots: [Shot]) {
        self.name = name
        self.lines = lines
        self.shots = shots
    }

    public struct Line: Codable, Equatable, Sendable {
        public var id: String
        public var text: String
        /// Whether the line has a take read from it as it now reads.
        public var recorded: Bool
        /// The line as its take was heard to say it, where that is other
        /// words; servers before it was added send none.
        public var said: String?

        public init(id: String, text: String, recorded: Bool, said: String? = nil) {
            self.id = id
            self.text = text
            self.recorded = recorded
            self.said = said
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

    public func json() -> Data {
        let object: [String: Any] = switch self {
        case let .start(from, rate): ["type": "start", "from": from, "rate": rate]
        case .stop: ["type": "stop"]
        case let .keepSaid(line): ["type": "keep_said", "line": line]
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
