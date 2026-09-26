import XCTest
@testable import TelepromptKit

final class PrompterStateTests: XCTestCase {
    private func loaded() throws -> PrompterState {
        var state = PrompterState()
        state.load(try JSONDecoder().decode(Script.self, from: example("script.json")))
        return state
    }

    func testWordsBeforeTheReaderAreSaid() throws {
        var state = try loaded()
        state.apply(.reached(at: Position(line: 0, word: 3), play: []))
        XCTAssertEqual(state.word(line: 0, word: 2), .said)
        XCTAssertEqual(state.word(line: 0, word: 3), .next)
        XCTAssertEqual(state.word(line: 1, word: 0), .ahead)
    }

    /// Shots play in the order given; reading on cuts off the one playing.
    func testShotsPlayInOrderAndReadingOnCutsThemOff() throws {
        var state = try loaded()
        state.startTake(from: 0)
        state.apply(.reached(at: Position(line: 0, word: 3), play: ["intro#0", "welcome-a#0"]))
        XCTAssertEqual(state.playing, "intro#0")
        state.clipEnded()
        XCTAssertEqual(state.playing, "welcome-a#0")
        state.apply(.reached(at: Position(line: 1, word: 0), play: ["welcome-b#0"]))
        XCTAssertEqual(state.playing, "welcome-b#0")
        state.clipEnded()
        XCTAssertNil(state.playing)
        XCTAssertEqual(state.started, ["intro#0", "welcome-a#0", "welcome-b#0"])
    }

    func testANewTakeForgetsWhatPlayed() throws {
        var state = try loaded()
        state.startTake(from: 0)
        state.apply(.reached(at: Position(line: 0, word: 3), play: ["intro#0"]))
        state.startTake(from: 1)
        XCTAssertEqual(state.at, Position(line: 1, word: 0))
        XCTAssertNil(state.playing)
        XCTAssertTrue(state.started.isEmpty)
        XCTAssertTrue(state.listening)
    }

    func testStoppingSaysWhatWasKept() throws {
        var state = try loaded()
        state.startTake(from: 0)
        state.apply(.stopped(saved: ["welcome"]))
        XCTAssertFalse(state.listening)
        XCTAssertEqual(state.status, .init("kept 1 line(s): welcome"))
        state.apply(.stopped(saved: []))
        XCTAssertEqual(state.status, .init("nothing read in full to keep"))
    }

    func testAnErrorIsShown() throws {
        var state = try loaded()
        state.apply(.error("disk full"))
        XCTAssertEqual(state.status, .init("disk full", isError: true))
    }

    func testTheEndOfTheScriptIsSaid() throws {
        var state = try loaded()
        state.apply(.reached(at: Position(line: 2, word: 0), play: []))
        XCTAssertEqual(state.status, .init("end of script"))
    }
}
