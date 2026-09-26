#if canImport(AVFoundation) && os(macOS)
import AVFoundation

/// The default microphone, as it is: no voice processing, since what it
/// hears is the take. Hands on mono samples about every 100 ms, and the
/// loudness of each buffer.
final class MicCapture: @unchecked Sendable {
    private let engine = AVAudioEngine()
    private let lock = NSLock()
    private var pending: [Float] = []
    private var _sending = false
    let rate: Int

    /// `sink` and `level` are called on the audio thread; `level` with each
    /// buffer's RMS, whether or not sending is on.
    init(sink: @escaping @Sendable ([Float]) -> Void, level: @escaping @Sendable (Float) -> Void) throws {
        let input = engine.inputNode
        let format = input.inputFormat(forBus: 0)
        rate = Int(format.sampleRate)
        let chunk = rate / 10
        input.installTap(onBus: 0, bufferSize: 4096, format: format) { [weak self] buffer, _ in
            guard let self, let channel = buffer.floatChannelData?[0] else { return }
            let samples = Array(UnsafeBufferPointer(start: channel, count: Int(buffer.frameLength)))
            if !samples.isEmpty {
                level((samples.reduce(0) { $0 + $1 * $1 } / Float(samples.count)).squareRoot())
            }
            let ready: [Float]? = self.lock.withLock {
                guard self._sending else { return nil }
                self.pending.append(contentsOf: samples)
                guard self.pending.count >= chunk else { return nil }
                defer { self.pending.removeAll(keepingCapacity: true) }
                return self.pending
            }
            if let ready { sink(ready) }
        }
        try engine.start()
    }

    /// Whether what the microphone hears is sent; while not, it is dropped.
    var sending: Bool {
        get { lock.withLock { _sending } }
        set { lock.withLock { _sending = newValue } }
    }

    /// The samples not yet handed on.
    func flush() -> [Float] {
        lock.withLock {
            defer { pending.removeAll() }
            return pending
        }
    }

    deinit {
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
    }
}
#endif
