#if canImport(SwiftUI)
import AppKit
import CoreText
import SwiftUI

/// The prompters' design (`apps/DESIGN.md`): the glass is black because a
/// beam-splitter does not reflect black; amber marks the next word and the
/// reading line, and nothing else; red is the tally, and means recording.
enum Theme {
    static let glass = Color.black
    static let ink = Color(red: 0.949, green: 0.957, blue: 0.969)
    static let cue = Color(red: 1.0, green: 0.722, blue: 0.0)
    static let tally = Color(red: 1.0, green: 0.231, blue: 0.188)
    static let recorded = Color(red: 0.239, green: 0.863, blue: 0.518)
    static let missing = Color(red: 0.949, green: 0.545, blue: 0.510)
    /// A take heard saying other words than its line.
    static let heard = Color(red: 0.541, green: 0.710, blue: 0.969)
    static let brought = Color(red: 0.506, green: 0.788, blue: 0.584)
    static let chrome = Color(red: 0.090, green: 0.094, blue: 0.106)
    static let raised = Color(red: 0.125, green: 0.133, blue: 0.153)
    static let line = Color(red: 0.173, green: 0.184, blue: 0.212)

    /// Where the reading line is, as a fraction of the glass's height.
    static let reading: CGFloat = 0.36
    /// A line of the script's height, as a multiple of its size.
    static let leading: CGFloat = 1.32

    static let family = "Atkinson Hyperlegible Next"

    static func face(_ size: CGFloat, _ weight: Font.Weight = .medium) -> Font {
        .custom(family, size: size).weight(weight)
    }

    /// The logo beside the name, for the welcome page.
    static let lockup = icon("teleprompt-lockup-dark.svg")

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

/// A key as a keycap.
struct Keycap: View {
    let key: String

    var body: some View {
        Text(key)
            .font(Theme.face(12, .bold))
            .foregroundStyle(Theme.ink)
            .padding(.horizontal, 7)
            .padding(.vertical, 1)
            .background(RoundedRectangle(cornerRadius: 5).fill(Theme.chrome))
            .overlay(RoundedRectangle(cornerRadius: 5).stroke(Theme.line))
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

extension Color {
    /// `#rrggbb`, as the speakers' colours are written.
    init(hex: String) {
        let value = UInt32(hex.dropFirst(), radix: 16) ?? 0
        self.init(
            red: Double((value >> 16) & 0xFF) / 255,
            green: Double((value >> 8) & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255
        )
    }
}

/// The reading line's cue arrow.
struct CueArrow: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: rect.minX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.midY))
        path.addLine(to: CGPoint(x: rect.minX, y: rect.maxY))
        path.closeSubpath()
        return path
    }
}
#endif
