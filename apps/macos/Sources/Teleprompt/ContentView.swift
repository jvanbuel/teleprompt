#if canImport(SwiftUI)
import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            switch model.phase {
            case .idle:
                Welcome()
            case let .launching(script):
                Loading(name: script.lastPathComponent)
            case .ready:
                HStack(spacing: 0) {
                    PrompterView()
                    if model.showsScreen && !model.state.script.shots.isEmpty {
                        MonitorView()
                    }
                }
                TallyBar()
            case let .failed(reasons):
                Failure(reasons: reasons)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.chrome)
        .foregroundStyle(Theme.ink)
        .preferredColorScheme(.dark)
        .toolbar {
            ToolbarItem(placement: .principal) {
                VStack(spacing: 0) {
                    Text(model.scriptName.isEmpty ? "Teleprompt" : model.scriptName)
                        .font(Theme.face(13, .bold))
                    if !model.projectName.isEmpty {
                        Text(model.projectName)
                            .font(Theme.face(11))
                            .foregroundStyle(Theme.ink.opacity(0.5))
                    }
                }
            }
            ToolbarItem(placement: .primaryAction) {
                if model.isReady { RecordButton() }
            }
        }
        .toolbarBackground(Theme.chrome, for: .windowToolbar)
        .fileImporter(isPresented: $model.chooseScript, allowedContentTypes: [.plainText, .init(filenameExtension: "md")!]) { result in
            if case let .success(url) = result { model.open(url) }
        }
    }
}

/// The header's one action: record from the line the reader is on, or keep
/// the take under way.
struct RecordButton: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let taking = model.isTaking
        Button {
            if taking {
                model.keep()
            } else {
                let at = model.state.at.line
                model.take(from: at < model.state.script.lines.count ? at : 0)
            }
        } label: {
            HStack(spacing: 8) {
                RoundedRectangle(cornerRadius: taking ? 1 : 4)
                    .fill(taking ? Color.black : Color.white)
                    .frame(width: 8, height: 8)
                Text(taking ? "Keep take" : "Record")
                    .font(Theme.face(13, .bold))
            }
            .foregroundStyle(taking ? Color.black : Color.white)
            .padding(.horizontal, 14)
            .padding(.vertical, 5)
            .background(Capsule().fill(taking ? Theme.ink : Theme.tally))
        }
        .buttonStyle(.plain)
        .help("Record from the line you are on (⌘T from the top)")
    }
}

private struct Welcome: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        HStack(spacing: 64) {
            VStack(alignment: .leading, spacing: 18) {
                Text("Teleprompt")
                    .font(Theme.face(44, .heavy))
                Text("Open a script and read it aloud. The words follow your voice, each shot plays as you reach it, and every line you finish is kept as a take.")
                    .font(Theme.face(18))
                    .foregroundStyle(Theme.ink.opacity(0.62))
                    .lineSpacing(4)
                    .frame(maxWidth: 440, alignment: .leading)
                HStack(spacing: 12) {
                    Button("Open script…") { model.chooseScript = true }
                        .buttonStyle(Pill(primary: true))
                        .keyboardShortcut(.defaultAction)
                    if let last = model.lastScriptURL {
                        Button("Reopen \(last.lastPathComponent)") { model.open(last) }
                            .buttonStyle(Pill())
                    }
                }
                .padding(.top, 10)
            }
            GlassSample()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// A slice of the glass, to show what the prompter does before a script is
/// open: the reading line, what is said, the next word.
private struct GlassSample: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            row(false, Text("Every take starts here.").foregroundColor(Theme.ink.opacity(0.32)))
            row(true, Text("The words follow ").foregroundColor(Theme.ink.opacity(0.32))
                + Text("your").foregroundColor(Theme.cue).underline(color: Theme.cue)
                + Text(" voice,").foregroundColor(Theme.ink))
            row(false, Text("and the shots play").foregroundColor(Theme.ink.opacity(0.55)))
            row(false, Text("as you reach them.").foregroundColor(Theme.ink.opacity(0.55)))
        }
        .padding(EdgeInsets(top: 34, leading: 22, bottom: 34, trailing: 40))
        .background(RoundedRectangle(cornerRadius: 14).fill(Theme.glass))
        .overlay(RoundedRectangle(cornerRadius: 14).stroke(Theme.line))
        .shadow(color: .black.opacity(0.45), radius: 30, y: 24)
    }

    private func row(_ current: Bool, _ text: Text) -> some View {
        HStack(spacing: 14) {
            CueArrow().fill(Theme.cue).frame(width: 9, height: 12).opacity(current ? 1 : 0)
            text.font(Theme.face(27))
        }
    }
}

private struct Loading: View {
    let name: String

    var body: some View {
        VStack(spacing: 10) {
            ProgressView().controlSize(.large).padding(.bottom, 8)
            Text("Loading the speech model").font(Theme.face(22, .bold))
            Text(name).font(Theme.face(15)).foregroundStyle(Theme.ink.opacity(0.5))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct Failure: View {
    @EnvironmentObject private var model: AppModel
    let reasons: [String]

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "exclamationmark.triangle")
                .font(.system(size: 40))
                .foregroundStyle(Theme.ink.opacity(0.5))
            Text("The prompter stopped").font(Theme.face(24, .bold))
            ForEach(reasons, id: \.self) { reason in
                Text(reason)
                    .font(Theme.face(15))
                    .foregroundStyle(Theme.ink.opacity(0.7))
                    .multilineTextAlignment(.center)
                    .textSelection(.enabled)
            }
            HStack(spacing: 12) {
                if let last = model.lastScriptURL {
                    Button("Try again") { model.open(last) }.buttonStyle(Pill(primary: true))
                }
                SettingsLink { Text("Settings") }.buttonStyle(Pill())
            }
            .padding(.top, 8)
        }
        .frame(maxWidth: 560)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
#endif
