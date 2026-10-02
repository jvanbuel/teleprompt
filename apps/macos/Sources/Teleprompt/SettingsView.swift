#if canImport(SwiftUI)
import SwiftUI
import UniformTypeIdentifiers

struct SettingsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var choosing: Choice?

    private enum Choice { case binary, model }

    var body: some View {
        Form {
            Section {
                path("teleprompt binary", model.binaryPath) { choosing = .binary }
                Text("Built with `cargo install --path crates/teleprompt-cli --features listen`.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section {
                path("Speech model", model.modelPath) { choosing = .model }
                Text("Not needed when Set Up Teleprompt has installed one: that one is used.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section {
                Button("Set Up Teleprompt…") { model.offerSetup() }
                Text("Install the tools and models for what you want to do.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section {
                TextField("Locale", text: $model.locale)
                Toggle("Count down before a take", isOn: $model.countdownOn)
            }
        }
        .formStyle(.grouped)
        .frame(width: 520)
        .fileImporter(
            isPresented: Binding(get: { choosing != nil }, set: { if !$0 { choosing = nil } }),
            allowedContentTypes: choosing == .model ? [.folder] : [.unixExecutable, .item]
        ) { result in
            guard case let .success(url) = result else { return }
            switch choosing {
            case .binary: model.binaryPath = url.path
            case .model: model.modelPath = url.path
            case nil: break
            }
            choosing = nil
        }
    }

    private func path(_ label: String, _ value: String, choose: @escaping () -> Void) -> some View {
        LabeledContent(label) {
            HStack {
                Text(value.isEmpty ? "not set" : value)
                    .lineLimit(1).truncationMode(.middle)
                    .foregroundStyle(value.isEmpty ? .red : .primary)
                Button("Choose…", action: choose)
            }
        }
    }
}
#endif
