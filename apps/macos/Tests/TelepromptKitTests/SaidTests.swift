import XCTest
@testable import TelepromptKit

/// A line against what its take said, as the review shows it.
final class SaidTests: XCTestCase {
    func testAWordSwappedIsGoneAndNew() {
        XCTAssertEqual(
            saidDiff("Let me walk you around.", "Let me show you around."),
            [.same("Let me"), .gone("walk"), .new("show"), .same("you around.")]
        )
    }

    func testWordsLeftOutAreGoneAndTheEndMovesBack() {
        XCTAssertEqual(
            saidDiff("Let me show you around the office!", "Let me show you around!"),
            [.same("Let me show you"), .gone("around the office!"), .new("around!")]
        )
    }

    func testTheSameWordsAreOneRun() {
        XCTAssertEqual(saidDiff("Deploy it.", "Deploy it."), [.same("Deploy it.")])
    }
}

final class ReviewTests: XCTestCase {
    private func heard() -> PrompterState {
        var state = PrompterState()
        state.load(Script(lines: [
            .init(id: "welcome", text: "Let me walk you around.", recorded: true, said: "Let me show you around."),
            .init(id: "deploy", text: "Deploy it.", recorded: true),
            .init(id: "end", text: "Bye now.", recorded: true, said: "Bye."),
        ], shots: []))
        return state
    }

    func testTheFirstLineSaidOtherwiseIsReviewedFirst() {
        XCTAssertEqual(heard().saidOtherwise, 0)
    }

    /// Keeping the script is not asked again, until the take says
    /// something else.
    func testAKeptScriptIsNotAskedAgain() {
        var state = heard()
        state.keepScript(line: 0)
        XCTAssertEqual(state.saidOtherwise, 2)
        state.keepScript(line: 2)
        XCTAssertNil(state.saidOtherwise)
        state.script.lines[0].said = "Let me take you around."
        XCTAssertEqual(state.saidOtherwise, 0)
    }

    func testAKeptRewordingSaysSo() {
        var state = heard()
        state.apply(.keptSaid(line: "end"))
        XCTAssertEqual(state.status, .init("Line 3 now reads as you said it"))
    }
}
