#if canImport(SwiftUI)
import SwiftUI
import TelepromptKit

/// Why setup opened, and what to carry on with once it has installed.
struct SetupRequest: Identifiable {
    let id = UUID()
    /// Ticked from the start: what a command found missing.
    var wanted: [String] = []
    var why: String?
    var then: (@MainActor () -> Void)?
}

/// The sheet's state: the uses, what is ticked, and how an install goes.
@MainActor
final class SetupModel: ObservableObject {
    @Published private(set) var uses: [SetupUse]?
    @Published var chosen: Set<String> = []
    @Published private(set) var status: String?
    @Published private(set) var failed = false
    @Published private(set) var fraction: Double?
    @Published private(set) var installing = false

    let binary: URL
    let request: SetupRequest

    init(binary: URL, request: SetupRequest) {
        self.binary = binary
        self.request = request
    }

    /// Asks teleprompt what it can set up.
    func look() async {
        do {
            let uses = try await fetchSetupUses(binary: binary)
            self.uses = uses
            if chosen.isEmpty {
                chosen = Set(request.wanted.filter { want in
                    uses.contains { $0.name == want && $0.available && !$0.installed }
                })
            }
        } catch {
            fail(error)
        }
    }

    var chosenUses: [SetupUse] { (uses ?? []).filter { chosen.contains($0.name) } }

    var installLabel: String {
        let mb = downloadMb(of: chosenUses)
        return mb == 0 ? "Install" : "Install · \(mb) MB"
    }

    func install() {
        let names = chosenUses.map(\.name)
        guard !names.isEmpty else { return }
        installing = true
        failed = false
        status = "Starting…"
        fraction = 0
        Task {
            do {
                try await installSetup(binary: binary, uses: names) { [weak self] step in
                    Task { @MainActor in self?.show(step) }
                }
                status = "Installed."
                fraction = nil
                chosen = []
                request.then?()
                await look()
            } catch {
                fraction = nil
                fail(error)
            }
            installing = false
        }
    }

    private func show(_ step: SetupStep) {
        switch step {
        case let .started(tool):
            status = "Installing \(spoken(tool))…"
        case let .downloading(tool, mb, of):
            status = "Downloading \(spoken(tool)) · \(mb) of \(of) MB"
            fraction = Double(mb) / Double(max(of, 1))
        case let .done(tool):
            status = "Installed \(spoken(tool))."
        }
    }

    private func fail(_ error: Error) {
        failed = true
        status = (error as? SetupError)?.reasons.joined(separator: "\n") ?? error.localizedDescription
    }

    /// A tool as a sentence names it.
    private func spoken(_ name: String) -> String {
        ["speech-model": "the speech model", "punctuation-model": "the punctuation model",
         "speaker-model": "the speaker models"][name] ?? name
    }
}

/// Setting teleprompt up: what you want to do with it, each use with what it
/// still needs, and one Install that shows how it goes.
struct SetupView: View {
    @StateObject private var setup: SetupModel
    @Environment(\.dismiss) private var dismiss

    init(binary: URL, request: SetupRequest) {
        _setup = StateObject(wrappedValue: SetupModel(binary: binary, request: request))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Set up teleprompt").font(.title2.bold())
            Text(setup.request.why ?? "Choose what you want to do. Teleprompt ships no tools or models of its own: each installs with your own package manager, under its own license.")
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let uses = setup.uses {
                ScrollView {
                    VStack(spacing: 0) {
                        ForEach(uses) { use in
                            row(use)
                            if use.id != uses.last?.id { Divider() }
                        }
                    }
                }
                .background(RoundedRectangle(cornerRadius: 10).fill(.quaternary.opacity(0.5)))
                .disabled(setup.installing)
            } else {
                ProgressView("Looking at what is installed…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            if let fraction = setup.fraction {
                ProgressView(value: fraction)
            }
            if let status = setup.status {
                Text(status)
                    .foregroundStyle(setup.failed ? .red : .primary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer()
                Button("Close") { dismiss() }
                Button(setup.installLabel) { setup.install() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(setup.chosen.isEmpty || setup.installing)
            }
        }
        .padding(24)
        .frame(width: 600, height: 660)
        .task { await setup.look() }
    }

    @ViewBuilder
    private func row(_ use: SetupUse) -> some View {
        HStack(alignment: .center, spacing: 12) {
            if use.installed {
                Image(systemName: "checkmark").foregroundStyle(.secondary).frame(width: 18)
            } else {
                Toggle("", isOn: Binding(
                    get: { setup.chosen.contains(use.name) },
                    set: { on in
                        if on { setup.chosen.insert(use.name) } else { setup.chosen.remove(use.name) }
                    }
                ))
                .toggleStyle(.checkbox)
                .labelsHidden()
                .frame(width: 18)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(use.label)
                Text(subtitle(use)).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .opacity(use.available ? 1 : 0.5)
        .disabled(!use.available)
    }

    private func subtitle(_ use: SetupUse) -> String {
        let password = use.available && !use.installed && use.missing.contains { $0.password }
        return password ? "\(use.state) · asks for your password" : use.state
    }
}
#endif
