import Foundation

/// A run of words in a line against what its take said, as the server's
/// script sends it: `{"kind": "same" | "gone" | "new", "words": …}`.
public enum SaidChange: Codable, Equatable, Sendable {
    case same(String)
    /// In the line, not said.
    case gone(String)
    /// Said, not in the line.
    case new(String)

    private enum CodingKeys: String, CodingKey { case kind, words }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let words = try c.decode(String.self, forKey: .words)
        switch try c.decode(String.self, forKey: .kind) {
        case "gone": self = .gone(words)
        case "new": self = .new(words)
        default: self = .same(words)
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        let (kind, words) = switch self {
        case let .same(w): ("same", w)
        case let .gone(w): ("gone", w)
        case let .new(w): ("new", w)
        }
        try c.encode(kind, forKey: .kind)
        try c.encode(words, forKey: .words)
    }
}
