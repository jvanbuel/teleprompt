#if canImport(SwiftUI)
import AVKit
import SwiftUI
import TelepromptKit

/// The shot playing, or a slate saying why there is none.
struct ScreenView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        ZStack {
            Color.black
            if let shot = model.state.playing {
                if model.state.script.shots.first(where: { $0.shot == shot })?.clip == nil {
                    slate("\(shot) was never captured: run teleprompt capture")
                } else {
                    VideoPlayer(player: model.player)
                        .disabled(true)
                }
            } else {
                slate(model.state.started.isEmpty ? "shots play here as you reach them" : "")
            }
        }
    }

    private func slate(_ text: String) -> some View {
        Text(text)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .padding()
    }
}
#endif
