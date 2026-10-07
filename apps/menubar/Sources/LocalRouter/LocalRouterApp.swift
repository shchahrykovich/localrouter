import LocalRouterKit
import SwiftUI

/// An AppKit entry point, not a SwiftUI App: the app has no windows of its
/// own, only the menu bar icon and its popover (StatusItemController), and a
/// SwiftUI App with no scene to show opens an empty Settings window.
@main
enum LocalRouterApp {
    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        // No Dock icon, as LSUIElement does for the bundle; also for `swift run`.
        app.setActivationPolicy(.accessory)
        withExtendedLifetime(delegate) { app.run() }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let model = AppModel()
    private var statusItem: StatusItemController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Never shown; it gives text fields ⌘A, ⌘C, ⌘V, ⌘X and ⌘Z.
        NSApp.mainMenu = MainMenu.make(appName: Instance.current.appName)
        statusItem = StatusItemController(model: model)
        model.start()
    }
}

