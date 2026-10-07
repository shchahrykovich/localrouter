import AppKit
import LocalRouterKit
import SwiftUI

/// The colors of the design (the "LocalRouter menu bar redesign" canvas).
/// Surfaces and lines are system colors, so light and dark mode come from
/// macOS; only the accent and the status colors are our own.
enum Theme {
    /// Text and icons in the accent color.
    static let accent = Color(light: 0x0E7C6B, dark: 0x3FC7AE)
    /// A fill under white text.
    static let accentFill = Color(light: 0x0E7C6B, dark: 0x138A76)
    static let accentSoft = Color(light: 0xE3F2EF, dark: 0x17463F)
    static let accentSoftText = Color(light: 0x0B5F53, dark: 0x7FE3CF)
    static let rowHover = Color(light: 0xF3F8F6, dark: 0x2D3533)

    static let online = Color(light: 0x23A55A, dark: 0x30C46B)
    static let warning = Color(light: 0xE08A00, dark: 0xF0A030)
    static let warningText = Color(light: 0x8A4B00, dark: 0xF5B556)
    static let warningSoft = Color(light: 0xFFF4E5, dark: 0x3A2A12)
    static let warningLine = Color(light: 0xF3D7AE, dark: 0x5A4220)
    static let successText = Color(light: 0x17663A, dark: 0x7FD8A0)
    static let successSoft = Color(light: 0xE4F4EA, dark: 0x1C3B28)
    static let dangerText = Color(light: 0xA1251B, dark: 0xFF8A80)
    static let dangerSoft = Color(light: 0xFBE1DE, dark: 0x4A1E1B)
    static let dangerRow = Color(light: 0xFDF3F2, dark: 0x3A1C1A)
    static let neutralSoft = Color(light: 0xF1F1EE, dark: 0x3A3A3E)
    static let proxyText = Color(light: 0x4A3BA8, dark: 0xB7AEFF)
    static let proxySoft = Color(light: 0xECEAF8, dark: 0x2E2A4A)

    /// The window's background. Solid: the popover's own material is almost
    /// clear on macOS 26, and the page behind it showed through.
    static let ground = Color(light: 0xF4F4F2, dark: 0x1C1C1E)
    static let card = Color(nsColor: .controlBackgroundColor)
    static let line = Color(nsColor: .separatorColor)
    static let field = Color.primary.opacity(0.06)

    static let mono = Font.system(size: 11, design: .monospaced)
}

extension Color {
    /// A color that follows the light or dark appearance.
    init(light: UInt32, dark: UInt32) {
        self.init(nsColor: NSColor(name: nil) { appearance in
            let rgb = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? dark : light
            return NSColor(srgbRed: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255,
                           blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
        })
    }
}

/// A white (dark grey in dark mode) box with a thin line and round corners.
struct Card<Content: View>: View {
    var padding: CGFloat = 0
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Theme.line, lineWidth: 0.5))
    }
}

/// A small upper-case title above a card, with an optional control at the end.
struct SectionLabel<Trailing: View>: View {
    let text: String
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(text).textCase(.uppercase).kerning(0.3)
                .font(.system(size: 11, weight: .semibold)).foregroundStyle(.secondary)
            Spacer()
            trailing
        }
        .padding(.horizontal, 4)
    }
}

extension SectionLabel where Trailing == EmptyView {
    init(_ text: String) { self.init(text: text) { EmptyView() } }
}

/// A short label: a route's kind, a status, a script rule.
struct Chip: View {
    enum Kind { case outline, accent, success, warning, danger, neutral, proxy, solid }
    let text: String
    var kind: Kind = .outline
    var mono = false

    var body: some View {
        Text(text)
            .font(mono ? .system(size: 10.5, weight: .semibold, design: .monospaced) : .system(size: 10.5, weight: weight))
            .foregroundStyle(foreground)
            .lineLimit(1)
            .padding(.horizontal, 5)
            .padding(.vertical, 1.5)
            .background(background, in: RoundedRectangle(cornerRadius: 4))
            .overlay {
                if kind == .outline { RoundedRectangle(cornerRadius: 4).strokeBorder(Theme.line, lineWidth: 1) }
            }
    }

    private var weight: Font.Weight { kind == .outline ? .regular : .semibold }

    private var foreground: Color {
        switch kind {
        case .outline: .secondary
        case .accent: Theme.accentSoftText
        case .success: Theme.successText
        case .warning: Theme.warningText
        case .danger: Theme.dangerText
        case .neutral: .primary
        case .proxy: Theme.proxyText
        case .solid: .white
        }
    }

    private var background: Color {
        switch kind {
        case .outline: .clear
        case .accent: Theme.accentSoft
        case .success: Theme.successSoft
        case .warning: Theme.warningSoft
        case .danger: Theme.dangerSoft
        case .neutral: Theme.neutralSoft
        case .proxy: Theme.proxySoft
        case .solid: Theme.accentFill
        }
    }
}

/// A borderless icon button that shows a light square under the pointer.
struct IconButton: View {
    let symbol: String
    let help: String
    var size: CGFloat = 26
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: size * 0.48))
                .frame(width: size, height: size)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
        .background(hovering ? Theme.field : .clear, in: RoundedRectangle(cornerRadius: 6))
        .onHover { hovering = $0 }
        .help(help)
        .accessibilityLabel(help)
    }
}

/// A text button in the accent color, as a link on a web page.
struct LinkButton: View {
    let title: String
    let action: () -> Void

    init(_ title: String, action: @escaping () -> Void) {
        self.title = title
        self.action = action
    }

    var body: some View {
        Button(title, action: action)
            .buttonStyle(.plain)
            .font(.system(size: 12, weight: .medium))
            .foregroundStyle(Theme.accent)
            .pointingHandCursor()
    }
}

/// A command or a value to copy, in a monospaced box with a copy button.
struct CopyLine: View {
    @Environment(AppModel.self) private var model
    let text: String
    /// Shown before the text, for example `$` before a shell command.
    var prompt: String?

    var body: some View {
        HStack(spacing: 8) {
            if let prompt { Text(prompt).font(Theme.mono).foregroundStyle(.secondary) }
            Text(text).font(.system(size: 11.5, design: .monospaced)).textSelection(.enabled).lineLimit(3)
                .frame(maxWidth: .infinity, alignment: .leading)
            IconButton(symbol: "doc.on.doc", help: "Copy", size: 22) { model.copy(text) }
        }
        .padding(.leading, 10)
        .padding(.trailing, 6)
        .padding(.vertical, 5)
        .background(Theme.field.opacity(0.6), in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.line, lineWidth: 0.5))
    }
}

/// Segments that fill the width: the window tabs, the Traffic scopes.
struct Segments<Item: Hashable, Label: View>: View {
    let items: [Item]
    @Binding var selection: Item
    var fill = true
    @ViewBuilder let label: (Item, Bool) -> Label

    var body: some View {
        HStack(spacing: 2) {
            ForEach(items, id: \.self) { item in
                let selected = item == selection
                Button { selection = item } label: {
                    label(item, selected)
                        .font(.system(size: fill ? 12 : 11, weight: selected ? .semibold : .medium))
                        .foregroundStyle(selected ? .primary : .secondary)
                        .frame(maxWidth: fill ? .infinity : nil)
                        .padding(.horizontal, fill ? 0 : 9)
                        .padding(.vertical, fill ? 5 : 3)
                        .background {
                            if selected {
                                RoundedRectangle(cornerRadius: fill ? 6 : 5).fill(Theme.card)
                                    .shadow(color: .black.opacity(0.12), radius: 1, y: 1)
                            }
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(selected ? .isSelected : [])
            }
        }
        .padding(2)
        .background(Color.primary.opacity(0.07), in: RoundedRectangle(cornerRadius: fill ? 8 : 7))
    }
}

/// The app's mark: the menu bar symbol, white on the accent color, grey
/// while the daemon is not running.
struct AppMark: View {
    var active = true
    var body: some View {
        Image(systemName: "point.3.connected.trianglepath.dotted")
            .font(.system(size: 14, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 28, height: 28)
            .background(active ? Theme.accentFill : Color.gray, in: RoundedRectangle(cornerRadius: 7))
            .accessibilityHidden(true)
    }
}

/// A search field in a rounded box.
struct FilterField: View {
    let prompt: String
    @Binding var text: String

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").font(.system(size: 11)).foregroundStyle(.secondary)
            TextField(prompt, text: $text).textFieldStyle(.plain).font(.system(size: 12))
            if !text.isEmpty {
                Button { text = "" } label: { Image(systemName: "xmark.circle.fill") }
                    .buttonStyle(.plain).foregroundStyle(.secondary).help("Clear")
            }
        }
        .padding(.horizontal, 8)
        .frame(height: 26)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 7))
        .overlay(RoundedRectangle(cornerRadius: 7).strokeBorder(Theme.line, lineWidth: 0.5))
    }
}

extension View {
    /// The hand cursor over a link, as in a browser. `.pointerStyle(.link)`
    /// needs macOS 15; the app supports 14.
    func pointingHandCursor() -> some View {
        onHover { inside in
            if inside { NSCursor.pointingHand.push() } else { NSCursor.pop() }
        }
    }
}
