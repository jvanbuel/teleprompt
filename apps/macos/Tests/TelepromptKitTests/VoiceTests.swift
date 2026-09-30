import XCTest
@testable import TelepromptKit

/// A script read by its voice, as `docs/api/v1/examples/voiced_script.json`
/// has it: the same cases as the Linux app's `tests/voice.rs`.
final class VoiceTests: XCTestCase {
    private func voiced() throws -> Script {
        try JSONDecoder().decode(Script.self, from: example("voiced_script.json"))
    }

    func testTheVoicedScriptExampleDecodes() throws {
        let script = try voiced()
        XCTAssertEqual(script.voice, Script.Voice(name: "kokoro · af_heart", listens: false))
        XCTAssertTrue(script.voiced)
        XCTAssertEqual(script.lengthMs, 6120)
        let welcome = script.lines[0]
        XCTAssertEqual(welcome.instruct, "warmly")
        XCTAssertEqual(welcome.audio?.source, .voice)
        XCTAssertEqual(welcome.audio?.url, "/api/v1/voice/welcome.wav")
        XCTAssertEqual(welcome.audio?.durationMs, 2600)
        XCTAssertEqual(welcome.audio?.words?.count, welcome.words.count)
        XCTAssertEqual(script.lines[1].audio?.ready, false)
        XCTAssertNil(script.lines[1].audio?.words)
        // A server that says nothing of voices reads as before.
        let plain = try JSONDecoder().decode(Script.self, from: example("script.json"))
        XCTAssertFalse(plain.voiced)
        XCTAssertNil(plain.lines[0].audio)
    }

    func testTheWordBeingSaidIsTheLastOneBegun() {
        let starts = [0, 520, 660, 1100]
        XCTAssertEqual(Voice.word(at: 0, starts: starts), 0)
        XCTAssertEqual(Voice.word(at: 519, starts: starts), 0)
        XCTAssertEqual(Voice.word(at: 520, starts: starts), 1)
        XCTAssertEqual(Voice.word(at: 5000, starts: starts), 3)
        XCTAssertEqual(Voice.word(at: 300, starts: []), 0)
    }

    func testShotsAreDueAsTheReadingReachesThemAndOnce() throws {
        let script = try voiced()
        let pos = { Position(line: $0, word: $1) }
        XCTAssertEqual(Voice.due(script, from: pos(0, 0), to: pos(0, 0), started: []), ["intro#0"])
        XCTAssertEqual(Voice.due(script, from: pos(0, 0), to: pos(0, 7), started: ["intro#0"]), [])
        XCTAssertEqual(Voice.due(script, from: pos(0, 0), to: pos(1, 0), started: ["intro#0"]), ["welcome-b#0"])
        XCTAssertEqual(Voice.due(script, from: pos(1, 0), to: pos(1, 2), started: []), ["welcome-b#0"])
    }

    func testEachLineIsMarkedByWhereItsAudioComesFrom() throws {
        var script = try voiced()
        XCTAssertEqual(Voice.mark(script.lines[0]), .voiced)
        XCTAssertEqual(Voice.mark(script.lines[1]), .unvoiced)
        script.lines[1].audio?.source = .take
        XCTAssertEqual(Voice.mark(script.lines[1]), .take)
        let plain = try JSONDecoder().decode(Script.self, from: example("script.json"))
        XCTAssertNil(Voice.mark(plain.lines[0]))
    }

    func testProgressCountsTheLinesTheVoiceHasMade() throws {
        let script = try voiced()
        XCTAssertEqual(Voice.made(script).made, 1)
        XCTAssertEqual(Voice.made(script).of, 2)
        XCTAssertEqual(Voice.unmade(script), [1])
    }

    func testALengthReadsAsMinutesAndSeconds() {
        XCTAssertEqual(Voice.clock(6120), "0:06")
        XCTAssertEqual(Voice.clock(83400), "1:23")
        XCTAssertEqual(Voice.clock(3_725_000), "62:05")
    }

    func testEditsAreTheExamples() throws {
        XCTAssertEqual(
            try jsonObject(ClientMessage.reword(line: "deploy", text: "Deploying takes one command.").json()),
            try jsonObject(example("reword.json"))
        )
        XCTAssertEqual(
            try jsonObject(ClientMessage.instruct(line: "welcome", text: "warmly").json()),
            try jsonObject(example("instruct.json"))
        )
        XCTAssertEqual(try ServerMessage(json: example("edited.json")), .edited(line: "deploy"))
    }
}
