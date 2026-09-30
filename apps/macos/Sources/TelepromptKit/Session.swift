import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// The API of a running server: its script, clips and session socket.
public final class SessionClient: @unchecked Sendable {
    public let origin: URL
    private let urlSession = URLSession(configuration: .ephemeral)
    private var socket: URLSessionWebSocketTask?

    public init(origin: URL) {
        self.origin = origin
    }

    /// A path the server gave, such as a shot's clip, as a URL.
    public func url(_ path: String) -> URL {
        URL(string: path, relativeTo: origin)!.absoluteURL
    }

    /// What the server serves at `path`, such as a line's audio.
    public func data(_ path: String) async throws -> Data {
        try await get(url(path))
    }

    public func script() async throws -> Script {
        let data = try await get(url("\(API.version)/script"))
        return try JSONDecoder().decode(Script.self, from: data)
    }

    /// Opens the session socket. `onMessage` gets each message from the
    /// server and `onClose` the end of the socket, with why if it failed;
    /// both on a background queue.
    public func open(
        onMessage: @escaping @Sendable (ServerMessage) -> Void,
        onClose: @escaping @Sendable (String?) -> Void
    ) {
        var components = URLComponents(url: origin, resolvingAgainstBaseURL: false)!
        components.scheme = origin.scheme == "https" ? "wss" : "ws"
        components.path = "\(API.version)/session"
        let task = urlSession.webSocketTask(with: components.url!)
        socket = task
        task.resume()
        receive(on: task, onMessage: onMessage, onClose: onClose)
    }

    public func send(_ message: ClientMessage) {
        socket?.send(.string(String(decoding: message.json(), as: UTF8.self))) { _ in }
    }

    public func sendAudio(_ samples: [Float]) {
        guard !samples.isEmpty else { return }
        socket?.send(.data(encodeSamples(samples))) { _ in }
    }

    public func close() {
        socket?.cancel(with: .normalClosure, reason: nil)
        socket = nil
    }

    private func receive(
        on task: URLSessionWebSocketTask,
        onMessage: @escaping @Sendable (ServerMessage) -> Void,
        onClose: @escaping @Sendable (String?) -> Void
    ) {
        task.receive { [weak self] result in
            switch result {
            case let .success(.string(text)):
                if let message = try? ServerMessage(json: Data(text.utf8)) { onMessage(message) }
            case let .success(.data(data)):
                if let message = try? ServerMessage(json: data) { onMessage(message) }
            case .success:
                break
            case let .failure(error):
                let closed = task.closeCode == .normalClosure || self?.socket !== task
                onClose(closed ? nil : Self.describe(error, task))
                return
            }
            self?.receive(on: task, onMessage: onMessage, onClose: onClose)
        }
    }

    private static func describe(_ error: Error, _ task: URLSessionWebSocketTask) -> String {
        if let status = (task.response as? HTTPURLResponse)?.statusCode, status == 409 {
            return "another prompter already has the session"
        }
        return "lost the server: \(error.localizedDescription)"
    }

    private func get(_ url: URL) async throws -> Data {
        try await withCheckedThrowingContinuation { continuation in
            urlSession.dataTask(with: url) { data, response, error in
                if let error {
                    continuation.resume(throwing: error)
                } else if let status = (response as? HTTPURLResponse)?.statusCode, status != 200 {
                    continuation.resume(throwing: URLError(.badServerResponse))
                } else {
                    continuation.resume(returning: data ?? Data())
                }
            }.resume()
        }
    }
}
