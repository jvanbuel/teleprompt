#if canImport(SwiftUI)
import SwiftUI
import UniformTypeIdentifiers
import TelepromptKit

struct ContentView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            switch model.phase {
            case .idle:
                Welcome()
            case let .launching(script):
                ProgressView("Starting teleprompt for \(script.lastPathComponent)…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            case .ready:
                HSplitView {
                    PrompterView()
                        .frame(minWidth: 400)
                    if model.showsScreen && !model.state.script.shots.isEmpty {
                        ScreenView()
                            .frame(minWidth: 320, idealWidth: 480)
                    }
                }
            case let .failed(reasons):
                Failure(reasons: reasons)
            }
            StatusBar()
        }
        .fileImporter(isPresented: $model.chooseScript, allowedContentTypes: [.plainText, .init(filenameExtension: "md")!]) { result in
            if case let .success(url) = result { model.open(url) }
        }
    }
}

private struct Welcome: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(spacing: 16) {
            Text("Teleprompt").font(.largeTitle)
            Text("Open a script to read it. The prompter follows your voice, plays each shot as you reach it, and records your takes.")
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 420)
            HStack {
                Button("Open Script…") { model.chooseScript = true }
                    .keyboardShortcut(.defaultAction)
                if let last = model.lastScriptURL {
                    Button("Reopen \(last.lastPathComponent)") { model.open(last) }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct Failure: View {
    @EnvironmentObject private var model: AppModel
    let reasons: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("teleprompt stopped", systemImage: "exclamationmark.triangle")
                .font(.title2)
            ForEach(reasons, id: \.self) { reason in
                Text(reason).font(.system(.body, design: .monospaced)).textSelection(.enabled)
            }
            HStack {
                if let last = model.lastScriptURL {
                    Button("Try Again") { model.open(last) }
                }
                Button("Open Script…") { model.chooseScript = true }
                SettingsLink { Text("Settings…") }
            }
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

private struct StatusBar: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        HStack {
            if model.state.listening {
                Image(systemName: "record.circle").foregroundStyle(.red)
            }
            Text(model.state.status.text)
                .foregroundStyle(model.state.status.isError ? .red : .secondary)
            Spacer()
            Text("click a line to take it from there · ⏎ keep · space pause · m mirror · + − size · s screen")
                .foregroundStyle(.tertiary)
        }
        .font(.caption)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(.bar)
    }
}
#endif
