import XCTest
@testable import TelepromptKit

final class LaunchTests: XCTestCase {
    func testTheCommandAsksForJSONAndAFreePort() {
        let request = LaunchRequest(
            binary: URL(fileURLWithPath: "/bin/teleprompt"),
            script: URL(fileURLWithPath: "/p/scripts/tour.md"),
            model: URL(fileURLWithPath: "/models/zipformer")
        )
        XCTAssertEqual(request.arguments, [
            "--format", "json", "serve", "/p/scripts/tour.md", "--locale", "en",
            "--port", "0", "--model", "/models/zipformer",
        ])
    }

    func testWithoutAModelTheScriptsVoiceReadsIt() {
        let request = LaunchRequest(
            binary: URL(fileURLWithPath: "/bin/teleprompt"),
            script: URL(fileURLWithPath: "/p/scripts/tour.md"),
            model: nil
        )
        XCTAssertEqual(request.arguments.last, "--voice")
        XCTAssertFalse(request.arguments.contains("--model"))
    }

    func testTheListeningExampleNamesTheOrigin() throws {
        let line = String(decoding: try example("listening.json"), as: UTF8.self)
        XCTAssertEqual(parseListening(line), URL(string: "http://127.0.0.1:7879"))
        XCTAssertNil(parseListening(#"{"event":"listening","url":"http://127.0.0.1:1","api":"/api/v2"}"#))
        XCTAssertNil(parseListening("prompting at http://127.0.0.1:1/"))
    }

    func testAnExitSaysWhy() {
        let report = Data("{\n  \"ok\": false,\n  \"errors\": [\n    \"no model\"\n  ]\n}\n".utf8)
        XCTAssertEqual(exitReasons(stdout: report, stderr: Data(), status: 1), ["no model"])
        let stderr = Data((1...8).map { "line \($0)" }.joined(separator: "\n").utf8)
        XCTAssertEqual(exitReasons(stdout: Data(), stderr: stderr, status: 1), (4...8).map { "line \($0)" })
        XCTAssertEqual(exitReasons(stdout: Data(), stderr: Data(), status: 9), ["teleprompt exited with status 9"])
    }

    /// A stand-in for `teleprompt`: a shell script.
    private func fake(_ body: String) throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("fake-teleprompt-\(UUID().uuidString)")
        try "#!/bin/sh\n\(body)\n".write(to: url, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
        return url
    }

    private func launch(_ body: String) throws -> (ServerProcess, XCTestExpectation, Box) {
        let request = LaunchRequest(
            binary: try fake(body),
            script: URL(fileURLWithPath: "/s.md"),
            model: URL(fileURLWithPath: "/m")
        )
        let server = ServerProcess(request)
        let ended = expectation(description: "ended")
        let events = Box()
        try server.start { event in
            events.append(event)
            if case .ended = event { ended.fulfill() }
        }
        return (server, ended, events)
    }

    func testAServerThatListensSaysWhereAndStopsWhenAsked() throws {
        let (server, ended, events) = try launch("""
        echo 'prompting at http://127.0.0.1:4242/' >&2
        echo '{"event":"listening","url":"http://127.0.0.1:4242","api":"/api/v1"}'
        exec sleep 30
        """)
        let deadline = Date().addingTimeInterval(5)
        while events.all.isEmpty, Date() < deadline { Thread.sleep(forTimeInterval: 0.05) }
        XCTAssertEqual(events.all.first, .listening(origin: URL(string: "http://127.0.0.1:4242")!))
        server.stop()
        wait(for: [ended], timeout: 5)
    }

    func testAServerThatFailsSaysWhy() throws {
        let (_, ended, events) = try launch("""
        printf '{\\n  "ok": false,\\n  "errors": [\\n    "no speech model"\\n  ]\\n}\\n'
        exit 1
        """)
        wait(for: [ended], timeout: 5)
        XCTAssertEqual(events.all, [.ended(["no speech model"])])
    }
}

final class Box: @unchecked Sendable {
    private let lock = NSLock()
    private var events: [LaunchEvent] = []
    func append(_ e: LaunchEvent) { lock.withLock { events.append(e) } }
    var all: [LaunchEvent] { lock.withLock { events } }
}
