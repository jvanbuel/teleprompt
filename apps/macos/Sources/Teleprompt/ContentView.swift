#if canImport(SwiftUI)
import SwiftUI
import TelepromptKit
import UniformTypeIdentifiers

struct ContentView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            switch model.phase {
            case .welcome:
                Welcome()
            case .launching:
                Loading()
            case let .ready(page):
                PrompterPage(page: page, model: model)
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
                // The app's mark beside the script's name: in the chrome,
                // never on the glass, where it would show in the reflection.
                HStack(spacing: 8) {
                    if let mark = Theme.mark {
                        Image(nsImage: mark)
                            .resizable()
                            .frame(width: 26, height: 26)
                            .accessibilityHidden(true)
                    }
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
            }
        }
        .toolbarBackground(Theme.chrome, for: .windowToolbar)
        .fileImporter(isPresented: $model.chooseScript, allowedContentTypes: [.plainText, .init(filenameExtension: "md")!]) { result in
            if case let .success(url) = result { model.open(url) }
        }
        .alert(
            model.openFailure?.title ?? "",
            isPresented: Binding(get: { model.openFailure != nil }, set: { if !$0 { model.openFailure = nil } })
        ) {
            Button("Close", role: .cancel) {}
        } message: {
            Text(model.openFailure?.errors.joined(separator: "\n") ?? "")
        }
    }
}

private struct Welcome: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        HStack(spacing: 64) {
            VStack(alignment: .leading, spacing: 18) {
                if let lockup = Theme.lockup {
                    Image(nsImage: lockup)
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .frame(height: 64)
                        .accessibilityLabel("Teleprompt")
                } else {
                    Text("Teleprompt")
                        .font(Theme.face(44, .heavy))
                }
                Text("Open a script and read it aloud: the words follow your voice, each shot plays as you reach it, and every line you finish is kept as a take. Or let a voice read it, and direct it line by line.")
                    .font(Theme.face(18))
                    .foregroundStyle(Theme.ink.opacity(0.62))
                    .lineSpacing(4)
                    .frame(maxWidth: 440, alignment: .leading)
                HStack(spacing: 14) {
                    Text("Who narrates")
                        .font(Theme.face(14))
                        .foregroundStyle(Theme.ink.opacity(0.66))
                    Picker("Who narrates", selection: $model.voiceReads) {
                        Text("I read").tag(false)
                        Text("A voice reads").tag(true)
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .fixedSize()
                }
                Button("Open script…") { model.chooseScript = true }
                    .buttonStyle(Pill(primary: true))
                    .keyboardShortcut(.defaultAction)
                    .padding(.top, 10)
                let recent = model.recentScripts
                if !recent.isEmpty {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Recent")
                            .font(Theme.face(13, .bold))
                            .foregroundStyle(Theme.ink.opacity(0.55))
                        ForEach(recent, id: \.self) { script in
                            Button { model.open(script) } label: {
                                VStack(alignment: .leading, spacing: 0) {
                                    Text(script.lastPathComponent).font(Theme.face(14, .bold))
                                    Text(projectDir(of: script).lastPathComponent)
                                        .font(Theme.face(12))
                                        .foregroundStyle(Theme.ink.opacity(0.5))
                                }
                            }
                            .buttonStyle(.plain)
                            .help(script.path)
                        }
                    }
                    .padding(.top, 6)
                }
                Button("Set up what teleprompt needs…") { model.offerSetup() }
                    .buttonStyle(.link)
                    .font(Theme.face(13))
                    .foregroundStyle(Theme.ink.opacity(0.66))
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
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(spacing: 10) {
            ProgressView().controlSize(.large).padding(.bottom, 8)
            Text("Starting the prompter").font(Theme.face(22, .bold))
            Text(model.scriptName).font(Theme.face(15)).foregroundStyle(Theme.ink.opacity(0.5))
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
                Button("Try again") {
                    if let last = model.lastScriptURL { model.open(last) } else { model.showWelcome() }
                }
                .buttonStyle(Pill(primary: true))
                SettingsLink { Text("Settings") }.buttonStyle(Pill())
            }
            .padding(.top, 8)
        }
        .frame(maxWidth: 560)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
#endif
