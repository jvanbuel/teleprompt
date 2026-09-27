#if canImport(SwiftUI)
import SwiftUI

/// The tally bar: whether the take is on air, its running time, the
/// microphone's level, what just happened, and the keys that matter now.
struct TallyBar: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let onAir = model.state.listening
        HStack(spacing: 12) {
            Circle()
                .fill(onAir ? Theme.tally : Theme.ink.opacity(0.18))
                .frame(width: 10, height: 10)
                .shadow(color: onAir ? Theme.tally.opacity(0.7) : .clear, radius: 5)
            TimelineView(.periodic(from: .now, by: 0.1)) { context in
                Text(model.timecode(at: context.date))
                    .font(Theme.face(15, .semibold))
                    .monospacedDigit()
                    .foregroundStyle(onAir ? Theme.ink : Theme.ink.opacity(0.4))
            }
            GeometryReader { g in
                ZStack(alignment: .leading) {
                    Capsule().fill(Theme.line)
                    Capsule().fill(Theme.recorded).frame(width: g.size.width * (onAir ? model.level : 0))
                }
            }
            .frame(width: 72, height: 4)
            Text(model.state.status.text)
                .font(Theme.face(14))
                .foregroundStyle(model.state.status.isError ? Theme.missing : Theme.ink.opacity(0.7))
                .lineLimit(1)
                .padding(.leading, 6)
            Spacer()
            ForEach(keys, id: \.0) { pair in
                HStack(spacing: 6) {
                    Keycap(key: pair.0)
                    Text(pair.1).font(Theme.face(13)).foregroundStyle(Theme.ink.opacity(0.6))
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(Theme.raised)
        .overlay(alignment: .top) { Rectangle().fill(Theme.line).frame(height: 1) }
    }

    private var keys: [(String, String)] {
        if model.counting != nil { return [("⌘⇧Space", "Cancel")] }
        if model.paused { return [("⌘⇧Space", "Keep take"), ("P", "Resume")] }
        if model.state.listening { return [("⌘⇧Space", "Keep take"), ("P", "Pause")] }
        return [("⌘⇧Space", "Record"), ("⌘T", "From the top"), ("M", "Mirror")]
    }
}
#endif
