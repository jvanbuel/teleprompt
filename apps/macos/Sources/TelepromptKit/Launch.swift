import Foundation

/// How to run `teleprompt prompt` for the app.
public struct LaunchRequest: Equatable, Sendable {
    /// The `teleprompt` binary, built with `--features listen` to follow a
    /// reader by ear.
    public var binary: URL
    public var script: URL
    /// An unpacked sherpa-onnx streaming zipformer, to follow a reader by
    /// ear; without one, the script's voice reads it.
    public var model: URL?
    public var locale: String

    public init(binary: URL, script: URL, model: URL?, locale: String = "en") {
        self.binary = binary
        self.script = script
        self.model = model
        self.locale = locale
    }

    /// JSON output, so the app can read where it listens; port 0, so the
    /// OS picks a free one.
    public var arguments: [String] {
        ["--format", "json", "prompt", script.path, "--locale", locale, "--port", "0"]
            + (model.map { ["--model", $0.path] } ?? ["--voice"])
    }
}

/// What a launched server reports.
public enum LaunchEvent: Equatable, Sendable {
    /// Serving the API at `origin`.
    case listening(origin: URL)
    /// It stopped, or never started: its errors, or the tail of its stderr.
    case ended([String])
}

/// The listening event, if `line` is one.
public func parseListening(_ line: String) -> URL? {
    struct Listening: Decodable { var event: String; var url: String; var api: String }
    guard let data = line.data(using: .utf8),
          let event = try? JSONDecoder().decode(Listening.self, from: data),
          event.event == "listening", event.api == API.version
    else { return nil }
    return URL(string: event.url)
}

/// Why a server that exited did: the errors of its JSON error report on
/// stdout, else the last lines of its stderr, else its exit status.
public func exitReasons(stdout: Data, stderr: Data, status: Int32) -> [String] {
    struct Report: Decodable { var errors: [String] }
    if let report = try? JSONDecoder().decode(Report.self, from: stdout), !report.errors.isEmpty {
        return report.errors
    }
    let tail = String(decoding: stderr, as: UTF8.self)
        .split(separator: "\n").suffix(5).map(String.init)
    return tail.isEmpty ? ["teleprompt exited with status \(status)"] : tail
}

/// A running `teleprompt prompt`: started, watched and stopped by the app.
public final class ServerProcess: @unchecked Sendable {
    private let process = Process()
    private let lock = NSLock()
    private var stdout = Data()
    private var stderr = Data()
    private var listening = false

    public init(_ request: LaunchRequest) {
        process.executableURL = request.binary
        process.arguments = request.arguments
    }

    /// Starts the server; `onEvent` is called, on some background queue,
    /// once it listens and once it ends.
    public func start(onEvent: @escaping @Sendable (LaunchEvent) -> Void) throws {
        let out = Pipe()
        let err = Pipe()
        process.standardOutput = out
        process.standardError = err
        out.fileHandleForReading.readabilityHandler = { [weak self] handle in
            guard let self else { return }
            let chunk = handle.availableData
            if let origin = self.took(chunk, into: \.stdout) {
                onEvent(.listening(origin: origin))
            }
        }
        err.fileHandleForReading.readabilityHandler = { [weak self] handle in
            _ = self?.took(handle.availableData, into: \.stderr)
        }
        // Holds on to the server until it has said how it ended, even if
        // the app has let go of it; the cycle ends with the process.
        process.terminationHandler = { process in
            out.fileHandleForReading.readabilityHandler = nil
            err.fileHandleForReading.readabilityHandler = nil
            defer { process.terminationHandler = nil }
            // What was still in the pipes when it exited.
            _ = self.took(out.fileHandleForReading.readDataToEndOfFile(), into: \.stdout)
            _ = self.took(err.fileHandleForReading.readDataToEndOfFile(), into: \.stderr)
            let reasons = self.lock.withLock {
                exitReasons(stdout: self.stdout, stderr: self.stderr, status: process.terminationStatus)
            }
            onEvent(.ended(reasons))
        }
        try process.run()
    }

    public func stop() {
        if process.isRunning { process.terminate() }
    }

    /// Appends `chunk`; the origin, the first time stdout holds the
    /// listening event.
    private func took(_ chunk: Data, into buffer: ReferenceWritableKeyPath<ServerProcess, Data>) -> URL? {
        lock.withLock {
            self[keyPath: buffer].append(chunk)
            guard buffer == \ServerProcess.stdout, !listening else { return nil }
            for line in String(decoding: stdout, as: UTF8.self).split(separator: "\n") {
                if let origin = parseListening(String(line)) {
                    listening = true
                    return origin
                }
            }
            return nil
        }
    }
}
