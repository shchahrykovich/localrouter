import AppKit
import LocalRouterKit
import Observation
import SwiftUI

/// The menu bar icon. A left click toggles the LocalRouter window (a popover),
/// a right click shows the app commands. This replaces SwiftUI's MenuBarExtra,
/// which has no API to open its window from code: the menu commands must open
/// the window to show their result in the footer.
@MainActor
final class StatusItemController: NSObject, NSPopoverDelegate {
    private unowned let model: AppModel
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    private let popover = NSPopover()
    /// When the popover last closed. A click on the icon first closes a
    /// transient popover, then reaches `clicked`; without this check the same
    /// click would open it again.
    private var closedAt = Date.distantPast

    init(model: AppModel) {
        self.model = model
        super.init()
        popover.behavior = .transient
        popover.delegate = self
        let content = NSHostingController(rootView: MainView().environment(model).frame(width: 480, height: 540))
        content.sizingOptions = .preferredContentSize
        // After the controller: setting it resets contentSize.
        popover.contentViewController = content
        popover.contentSize = NSSize(width: 480, height: 540)
        if let button = item.button {
            button.target = self
            button.action = #selector(clicked)
            button.sendAction(on: [.leftMouseDown, .rightMouseDown])
        }
        trackIcon()
    }

    /// Redraw the icon whenever `model.running` changes.
    private func trackIcon() {
        withObservationTracking {
            let name = model.running ? "point.3.filled.connected.trianglepath.dotted" : "point.3.connected.trianglepath.dotted"
            item.button?.image = NSImage(systemSymbolName: name, accessibilityDescription: "LocalRouter")
        } onChange: {
            Task { @MainActor [weak self] in self?.trackIcon() }
        }
    }

    @objc private func clicked() {
        let event = NSApp.currentEvent
        if event?.type == .rightMouseDown || event?.modifierFlags.contains(.control) == true {
            popover.performClose(nil)
            showMenu()
        } else if popover.isShown {
            popover.performClose(nil)
        } else if Date().timeIntervalSince(closedAt) > 0.3 {
            showWindow()
        }
    }

    func showWindow() {
        guard let button = item.button, !popover.isShown else { return }
        NSApp.activate()
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        popover.contentViewController?.view.window?.makeKey()
    }

    func popoverDidClose(_ notification: Notification) {
        closedAt = Date()
    }

    // MARK: Right-click menu

    /// A status item's own menu: macOS positions it, highlights the icon and
    /// gives it the system appearance. It is set only for this one click, so
    /// a left click still opens the window.
    private func showMenu() {
        let menu = NSMenu()
        menu.addItem(menuItem("Open LocalRouter", #selector(openWindow)))
        menu.addItem(.separator())
        menu.addItem(menuItem("Open Agent Instructions (\(AgentHelp.host))", #selector(openAgentHelp)))
        menu.addItem(menuItem("Copy Prompt for a Coding Agent", #selector(copyAgentPrompt)))
        menu.addItem(menuItem("Copy MCP Command", #selector(copyMCPCommand)))
        menu.addItem(.separator())
        menu.addItem(menuItem("Install Command Line Tool…", #selector(installCLI)))
        menu.addItem(menuItem("Install Claude Code Instructions…", #selector(installClaude)))
        menu.addItem(menuItem("Check for Updates…", #selector(checkForUpdates)))
        menu.addItem(.separator())
        menu.addItem(menuItem("Quit LocalRouter", #selector(quit)))
        item.menu = menu
        item.button?.performClick(nil)
        item.menu = nil
    }

    private func menuItem(_ title: String, _ action: Selector) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
        item.target = self
        return item
    }

    /// Menu actions run while the menu is closing; open the window after it.
    @objc private func openWindow() {
        DispatchQueue.main.async { self.showWindow() }
    }

    @objc private func openAgentHelp() { model.open(model.agentHelpURL) }

    /// The copy commands say "Copied …" in the footer, so show it.
    @objc private func copyAgentPrompt() {
        model.copy(model.agentPrompt)
        openWindow()
    }

    @objc private func copyMCPCommand() {
        model.copy(model.mcpCommand)
        openWindow()
    }

    @objc private func installCLI() {
        model.installCLI()
        openWindow()
    }

    @objc private func installClaude() {
        model.installClaude()
        openWindow()
    }

    /// The window footer shows "Checking…" and then the result.
    @objc private func checkForUpdates() {
        openWindow()
        Task { await model.checkForUpdates(manual: true) }
    }

    @objc private func quit() { NSApp.terminate(nil) }
}
