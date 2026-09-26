import AppKit

// Finds the menu bar icon's button in the status bar window, so the
// right-click menu can click it like a left click would.

public enum StatusButton {
    /// Searches the whole subtree: on macOS 14+ the button sits two levels
    /// below the window's content view.
    @MainActor
    public static func find(in view: NSView) -> NSButton? {
        view as? NSButton ?? view.subviews.lazy.compactMap { find(in: $0) }.first
    }
}
