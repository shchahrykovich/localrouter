import AppKit
import LocalRouterKit

/// Right-click on the menu bar icon shows the app commands. MenuBarExtra has
/// no API for this, so a local event monitor catches right-clicks on the
/// status item's window. A left click still opens the LocalRouter window.
@MainActor
final class StatusItemMenu: NSObject {
    private unowned let model: AppModel
    private var monitor: Any?
    private weak var statusWindow: NSWindow?

    init(model: AppModel) {
        self.model = model
    }

    func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .rightMouseDown) { [weak self] event in
            guard let self, let window = event.window, window.className.contains("NSStatusBarWindow"),
                  let view = window.contentView
            else { return event }
            self.statusWindow = window
            self.show(in: view)
            return nil
        }
    }

    private func show(in view: NSView) {
        let menu = NSMenu()
        menu.addItem(item("Open LocalRouter", #selector(openWindow)))
        menu.addItem(.separator())
        menu.addItem(item("Open Agent Instructions (\(AgentHelp.host))", #selector(openAgentHelp)))
        menu.addItem(item("Copy Prompt for a Coding Agent", #selector(copyAgentPrompt)))
        menu.addItem(item("Copy MCP Command", #selector(copyMCPCommand)))
        menu.addItem(.separator())
        menu.addItem(item("Install Command Line Tool…", #selector(installCLI)))
        menu.addItem(item("Install Claude Code Instructions…", #selector(installClaude)))
        menu.addItem(item("Check for Updates…", #selector(checkForUpdates)))
        menu.addItem(.separator())
        menu.addItem(item("Quit LocalRouter", #selector(quit)))
        // A menu takes the appearance of the view it opens in, and the menu
        // bar is dark over a dark wallpaper. Use the system appearance, as a
        // status item's own menu does.
        menu.appearance = NSApp.effectiveAppearance
        let below = view.isFlipped ? view.bounds.maxY + 4 : view.bounds.minY - 4
        let button = StatusButton.find(in: view)
        button?.highlight(true)
        menu.popUp(positioning: nil, at: NSPoint(x: view.bounds.minX, y: below), in: view)
        button?.highlight(false)
    }

    private func item(_ title: String, _ action: Selector) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
        item.target = self
        return item
    }

    /// Click the status item button, as a left click would. Wait until the
    /// menu has closed, so the window does not open during menu tracking.
    @objc private func openWindow() {
        guard let view = statusWindow?.contentView, let button = StatusButton.find(in: view) else { return }
        DispatchQueue.main.async { button.performClick(nil) }
    }

    @objc private func openAgentHelp() { model.open(model.agentHelpURL) }
    @objc private func copyAgentPrompt() { model.copy(model.agentPrompt) }
    @objc private func copyMCPCommand() { model.copy(model.mcpCommand) }

    @objc private func installCLI() {
        model.installCLI()
        openWindow()
    }

    @objc private func installClaude() {
        model.installClaude()
        openWindow()
    }

    /// The result shows in the window footer, so open the window afterwards.
    @objc private func checkForUpdates() {
        Task {
            await model.checkForUpdates(manual: true)
            openWindow()
        }
    }

    @objc private func quit() { NSApp.terminate(nil) }
}
