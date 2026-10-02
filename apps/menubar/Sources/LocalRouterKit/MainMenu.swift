import AppKit

/// The app's main menu. LocalRouter is an accessory app, so macOS never shows
/// this menu; it is here for its key equivalents. A text field gets ⌘A, ⌘C,
/// ⌘V, ⌘X and ⌘Z only from menu items like these: with no main menu the
/// keys do nothing in the search, filter and host fields.
@MainActor
public enum MainMenu {
    public static func make(appName: String) -> NSMenu {
        let main = NSMenu()
        main.addItem(submenu(appName, [
            item("Quit \(appName)", #selector(NSApplication.terminate(_:)), "q"),
        ]))
        main.addItem(submenu("Edit", [
            item("Undo", Selector(("undo:")), "z"),
            item("Redo", Selector(("redo:")), "z", [.command, .shift]),
            .separator(),
            item("Cut", #selector(NSText.cut(_:)), "x"),
            item("Copy", #selector(NSText.copy(_:)), "c"),
            item("Paste", #selector(NSText.paste(_:)), "v"),
            item("Select All", #selector(NSText.selectAll(_:)), "a"),
        ]))
        main.addItem(submenu("Window", [
            item("Close", #selector(NSWindow.performClose(_:)), "w"),
        ]))
        return main
    }

    private static func submenu(_ title: String, _ items: [NSMenuItem]) -> NSMenuItem {
        let menu = NSMenu(title: title)
        items.forEach(menu.addItem)
        let holder = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        holder.submenu = menu
        return holder
    }

    /// No target: the action goes along the responder chain to the field
    /// or window that has the focus.
    private static func item(_ title: String, _ action: Selector, _ key: String,
                             _ mods: NSEvent.ModifierFlags = .command) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.keyEquivalentModifierMask = mods
        return item
    }
}
