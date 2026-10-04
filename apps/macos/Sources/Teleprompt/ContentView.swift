#if canImport(SwiftUI)
import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            switch model.phase {
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
        .onAppear { model.showHome() }
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
                    if let last = model.lastScriptURL { model.open(last) } else { model.showHome() }
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
