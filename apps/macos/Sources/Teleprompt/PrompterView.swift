#if canImport(SwiftUI)
import SwiftUI
import TelepromptKit

/// The script, large, following the reader: said words dim, the next one
/// marked, the reader's line kept a third of the way down.
struct PrompterView: View {
    @EnvironmentObject private var model: AppModel
    @FocusState private var focused: Bool

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: model.textSize * 0.6) {
                    ForEach(Array(model.state.script.lines.enumerated()), id: \.offset) { index, line in
                        LineView(index: index, line: line)
                            .id(index)
                            .contentShape(Rectangle())
                            .onTapGesture { model.take(from: index) }
                    }
                    // Room to scroll the last line up to the reading point.
                    Color.clear.frame(height: 600)
                }
                .padding(.horizontal, 48)
                .padding(.vertical, 120)
            }
            .scaleEffect(x: model.mirrored ? -1 : 1, y: 1)
            .background(Color.black)
            .onChange(of: model.state.at.line) { _, line in
                withAnimation(.easeInOut(duration: 0.6)) {
                    proxy.scrollTo(min(line, model.state.script.lines.count - 1), anchor: UnitPoint(x: 0, y: 0.3))
                }
            }
        }
        .focusable()
        .focusEffectDisabled()
        .focused($focused)
        .onAppear { focused = true }
        .onKeyPress(.return) {
            model.keep()
            return .handled
        }
        .onKeyPress(.space) {
            model.togglePause()
            return .handled
        }
        .onKeyPress(characters: CharacterSet(charactersIn: "ms+=-"), phases: .down) { press in
            switch press.characters {
            case "m": model.mirrored.toggle()
            case "s": model.showsScreen.toggle()
            case "+", "=": model.textSize += 4
            default: model.textSize = max(16, model.textSize - 4)
            }
            return .handled
        }
    }
}

private struct LineView: View {
    @EnvironmentObject private var model: AppModel
    let index: Int
    let line: Script.Line

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            // A current take of the line.
            RoundedRectangle(cornerRadius: 2)
                .fill(line.recorded ? Color.green : Color.clear)
                .frame(width: 4)
            text
                .font(.system(size: model.textSize, weight: .medium))
                .lineSpacing(model.textSize * 0.2)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// The words, each styled by where the reader is, with a marker where
    /// each shot starts.
    private var text: Text {
        let words = line.words
        var text = Text("")
        for (w, word) in words.enumerated() {
            text = text + markers(before: w) + styled(word, model.state.word(line: index, word: w)) + Text(" ")
        }
        return text + markers(before: words.count)
    }

    private func styled(_ word: String, _ state: PrompterState.Word) -> Text {
        switch state {
        case .said: Text(word).foregroundColor(.white.opacity(0.35))
        case .next: Text(word).foregroundColor(.yellow).underline()
        case .ahead: Text(word).foregroundColor(.white)
        }
    }

    private func markers(before word: Int) -> Text {
        model.state.shots(at: Position(line: index, word: word)).reduce(Text("")) { text, shot in
            let started = model.state.started.contains(shot.shot)
            let label = shot.clip == nil ? "▶ \(shot.shot) (not captured) " : "▶ \(shot.shot) "
            return text + Text(label)
                .font(.system(size: model.textSize * 0.4, weight: .semibold, design: .monospaced))
                .foregroundColor(shot.clip == nil ? .red : started ? .gray : .cyan)
        }
    }
}
#endif
