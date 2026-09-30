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
                                    .onTapGesture { model.lineTapped(index) }
                                    .popover(
                                        isPresented: Binding(
                                            get: { model.panelLine == index },
                                            set: { if !$0, model.panelLine == index { model.panelLine = nil } }
                                        ),
                                        arrowEdge: .bottom
                                    ) {
                                        LinePanel(index: index).environmentObject(model)
                                    }
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
            guard model.rewording == nil else { return .ignored }
            model.keep()
            return .handled
        }
        .onKeyPress(.space) {
            guard model.voiced, model.rewording == nil else { return .ignored }
            model.playOrStop()
            return .handled
        }
        .onKeyPress(.escape) {
            guard model.isReading else { return .ignored }
            model.stopReading()
            return .handled
        }
        // F2, as AppKit names it.
        .onKeyPress(keys: [KeyEquivalent("\u{F705}")]) { _ in
            guard model.rewording == nil else { return .ignored }
            model.rewordCurrent()
            return .handled
        }
        .onKeyPress(characters: CharacterSet(charactersIn: "pmsw+=-"), phases: .down) { press in
            guard model.rewording == nil else { return .ignored }
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
                // A speaker's line: their initial in their colour; the
                // narrator's, read by a voice: a waveform. Faint until made.
                if let speaker = line.speaker, !line.recorded {
                    Text(Voice.initial(speaker))
                        .font(Theme.face(13, .bold))
                        .foregroundStyle(Color(hex: Voice.speakerColour(speaker)))
                        .opacity(Voice.mark(line) == .unvoiced ? 0.45 : 1)
                        .padding(.leading, 26)
                        .accessibilityLabel("Said by \(speaker)")
                } else if model.voiced, let mark = Voice.mark(line), mark != .take {
                    Image(systemName: "waveform")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(Theme.ink.opacity(mark == .voiced ? 0.7 : 0.25))
                        .padding(.leading, 26)
                        .accessibilityLabel(mark == .voiced ? "Voiced" : "Not voiced yet")
                }
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
            if model.rewording == index {
                LineEditor()
            } else {
                text
                    .font(Theme.face(size))
                    .lineSpacing(size * 0.12)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
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
/// A line's words, edited where they stand, in the glass's type and
/// underlined in the cue's amber: Return keeps them, Escape leaves them.
private struct LineEditor: View {
    @EnvironmentObject private var model: AppModel
    @FocusState private var focused: Bool

    var body: some View {
        let size = model.textSize
        TextField("", text: $model.rewordText, axis: .vertical)
            .textFieldStyle(.plain)
            .font(Theme.face(size))
            .lineSpacing(size * 0.12)
            .foregroundStyle(Theme.ink)
            .focused($focused)
            .onSubmit { model.endReword(keep: true) }
            .onExitCommand { model.endReword(keep: false) }
            .padding(.bottom, 6)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.cue).frame(height: 2) }
            .frame(maxWidth: .infinity, alignment: .leading)
            .onAppear { focused = true }
    }
}

/// What can be done to a line read by a voice: hear it, read on from it,
/// tell the voice how to say it, have it said anew, or reword it.
private struct LinePanel: View {
    @EnvironmentObject private var model: AppModel
    let index: Int
    @State private var instruct = ""

    var body: some View {
        if model.state.script.lines.indices.contains(index) {
            let line = model.state.script.lines[index]
            let byVoice = line.audio?.source == .voice
            VStack(alignment: .leading, spacing: 14) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Line \(index + 1)").font(Theme.face(15, .bold))
                    Text(!byVoice ? "Read from your take"
                        : line.speaker.map { "Said by \($0)" } ?? "Read by \(model.state.script.voice?.name ?? "the voice")")
                        .font(Theme.face(13))
                        .foregroundStyle(Theme.ink.opacity(0.66))
                }
                HStack(spacing: 8) {
                    Button { model.read(from: index, only: true) } label: {
                        Label("Listen", systemImage: "play.fill")
                    }
                    .buttonStyle(Pill(primary: true))
                    .keyboardShortcut(.defaultAction)
                    Button("Read on from here") { model.read(from: index) }
                        .buttonStyle(Pill())
                }
                if byVoice {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("How to say it").font(Theme.face(13)).foregroundStyle(Theme.ink.opacity(0.66))
                        TextField("slower, amused", text: $instruct)
                            .textFieldStyle(.roundedBorder)
                            .onSubmit { model.instruct(index, instruct) }
                        Text("For a voice that takes directions. Return to apply.")
                            .font(Theme.face(12))
                            .foregroundStyle(Theme.ink.opacity(0.55))
                    }
                }
                HStack(spacing: 8) {
                    if byVoice {
                        Button { model.read(from: index, only: true, fresh: true) } label: {
                            Label("Say it again", systemImage: "arrow.clockwise")
                        }
                        .buttonStyle(Pill())
                        .help("Have the voice make this line anew: for a voice that says a line differently each time")
                    }
                    Button("Reword…") { model.beginReword(index) }
                        .buttonStyle(Pill())
                }
            }
            .padding(18)
            .frame(width: 360)
            .foregroundStyle(Theme.ink)
            .background(Theme.raised)
            .onAppear { instruct = line.instruct ?? "" }
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
