import Foundation
import XCTest
@testable import TelepromptKit

/// Setting teleprompt up from the app: what `teleprompt setup --uses` says
/// each use needs (as the Linux app is tested against too), and an
/// install's progress as its events say.
final class SetupTests: XCTestCase {
    private func uses() throws -> [SetupUse] {
        try parseUses(Data(contentsOf: repository.appendingPathComponent("apps/fixtures/setup-uses.json")))
    }

    func testEachUseSaysWhatItStillNeeds() throws {
        let uses = try uses()
        XCTAssertEqual(uses.map(\.name), ["render", "terminal", "conversations"])
        XCTAssertEqual(uses.map(\.state), [
            "Installed",
            "Needs vhs, ttyd",
            "Needs teleprompt built with speech models",
        ])
        XCTAssertEqual(uses[2].missing.count, 3)
        XCTAssertTrue(uses[1].tools.contains { $0.name == "ttyd" && $0.password })
    }

    func testAModelSeveralUsesNeedIsDownloadedOnce() throws {
        let conversations = try uses()[2]
        XCTAssertEqual(downloadMb(of: [conversations, conversations]), 388)
    }

    func testAnInstallEventIsAStepAndAnythingElseIsNot() {
        XCTAssertEqual(
            parseSetupStep(#"{"event":"progress","stage":"install","state":"downloading","tool":"speech-model","mb":319,"of":310}"#),
            .downloading(tool: "speech-model", mb: 310, of: 310)
        )
        XCTAssertEqual(
            parseSetupStep(#"{"event":"progress","stage":"install","state":"done","tool":"vhs"}"#),
            .done(tool: "vhs")
        )
        XCTAssertNil(parseSetupStep(#"{"event":"progress","stage":"capture","state":"start","tool":"x"}"#))
        XCTAssertNil(parseSetupStep("  % Total    % Received"))
    }

    func testAnAdapterIsSetUpByItsUse() {
        XCTAssertEqual(setupUse(forAdapter: "vhs"), "terminal")
        XCTAssertNil(setupUse(forAdapter: "x11"))
    }
}
