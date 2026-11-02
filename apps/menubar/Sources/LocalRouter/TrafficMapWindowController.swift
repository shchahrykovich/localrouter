import AppKit
import LocalRouterKit
import SwiftUI

/// The traffic map window (ADR 11, 2.4), opened by Map in the Traffic tab
/// and Open Traffic Map in the menu. Made again on each open after a close,
/// so its polling stops while it is closed.
@MainActor
final class TrafficMapWindowController {
    private unowned let model: AppModel
    private let openApp: (String) -> Void
    private var window: NSWindow?

    init(model: AppModel, openApp: @escaping (String) -> Void) {
        self.model = model
        self.openApp = openApp
    }

    func show() {
        if let window, window.isVisible {
            NSApp.activate()
            window.makeKeyAndOrderFront(nil)
            return
        }
        let window = self.window ?? makeWindow()
        self.window = window
        window.contentViewController = NSHostingController(rootView: TrafficMapView(openApp: openApp).environment(model))
        window.setContentSize(NSSize(width: 1180, height: 760))
        window.center()
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }

    private func makeWindow() -> NSWindow {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1180, height: 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false)
        window.title = "Traffic Map"
        window.isReleasedWhenClosed = false
        window.contentMinSize = NSSize(width: 900, height: 560)
        // Closing drops the view, which stops its polling.
        NotificationCenter.default.addObserver(forName: NSWindow.willCloseNotification, object: window, queue: .main) { [weak window] _ in
            MainActor.assumeIsolated { window?.contentViewController = nil }
        }
        return window
    }
}
