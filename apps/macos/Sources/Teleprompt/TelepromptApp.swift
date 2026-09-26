#if canImport(SwiftUI)
import SwiftUI

struct TelepromptApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate
    @StateObject private var model = AppModel()

    var body: some Scene {
        WindowGroup("Teleprompt") {
            ContentView()
                .environmentObject(model)
                .frame(minWidth: 720, minHeight: 480)
                .onAppear { delegate.model = model }
        }
        .commands { PrompterCommands(model: model) }

        // The screen on its own, for a second display.
        Window("Screen", id: "screen") {
            ScreenView()
                .environmentObject(model)
                .frame(minWidth: 480, minHeight: 270)
        }

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

struct PrompterCommands: Commands {
    @ObservedObject var model: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(after: .newItem) {
            Button("Open Script…") { model.chooseScript = true }
                .keyboardShortcut("o")
        }
        // The plain keys (return, space, m, + −, s) are the prompter's own;
        // see PrompterView. Here they would fire while typing in Settings.
        CommandMenu("Prompter") {
            Button("Take from the Top") { model.take(from: 0) }
                .keyboardShortcut("t")
                .disabled(!model.isReady)
            Button("Keep Take") { model.keep() }
                .keyboardShortcut(.return)
                .disabled(!model.state.listening)
            Button(model.paused ? "Resume" : "Pause") { model.togglePause() }
                .keyboardShortcut("p", modifiers: [.command, .shift])
                .disabled(!model.isTaking)
            Divider()
            Toggle("Mirror Text", isOn: $model.mirrored)
            Button("Larger Text") { model.textSize += 4 }
                .keyboardShortcut("+")
            Button("Smaller Text") { model.textSize = max(16, model.textSize - 4) }
                .keyboardShortcut("-")
            Divider()
            Toggle("Show Screen", isOn: $model.showsScreen)
            Button("Screen in Its Own Window") {
                model.showsScreen = false
                openWindow(id: "screen")
            }
            .keyboardShortcut("s", modifiers: [.command, .shift])
        }
    }
}
#endif
