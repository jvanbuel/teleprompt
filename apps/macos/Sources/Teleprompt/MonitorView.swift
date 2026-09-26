#if canImport(SwiftUI)
import AVFoundation
import SwiftUI
import TelepromptKit

/// The monitor: the shot on screen at this point in the script, and the
/// rundown of every shot with what has played.
struct MonitorView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            MonitorScreen()
            HStack {
                Text(model.state.playing.map(displayName) ?? "")
                    .font(Theme.face(15, .bold))
                    .foregroundStyle(Theme.ink)
                Spacer()
                Text(model.clipTime)
                    .font(Theme.face(13))
                    .monospacedDigit()
                    .foregroundStyle(Theme.ink.opacity(0.55))
            }
            .frame(height: 21)
            .padding(.top, 12)
            GeometryReader { g in
                ZStack(alignment: .leading) {
                    Capsule().fill(Theme.line)
                    Capsule().fill(Theme.ink).frame(width: g.size.width * model.clipFraction)
                }
            }
            .frame(height: 3)
            .padding(.top, 8)
            Text("Rundown")
                .font(Theme.face(13, .bold))
                .foregroundStyle(Theme.ink.opacity(0.55))
                .padding(.top, 28)
                .padding(.bottom, 6)
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(model.state.script.shots, id: \.shot) { shot in
                        RundownRow(shot: shot)
                    }
                }
            }
        }
        .padding(20)
        .frame(width: 380)
        .background(Theme.chrome)
        .overlay(alignment: .leading) { Rectangle().fill(Theme.line).frame(width: 1) }
    }
}

/// The 16:9 screen: the clip, or a slate saying why there is none, or what
/// comes next.
struct MonitorScreen: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        ZStack {
            Theme.glass
            if let shot = model.state.playing {
                if model.state.script.shots.first(where: { $0.shot == shot })?.clip == nil {
                    slate("This shot was never captured. Run teleprompt capture to record it.", missing: true)
                } else {
                    PlayerLayer(player: model.player)
                }
            } else if let next = model.nextShot {
                slate("Next: \(next.name), at line \(next.line)")
            } else {
                slate(model.state.started.isEmpty ? "Each shot plays here as your reading reaches it." : "Every shot has played.")
            }
        }
        .aspectRatio(16 / 9, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.line))
    }

    private func slate(_ text: String, missing: Bool = false) -> some View {
        Text(text)
            .font(Theme.face(15))
            .multilineTextAlignment(.center)
            .foregroundStyle(missing ? Theme.missing : Theme.ink.opacity(0.42))
            .padding(24)
    }
}

private struct RundownRow: View {
    @EnvironmentObject private var model: AppModel
    let shot: Script.Shot

    var body: some View {
        let state = model.state
        let onScreen = state.playing == shot.shot
        let played = !onScreen && state.started.contains(shot.shot)
        let captured = shot.clip != nil
        HStack(spacing: 10) {
            ZStack {
                if onScreen {
                    Circle().fill(Theme.ink)
                } else if played {
                    Circle().fill(Theme.ink.opacity(0.3))
                } else {
                    Circle().stroke(captured ? Theme.ink.opacity(0.55) : Theme.missing, lineWidth: 1.5)
                }
            }
            .frame(width: 8, height: 8)
            Text(displayName(shot.shot))
                .font(Theme.face(14, onScreen ? .bold : .medium))
                .foregroundStyle(Theme.ink.opacity(played ? 0.35 : 1))
            Spacer()
            Text(where_(onScreen: onScreen, played: played, captured: captured))
                .font(Theme.face(13))
                .monospacedDigit()
                .foregroundStyle(!captured ? Theme.missing : onScreen ? Theme.ink : Theme.ink.opacity(played ? 0.35 : 0.45))
        }
        .padding(.vertical, 7)
        .padding(.horizontal, 4)
    }

    private func where_(onScreen: Bool, played: Bool, captured: Bool) -> String {
        if onScreen { return "on screen" }
        if !captured { return "not captured" }
        if played { return "played" }
        return shot.at.word == 0 ? "line \(shot.at.line + 1)" : "line \(shot.at.line + 1), word \(shot.at.word + 1)"
    }
}

/// The clip, with no playback controls: the prompter drives it.
struct PlayerLayer: NSViewRepresentable {
    let player: AVPlayer

    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        let layer = AVPlayerLayer(player: player)
        layer.videoGravity = .resizeAspect
        view.layer = layer
        view.wantsLayer = true
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {
        (view.layer as? AVPlayerLayer)?.player = player
    }
}
#endif
