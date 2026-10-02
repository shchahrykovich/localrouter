import AppKit
import LocalRouterKit
import SwiftUI

/// The Settings window, opened from the gear in the popover and from the
/// right-click menu. One window, made again on each open after a close, so
/// SettingsView reads the config and the login item state again.
@MainActor
final class SettingsWindowController {
    private unowned let model: AppModel
    private var window: NSWindow?

    init(model: AppModel) {
        self.model = model
    }

    /// Opens at `page`, or at the page shown last.
    func show(page: SettingsPage? = nil) {
        // SettingsView keeps its page in this default and follows a change.
        if let page { UserDefaults.standard.set(page.rawValue, forKey: SettingsView.pageKey) }
        if let window, window.isVisible {
            NSApp.activate()
            window.makeKeyAndOrderFront(nil)
            return
        }
        let window = self.window ?? makeWindow()
        self.window = window
        window.contentViewController = NSHostingController(rootView: SettingsWindowView().environment(model))
        window.setContentSize(NSSize(width: 780, height: 560))
        window.center()
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }

    private func makeWindow() -> NSWindow {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 780, height: 560),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false)
        window.title = "\(Instance.current.appName) Settings"
        window.isReleasedWhenClosed = false
        window.contentMinSize = NSSize(width: 640, height: 400)
        return window
    }
}

/// Settings with a footer for `model.message`: the actions here (Trust,
/// Open at login) report their result there, as in the popover.
private struct SettingsWindowView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 0) {
            SettingsView()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            if let m = model.message {
                Divider()
                HStack {
                    Text(m).font(.caption).foregroundStyle(.secondary).lineLimit(2).truncationMode(.middle)
                    Spacer()
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            }
        }
    }
}
