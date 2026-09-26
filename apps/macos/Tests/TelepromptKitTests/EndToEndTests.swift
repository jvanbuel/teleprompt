import XCTest
@testable import TelepromptKit

/// The real `teleprompt prompt`, launched and driven as the app does it,
/// hearing a recorded reading of `Tests/Fixtures/tour`. Opt-in: it needs a
/// binary built with `--features listen` and a speech model.
///
///     TELEPROMPT_BIN=…/teleprompt TELEPROMPT_MODEL=…/zipformer swift test --filter EndToEnd
///
/// On Linux, Foundation's WebSockets need a libcurl built with them, which
/// distributions do not ship; on macOS they are URLSession's own.
final class EndToEndTests: XCTestCase {
    func testAReadingIsFollowedAndKept() async throws {
        let env = ProcessInfo.processInfo.environment
        guard let binary = env["TELEPROMPT_BIN"], let model = env["TELEPROMPT_MODEL"] else {
            // CI sets this, so a job that stops providing them cannot go
            // quietly green.
            if env["TELEPROMPT_REQUIRE_E2E"] != nil {
                return XCTFail("TELEPROMPT_REQUIRE_E2E is set, but TELEPROMPT_BIN or TELEPROMPT_MODEL is not")
            }
            throw XCTSkip("set TELEPROMPT_BIN and TELEPROMPT_MODEL")
        }
        // A copy, so the takes it records land nowhere that lasts.
        let project = FileManager.default.temporaryDirectory
            .appendingPathComponent("teleprompt-e2e-\(UUID().uuidString)")
        try FileManager.default.copyItem(
            at: repository.appendingPathComponent("apps/macos/Tests/Fixtures/tour"), to: project
        )
        defer { try? FileManager.default.removeItem(at: project) }
        let script = project.appendingPathComponent("scripts/tour.md").path
        let wav = repository
            .appendingPathComponent("crates/teleprompt-listen-sherpa/tests/fixtures/two-lines.wav").path

        let server = ServerProcess(LaunchRequest(
            binary: URL(fileURLWithPath: binary),
            script: URL(fileURLWithPath: script),
            model: URL(fileURLWithPath: model)
        ))
        let events = AsyncStream<LaunchEvent>.makeStream()
        try server.start { events.continuation.yield($0) }
        defer { server.stop() }
        var launch = events.stream.makeAsyncIterator()
        guard case let .listening(origin) = await launch.next() else {
            return XCTFail("the server did not listen")
        }

        let client = SessionClient(origin: origin)
        let lines = try await client.script().lines
        XCTAssertEqual(lines.count, 2)
        let messages = AsyncStream<ServerMessage>.makeStream()
        client.open(
            onMessage: { messages.continuation.yield($0) },
            onClose: { reason in
                if let reason { print("session closed: \(reason)") }
                messages.continuation.finish()
            }
        )
        defer { client.close() }
        var heard = messages.stream.makeAsyncIterator()

        let (samples, rate) = try readWav(URL(fileURLWithPath: wav))
        client.send(.start(from: 0, rate: rate))
        let first = await heard.next()
        guard case let .reached(at, _) = first else { return XCTFail("started with \(String(describing: first))") }
        XCTAssertEqual(at, Position(line: 0, word: 0))
        for start in stride(from: 0, to: samples.count, by: rate / 10) {
            client.sendAudio(Array(samples[start..<min(start + rate / 10, samples.count)]))
        }
        // A second of silence, as a reader pauses before keeping a take.
        client.sendAudio([Float](repeating: 0, count: rate))
        client.send(.stop)

        var furthest = Position(line: 0, word: 0)
        while let message = await heard.next() {
            switch message {
            case let .reached(at, _):
                XCTAssertGreaterThanOrEqual(at, furthest, "the reader only moves on")
                furthest = at
            case let .stopped(saved):
                XCTAssertEqual(furthest.line, 2, "followed to the end")
                XCTAssertEqual(saved, lines.map(\.id), "both lines kept")
                let recorded = try await client.script().lines.map(\.recorded)
                XCTAssertEqual(recorded, [true, true])
                return
            default:
                XCTFail("unexpected \(message)")
            }
        }
        XCTFail("the session closed before the take was kept")
    }
}

/// 16-bit PCM mono WAV, as floats, and its rate.
private func readWav(_ url: URL) throws -> ([Float], Int) {
    let data = try Data(contentsOf: url)
    func u32(_ at: Int) -> Int {
        (0..<4).reduce(0) { $0 | Int(data[data.startIndex + at + $1]) << (8 * $1) }
    }
    func u16(_ at: Int) -> Int {
        Int(data[data.startIndex + at]) | Int(data[data.startIndex + at + 1]) << 8
    }
    var at = 12
    var rate = 0
    while at + 8 <= data.count {
        let id = String(decoding: data[(data.startIndex + at)..<(data.startIndex + at + 4)], as: UTF8.self)
        let size = u32(at + 4)
        if id == "fmt " {
            rate = u32(at + 12)
            guard u16(at + 8) == 1, u16(at + 10) == 1, u16(at + 22) == 16 else {
                throw CocoaError(.fileReadCorruptFile)
            }
        }
        if id == "data" {
            let samples = stride(from: at + 8, to: at + 8 + size - 1, by: 2).map {
                Float(Int16(bitPattern: UInt16(u16($0)))) / 32768
            }
            return (samples, rate)
        }
        at += 8 + size + size % 2
    }
    throw CocoaError(.fileReadCorruptFile)
}
