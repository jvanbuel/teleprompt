import XCTest
@testable import TelepromptKit

final class APITests: XCTestCase {
    func testTheScriptExampleDecodes() throws {
        let script = try JSONDecoder().decode(Script.self, from: example("script.json"))
        XCTAssertEqual(script.lines.map(\.id), ["welcome", "deploy"])
        XCTAssertEqual(script.lines[0].words.count, 8)
        XCTAssertEqual(script.shots.map(\.at), [
            Position(line: 0, word: 0), Position(line: 0, word: 3), Position(line: 1, word: 0),
        ])
        XCTAssertNil(script.shots[0].clip)
        XCTAssertEqual(script.shots[1].clip?.hasPrefix("/api/v1/clips/"), true)
    }

    func testTheServerMessageExamplesDecode() throws {
        XCTAssertEqual(
            try ServerMessage(json: example("reached.json")),
            .reached(at: Position(line: 1, word: 0), play: ["welcome-b#0"])
        )
        XCTAssertEqual(try ServerMessage(json: example("stopped.json")), .stopped(saved: ["welcome"]))
        XCTAssertEqual(
            try ServerMessage(json: example("error.json")),
            .error(#"not a message this server knows: {"type":"rewind"}"#)
        )
    }

    /// A later v1 server may send more; the client ignores what it does
    /// not know rather than failing.
    func testWhatThisClientDoesNotKnowIsIgnored() throws {
        let newer = #"{"type":"reached","line":2,"word":1,"play":[],"confidence":0.9}"#
        XCTAssertEqual(try ServerMessage(json: Data(newer.utf8)), .reached(at: Position(line: 2, word: 1), play: []))
        let unknown = #"{"type":"level","rms":0.2}"#
        XCTAssertEqual(try ServerMessage(json: Data(unknown.utf8)), .unknown(type: "level"))
    }

    func testCommandsAreTheExamples() throws {
        XCTAssertEqual(try jsonObject(ClientMessage.start(from: 1, rate: 48000).json()), try jsonObject(example("start.json")))
        XCTAssertEqual(try jsonObject(ClientMessage.stop.json()), try jsonObject(example("stop.json")))
    }

    func testSamplesAreLittleEndianFloats() {
        let data = encodeSamples([1.0, -0.5])
        XCTAssertEqual(Array(data), [0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x00, 0xBF])
    }
}
