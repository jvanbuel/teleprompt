import Foundation

/// A script read by its voice: where the reading is at a point in a line's
/// audio, which shots that reaches, and how far the voice has got making
/// the lines. The app plays the audio; this says what it means.
public enum Voice {
    /// The word being said `ms` into a line whose words start at `starts`:
    /// the last one begun.
    public static func word(at ms: Int, starts: [Int]) -> Int {
        max(0, starts.prefix { $0 <= ms }.count - 1)
    }

    /// The shots not yet `started` whose cue lies between `from`, where the
    /// reading began, and `at`, where it is now: as a reader's voice would
    /// start them.
    public static func due(_ script: Script, from: Position, to at: Position, started: Set<String>) -> [String] {
        script.shots
            .filter { $0.at >= from && $0.at <= at && !started.contains($0.shot) }
            .map(\.shot)
    }

    /// How a line's audio stands, for its mark in the gutter.
    public enum Mark: Equatable, Sendable {
        /// Read from its take.
        case take
        /// Made by the voice.
        case voiced
        /// The voice has yet to make it.
        case unvoiced
    }

    /// Nil from a server that says nothing of voices.
    public static func mark(_ line: Script.Line) -> Mark? {
        guard let audio = line.audio else { return nil }
        switch audio.source {
        case .take: return .take
        case .voice: return audio.ready ? .voiced : .unvoiced
        }
    }

    /// How many lines have their audio, of how many that have any.
    public static func made(_ script: Script) -> (made: Int, of: Int) {
        let audio = script.lines.compactMap(\.audio)
        return (audio.filter(\.ready).count, audio.count)
    }

    /// The lines whose audio the voice has yet to make, in order.
    public static func unmade(_ script: Script) -> [Int] {
        script.lines.indices.filter { index in
            guard let audio = script.lines[index].audio else { return false }
            return !audio.ready
        }
    }

    /// The colours a speaker's mark takes, as hex: none is the cue's amber,
    /// the tally's red, the recorded green or the heard blue.
    public static let speakerColours = ["#c58af9", "#4dd0c8", "#f28bd0", "#d7b98e"]

    /// Speaker `name`'s colour: chosen by the name's UTF-8 bytes, so it is
    /// the same in every app.
    public static func speakerColour(_ name: String) -> String {
        let sum = name.utf8.reduce(0) { $0 + Int($1) }
        return speakerColours[sum % speakerColours.count]
    }

    /// The letter a speaker's mark shows.
    public static func initial(_ name: String) -> String {
        name.first.map { String($0).uppercased() } ?? ""
    }

    /// A length as minutes and seconds: "1:23".
    public static func clock(_ ms: Int) -> String {
        let seconds = ms / 1000
        return String(format: "%d:%02d", seconds / 60, seconds % 60)
    }
}
