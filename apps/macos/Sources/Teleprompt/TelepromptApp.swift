#if canImport(SwiftUI)
import SwiftUI

struct TelepromptApp: App {
    init() {
        Theme.registerFonts()
    }

    @NSApplicationDelegateAdaptor private var delegate: AppDelegate
    @StateObject private var model = AppModel()

    var body: some Scene {
        WindowGroup("Teleprompt") {
            ContentView()
                .environmentObject(model)
                .frame(minWidth: 900, minHeight: 560)
                .onAppear { delegate.model = model }
        }
        .commands { PrompterCommands(model: model) }

        Settings {
            SettingsView()
                .environmentObject(model)
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var model: AppModel?

    func applicationWillTerminate(_ notification: Notification) {
        // The server is the app's child; it goes with it.
        MainActor.assumeIsolated { model?.shutdown() }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

/// The app's commands. The prompter's own keys are the page's: its ?
/// lists them.
struct PrompterCommands: Commands {
    @ObservedObject var model: AppModel

    var body: some Commands {
        CommandGroup(after: .appSettings) {
            Button("Set Up Teleprompt…") { model.offerSetup() }
        }
        CommandGroup(after: .newItem) {
            Button("Open Script…") { model.chooseScript = true }
                .keyboardShortcut("o")
            Button("Open in Editor") { model.openInEditor() }
                .keyboardShortcut("e")
                .disabled(model.lastScriptURL == nil)
        }
    }
}
#endif
