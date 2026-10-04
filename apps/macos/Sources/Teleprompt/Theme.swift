#if canImport(SwiftUI)
import AppKit
import CoreText
import SwiftUI

/// The prompters' design (`apps/DESIGN.md`), for the app around the page:
/// its chrome, and what shows while the page cannot.
enum Theme {
    static let glass = Color.black
    static let ink = Color(red: 0.949, green: 0.957, blue: 0.969)
    static let cue = Color(red: 1.0, green: 0.722, blue: 0.0)
    static let chrome = Color(red: 0.090, green: 0.094, blue: 0.106)
    static let raised = Color(red: 0.125, green: 0.133, blue: 0.153)
    static let line = Color(red: 0.173, green: 0.184, blue: 0.212)

    static let family = "Atkinson Hyperlegible Next"

    static func face(_ size: CGFloat, _ weight: Font.Weight = .medium) -> Font {
        .custom(family, size: size).weight(weight)
    }

    /// The app's mark, beside the script's name in the toolbar.
    static let mark = icon("teleprompt.svg")

    /// An SVG from `apps/icons`, which NSImage draws itself from macOS 14:
    /// from the bundle's resources, or beside the sources under `swift run`.
    private static func icon(_ name: String) -> NSImage? {
        let bundled = Bundle.main.resourceURL?.appendingPathComponent(name)
        let source = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // Teleprompt
            .deletingLastPathComponent() // Sources
            .deletingLastPathComponent() // macos
            .deletingLastPathComponent() // apps
            .appendingPathComponent("icons/\(name)")
        return [bundled, source].compactMap { $0 }.lazy.compactMap(NSImage.init(contentsOf:)).first
    }

    /// Makes the typeface available when the app runs outside its bundle,
    /// as under `swift run`; in the bundle, Info.plist's
    /// ATSApplicationFontsPath does it.
    static func registerFonts() {
        if NSFontManager.shared.availableFontFamilies.contains(family) { return }
        let fonts = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // Teleprompt
            .deletingLastPathComponent() // Sources
            .deletingLastPathComponent() // macos
            .deletingLastPathComponent() // apps
            .appendingPathComponent("fonts")
        for name in ["AtkinsonHyperlegibleNext[wght].ttf", "AtkinsonHyperlegibleNext-Italic[wght].ttf"] {
            CTFontManagerRegisterFontsForURL(fonts.appendingPathComponent(name) as CFURL, .process, nil)
        }
    }
}

/// A rounded button: ink on black for the main action, raised otherwise.
struct Pill: ButtonStyle {
    var primary = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(Theme.face(14, .bold))
            .foregroundStyle(primary ? Color.black : Theme.ink)
            .padding(.horizontal, 22)
            .padding(.vertical, 10)
            .background(Capsule().fill(primary ? Theme.ink : Theme.raised))
            .opacity(configuration.isPressed ? 0.8 : 1)
    }
}
#endif
