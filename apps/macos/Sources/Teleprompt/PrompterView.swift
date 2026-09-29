#if canImport(SwiftUI)
import SwiftUI
import TelepromptKit

/// The glass: the script on black, line numbers in a gutter, a reading line
/// with a cue arrow a third of the way down, and the text gliding up to it.
struct PrompterView: View {
    @EnvironmentObject private var model: AppModel
    @FocusState private var focused: Bool
    /// Each line's rendered height, to glide to the line the reader is on
    /// within a wrapped paragraph.
    @State private var heights: [Int: CGFloat] = [:]

    var body: some View {
        GeometryReader { geometry in
            let height = geometry.size.height
            ZStack(alignment: .topLeading) {
                ScrollViewReader { proxy in
                    ScrollView(.vertical, showsIndicators: false) {
                        VStack(alignment: .leading, spacing: model.textSize * 0.58) {
                            ForEach(Array(model.state.script.lines.enumerated()), id: \.offset) { index, line in
                                LineView(index: index, line: line)
                                    .id(index)
                                    .background(GeometryReader { g in
                                        Color.clear.preference(key: Heights.self, value: [index: g.size.height])
                                    })
                                    .contentShape(Rectangle())
                                    .onTapGesture { model.take(from: index) }
                            }
                        }
                        .padding(.top, height * Theme.reading)
                        .padding(.bottom, height * (1 - Theme.reading))
                        .padding(.trailing, 72)
                    }
                    .onPreferenceChange(Heights.self) { heights = $0 }
                    .onChange(of: model.state.at) { _, at in glide(proxy, to: at, height: height) }
                    .onChange(of: height) { _, h in glide(proxy, to: model.state.at, height: h) }
                }
                fades(height)
                CueArrow()
                    .fill(Theme.cue)
                    .frame(width: 12, height: 16)
                    .position(x: 16, y: height * Theme.reading)
                    .allowsHitTesting(false)
                if let beat = model.counting {
                    ZStack {
                        Color.black.opacity(0.78)
                        Text("\(beat)")
                            .font(Theme.face(180, .light))
                            .monospacedDigit()
                            .foregroundStyle(Theme.ink.opacity(0.92))
                    }
                    .allowsHitTesting(false)
                }
            }
            .scaleEffect(x: model.mirrored ? -1 : 1, y: 1)
            .overlay(alignment: .bottom) { toast }
        }
        .sheet(isPresented: Binding(
            get: { model.reviewing != nil },
            set: { if !$0 { model.keepScript() } }
        )) {
            if let line = model.reviewing, model.state.script.lines.indices.contains(line) {
                ReviewSaid(index: line, line: model.state.script.lines[line])
                    .environmentObject(model)
            }
        }
        .background(Theme.glass)
        .focusable()
        .focusEffectDisabled()
        .focused($focused)
        .onAppear { focused = true }
        .onKeyPress(.return) {
            model.keep()
            return .handled
        }
        .onKeyPress(characters: CharacterSet(charactersIn: "pmsw+=-"), phases: .down) { press in
            switch press.characters {
            case "p": model.togglePause()
            case "w": model.reviewSaid()
            case "m": model.mirrored.toggle()
            case "s": model.showsScreen.toggle()
            case "+", "=": model.textSize = min(120, model.textSize + 4)
            default: model.textSize = max(24, model.textSize - 4)
            }
            return .handled
        }
    }

    /// Scrolls so the display line holding the next word sits on the
    /// reading line. SwiftUI does not say where a paragraph wraps, so the
    /// line is estimated from how far through the paragraph the reader is.
    private func glide(_ proxy: ScrollViewProxy, to at: Position, height: CGFloat) {
        let lines = model.state.script.lines
        guard !lines.isEmpty else { return }
        let index = min(at.line, lines.count - 1)
        let rowHeight = model.textSize * Theme.leading
        let paragraph = heights[index] ?? rowHeight
        let rows = max(1, (paragraph / rowHeight).rounded())
        let words = lines[index].words
        let through = at.line >= lines.count || words.isEmpty ? 1 : Double(min(at.word, words.count)) / Double(words.count)
        let row = min(rows - 1, (CGFloat(through) * rows).rounded(.down))
        let centre = row * rowHeight + rowHeight / 2
        // scrollTo lines up the same unit point of the line and the glass:
        // pick the one that puts `centre` on the reading line.
        let unit = height == paragraph ? 0 : (Theme.reading * height - centre) / (height - paragraph)
        withAnimation(.easeOut(duration: 0.45)) {
            proxy.scrollTo(index, anchor: UnitPoint(x: 0, y: unit))
        }
    }

    private func fades(_ height: CGFloat) -> some View {
        VStack(spacing: 0) {
            LinearGradient(colors: [.black.opacity(0.85), .clear], startPoint: .top, endPoint: .bottom)
                .frame(height: height * 0.2)
            Spacer()
            LinearGradient(colors: [.clear, .black.opacity(0.9)], startPoint: .top, endPoint: .bottom)
                .frame(height: height * 0.22)
        }
        .allowsHitTesting(false)
    }

    @ViewBuilder private var toast: some View {
        if let toast = model.toast {
            Text(toast)
                .font(Theme.face(14, .bold))
                .foregroundStyle(Theme.ink)
                .padding(.horizontal, 20)
                .padding(.vertical, 10)
                .background(Capsule().fill(Theme.raised))
                .overlay(Capsule().stroke(Theme.line))
                .padding(.bottom, 24)
                .transition(.move(edge: .bottom).combined(with: .opacity))
        }
    }
}

private struct Heights: PreferenceKey {
    static let defaultValue: [Int: CGFloat] = [:]
    static func reduce(value: inout [Int: CGFloat], nextValue: () -> [Int: CGFloat]) {
        value.merge(nextValue()) { $1 }
    }
}

/// A line of the script: its number and a tick if it has a take, then its
/// words, each styled by where the reader is.
private struct LineView: View {
    @EnvironmentObject private var model: AppModel
    let index: Int
    let line: Script.Line

    var body: some View {
        let size = model.textSize
        let current = index == model.state.at.line
        HStack(alignment: .top, spacing: 0) {
            ZStack(alignment: .leading) {
                if line.recorded {
                    Image(systemName: "checkmark")
                        .font(.system(size: 11, weight: .heavy))
                        .foregroundStyle(Theme.recorded)
                        .padding(.leading, 10)
                }
                Text("\(index + 1)")
                    .font(Theme.face(14, current ? .bold : .medium))
                    .foregroundStyle(Theme.ink.opacity(current ? 0.9 : 0.32))
                    .frame(maxWidth: .infinity, alignment: .trailing)
                    .padding(.trailing, 22)
                // Heard saying other words: W reviews it.
                if line.said != nil {
                    Circle()
                        .fill(Theme.heard)
                        .frame(width: 7, height: 7)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                        .padding(.trailing, 8)
                }
            }
            .frame(width: 76, height: size * Theme.leading)
            text
                .font(Theme.face(size))
                .lineSpacing(size * 0.12)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// The words, with a small diamond where each shot starts.
    private var text: Text {
        let words = line.words
        var text = Text("")
        for (w, word) in words.enumerated() {
            text = text + markers(before: w) + styled(word, w) + Text(" ")
        }
        return text + markers(before: words.count)
    }

    private func styled(_ word: String, _ w: Int) -> Text {
        let state = model.state
        if index > state.at.line {
            return Text(word).foregroundColor(Theme.ink.opacity(0.55))
        }
        switch state.word(line: index, word: w) {
        case .said: return Text(word).foregroundColor(Theme.ink.opacity(0.3))
        case .next: return Text(word).foregroundColor(Theme.cue).underline(color: Theme.cue)
        case .ahead: return Text(word).foregroundColor(Theme.ink)
        }
    }

    private func markers(before word: Int) -> Text {
        model.state.shots(at: Position(line: index, word: word)).reduce(Text("")) { text, shot in
            let colour: Color = shot.clip == nil ? Theme.missing
                : model.state.started.contains(shot.shot) ? Theme.ink.opacity(0.22)
                : Theme.ink.opacity(0.6)
            return text + Text("◆ ")
                .font(Theme.face(model.textSize * 0.5))
                .baselineOffset(model.textSize * 0.12)
                .foregroundColor(colour)
        }
    }
}
/// A line against what its take said: the words not said struck through,
/// the ones said instead in bold, to keep either.
private struct ReviewSaid: View {
    @EnvironmentObject private var model: AppModel
    let index: Int
    let line: Script.Line

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Keep what you said on line \(index + 1)?")
                .font(Theme.face(19, .bold))
            Text("The take says this, not what the line reads. Keep it, and the line is reworded to match: nothing to record again.")
                .font(Theme.face(14))
                .foregroundStyle(Theme.ink.opacity(0.7))
                .fixedSize(horizontal: false, vertical: true)
            diff.font(Theme.face(20)).fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Keep the Script") { model.keepScript() }
                    .keyboardShortcut(.cancelAction)
                Button("Use What I Said") { model.keepSaid() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(width: 520)
        .foregroundStyle(Theme.ink)
        .background(Theme.raised)
    }

    private var diff: Text {
        (line.saidDiff ?? []).reduce(Text("")) { text, change in
            let words: Text = switch change {
            case let .same(w): Text(w)
            case let .gone(w): Text(w).strikethrough(color: Theme.missing).foregroundColor(Theme.missing)
            case let .new(w): Text(w).bold().foregroundColor(Theme.brought)
            }
            return text + words + Text(" ")
        }
    }
}
#endif
