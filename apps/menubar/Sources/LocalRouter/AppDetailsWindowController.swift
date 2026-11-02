import AppKit
import LocalRouterKit
import SwiftUI

/// The details window of one app (ADR 11, 3.5), opened by a row of the Apps
/// view, by Review, or by the menu. One window, showing the last app asked for.
@MainActor
final class AppDetailsWindowController {
    private unowned let model: AppModel
    private var window: NSWindow?

    init(model: AppModel) {
        self.model = model
    }

    /// `app` is the app's key, as in the traffic reply.
    func show(app: String) {
        let window = self.window ?? makeWindow()
        self.window = window
        window.contentViewController = NSHostingController(rootView: AppDetailsView(appKey: app).environment(model))
        window.setContentSize(NSSize(width: 620, height: 720))
        if !window.isVisible { window.center() }
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }

    private func makeWindow() -> NSWindow {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 620, height: 720),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false)
        window.title = "App Details"
        window.isReleasedWhenClosed = false
        window.contentMinSize = NSSize(width: 520, height: 480)
        return window
    }
}
