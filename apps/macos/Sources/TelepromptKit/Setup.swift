import Foundation

/// One use of teleprompt, as `teleprompt setup --uses` says it: render
/// videos, show a browser, follow your voice. What each needs and how it
/// installs are the CLI's; the app asks.
public struct SetupUse: Decodable, Equatable, Sendable, Identifiable {
    public var name: String
    public var label: String
    public var listens: Bool
    /// Whether this teleprompt can do it at all.
    public var available: Bool
    public var installed: Bool
    public var downloadMb: UInt32
    public var tools: [SetupTool]

    public var id: String { name }

    enum CodingKeys: String, CodingKey {
        case name, label, listens, available, installed, tools
        case downloadMb = "download_mb"
    }

    /// What it still needs that `setup` can install.
    public var missing: [SetupTool] { tools.filter { $0.installed == false } }

    /// What the list says beside it: installed, or what it needs.
    public var state: String {
        if !available { return "Needs teleprompt built with speech models" }
        if installed { return "Installed" }
        let models = missing.filter { $0.downloadMb != nil }
        var parts = missing.filter { $0.downloadMb == nil }.map(\.name)
        switch models.count {
        case 0: break
        case 1: parts.append("a model")
        default: parts.append("\(models.count) models")
        }
        let needs = "Needs \(parts.joined(separator: ", "))"
        return downloadMb == 0 ? needs : "\(needs) · \(downloadMb) MB"
    }
}

/// A tool or model a use needs.
public struct SetupTool: Decodable, Equatable, Sendable {
    public var name: String
    public var what: String
    /// Nil where teleprompt cannot tell: a server of your own.
    public var installed: Bool?
    public var license: String
    public var command: String?
    public var downloadMb: UInt32?
    public var password: Bool

    enum CodingKeys: String, CodingKey {
        case name, what, installed, license, command, password
        case downloadMb = "download_mb"
    }
}

/// The uses, as `--uses` printed them.
public func parseUses(_ data: Data) throws -> [SetupUse] {
    struct Report: Decodable { var uses: [SetupUse] }
    return try JSONDecoder().decode(Report.self, from: data).uses
}

/// What the chosen uses download, each model once though several need it.
public func downloadMb(of chosen: [SetupUse]) -> UInt32 {
    var seen = Set<String>()
    var total: UInt32 = 0
    for tool in chosen.flatMap(\.missing) {
        guard let mb = tool.downloadMb, seen.insert(tool.name).inserted else { continue }
        total += mb
    }
    return total
}

/// How an install goes, tool by tool.
public enum SetupStep: Equatable, Sendable {
    case started(tool: String)
    case downloading(tool: String, mb: UInt32, of: UInt32)
    case done(tool: String)
}

/// The step an event line on stderr says, if it says one.
public func parseSetupStep(_ line: String) -> SetupStep? {
    struct Event: Decodable {
        var event: String
        var stage: String
        var state: String?
        var tool: String?
        var mb: UInt32?
        var of: UInt32?
    }
    guard let data = line.trimmingCharacters(in: .whitespaces).data(using: .utf8),
          let e = try? JSONDecoder().decode(Event.self, from: data),
          e.event == "progress", e.stage == "install", let tool = e.tool
    else { return nil }
    switch e.state {
    case "start": return .started(tool: tool)
    case "downloading":
        guard let mb = e.mb, let of = e.of else { return nil }
        return .downloading(tool: tool, mb: min(mb, of), of: of)
    case "done": return .done(tool: tool)
    default: return nil
    }
}

/// The use that sets up what recording with `adapter` needs, if one does.
public func setupUse(forAdapter adapter: String) -> String? {
    ["vhs": "terminal", "asciinema": "casts", "playwright": "browser"][adapter]
}

/// Asks `binary` what it can be set up to do.
public func fetchSetupUses(binary: URL) async throws -> [SetupUse] {
    let (status, stdout, stderr) = try await run(binary, ["--format", "json", "setup", "--uses"])
    guard status == 0 else {
        throw SetupError(reasons: exitReasons(stdout: stdout, stderr: stderr, status: status))
    }
    return try parseUses(stdout)
}

/// Installs what `uses` need with `binary`, telling `onStep`, on some
/// background queue, how it goes. Throws why it failed, as teleprompt says.
public func installSetup(
    binary: URL,
    uses: [String],
    onStep: @escaping @Sendable (SetupStep) -> Void
) async throws {
    let (status, stdout, stderr) = try await run(
        binary, ["--format", "json", "setup"] + uses + ["--run"], onLine: { line in
            if let step = parseSetupStep(line) { onStep(step) }
        })
    guard status == 0 else {
        throw SetupError(reasons: exitReasons(stdout: stdout, stderr: stderr, status: status))
    }
}

public struct SetupError: Error, Equatable {
    public var reasons: [String]
}

/// The speech model `teleprompt setup` installed, if it did: where the CLI
/// puts it, `$TELEPROMPT_MODELS` or `~/.local/share/teleprompt/models`.
public func installedSpeechModel() -> URL? {
    let env = ProcessInfo.processInfo.environment
    let models = env["TELEPROMPT_MODELS"].map { URL(fileURLWithPath: $0) }
        ?? env["XDG_DATA_HOME"].map { URL(fileURLWithPath: $0).appendingPathComponent("teleprompt/models") }
        ?? FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".local/share/teleprompt/models")
    let model = models.appendingPathComponent("sherpa-onnx-streaming-zipformer-en-2023-06-26")
    var isDir: ObjCBool = false
    return FileManager.default.fileExists(atPath: model.path, isDirectory: &isDir) && isDir.boolValue
        ? model : nil
}

/// Runs `binary` with `arguments` to the end: its status, stdout and
/// stderr, each stderr line handed to `onLine` as it comes.
private func run(
    _ binary: URL,
    _ arguments: [String],
    onLine: (@Sendable (String) -> Void)? = nil
) async throws -> (Int32, Data, Data) {
    let process = Process()
    process.executableURL = binary
    process.arguments = arguments
    process.standardInput = FileHandle.nullDevice
    let out = Pipe()
    let err = Pipe()
    process.standardOutput = out
    process.standardError = err
    let buffers = Buffers()
    out.fileHandleForReading.readabilityHandler = { handle in
        buffers.append(handle.availableData, toStderr: false, onLine: nil)
    }
    err.fileHandleForReading.readabilityHandler = { handle in
        buffers.append(handle.availableData, toStderr: true, onLine: onLine)
    }
    return try await withCheckedThrowingContinuation { continuation in
        process.terminationHandler = { process in
            out.fileHandleForReading.readabilityHandler = nil
            err.fileHandleForReading.readabilityHandler = nil
            buffers.append(out.fileHandleForReading.readDataToEndOfFile(), toStderr: false, onLine: nil)
            buffers.append(err.fileHandleForReading.readDataToEndOfFile(), toStderr: true, onLine: onLine)
            let (stdout, stderr) = buffers.taken()
            continuation.resume(returning: (process.terminationStatus, stdout, stderr))
        }
        do {
            try process.run()
        } catch {
            process.terminationHandler = nil
            continuation.resume(throwing: error)
        }
    }
}

/// What a process printed, gathered from its pipes' queues: stderr split
/// into lines as they complete.
private final class Buffers: @unchecked Sendable {
    private let lock = NSLock()
    private var stdout = Data()
    private var stderr = Data()
    private var partial = ""

    func append(_ chunk: Data, toStderr: Bool, onLine: (@Sendable (String) -> Void)?) {
        let lines: [String] = lock.withLock {
            guard toStderr else {
                stdout.append(chunk)
                return []
            }
            stderr.append(chunk)
            partial += String(decoding: chunk, as: UTF8.self)
            var complete = partial.components(separatedBy: "\n")
            partial = complete.removeLast()
            return complete
        }
        if let onLine { lines.forEach(onLine) }
    }

    func taken() -> (Data, Data) {
        lock.withLock { (stdout, stderr) }
    }
}
