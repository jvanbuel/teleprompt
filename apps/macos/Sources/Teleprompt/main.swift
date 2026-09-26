#if canImport(SwiftUI)
MainActor.assumeIsolated { TelepromptApp.main() }
#else
print("the Teleprompt app runs on macOS; TelepromptKit builds anywhere")
#endif
