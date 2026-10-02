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
    private let settings: SettingsWindowController
    /// When the popover last closed. A click on the icon first closes a
    /// transient popover, then reaches `clicked`; without this check the same
    /// click would open it again.
    private var closedAt = Date.distantPast

    init(model: AppModel) {
        self.model = model
        settings = SettingsWindowController(model: model)
        super.init()
        popover.behavior = .transient
        popover.delegate = self
        let main = MainView(openSettings: { [weak self] page in self?.showSettings(page) })
        let content = NSHostingController(rootView: main.environment(model).frame(width: 480, height: 540))
        content.sizingOptions = .preferredContentSize
        // After the controller: setting it resets contentSize.
        popover.contentViewController = content
        popover.contentSize = NSSize(width: 480, height: 540)
        if let button = item.button {
            button.target = self
            button.action = #selector(clicked)
            button.sendAction(on: [.leftMouseDown, .rightMouseDown])
            let name = Instance.current.appName
            button.toolTip = BuildKind.current == .dev ? "\(name) (development build)" : name
        }
        trackIcon()
    }

    /// Orange icon: a local build, or any instance that is not the release.
    private let isDev = BuildKind.current == .dev || !Instance.current.isRelease

    /// Redraw the icon whenever `model.running` changes.
    private func trackIcon() {
        withObservationTracking {
            let name = model.running ? "point.3.filled.connected.trianglepath.dotted" : "point.3.connected.trianglepath.dotted"
            var image = NSImage(systemSymbolName: name, accessibilityDescription: "LocalRouter")
            // A local build is orange, so it is not mistaken for the release.
            // The status bar ignores contentTintColor, so the color is drawn
            // into the image, which then must not be a template.
            if isDev {
                image = image?.withSymbolConfiguration(.init(paletteColors: [.systemOrange]))
                image?.isTemplate = false
            }
            item.button?.image = image
        } onChange: { [weak self] in
            // Weak in both closures: the outer one would otherwise hold self
            // strongly to hand it to the inner one.
            Task { @MainActor [weak self] in self?.trackIcon() }
        }
    }

    @objc private func clicked() {
        let event = NSApp.currentEvent
        if event?.type == .rightMouseDown || event?.modifierFlags.contains(.control) == true {
            popover.performClose(nil)
            // The button calls this on mouse-down and tracks the mouse until
            // mouse-up in the event tracking run loop mode. A block on the main
            // queue runs in that mode too, so the menu would open during the
            // tracking and take its mouse-up; the button would then take the
            // next real click as its mouse-up. The default mode runs only
            // after the tracking ends.
            RunLoop.main.perform(inModes: [.default]) { MainActor.assumeIsolated { self.showMenu() } }
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

    /// The popover closes first: it is transient and would sit over the window.
    func showSettings(_ page: SettingsPage? = nil) {
        popover.performClose(nil)
        settings.show(page: page)
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
        // Only when Google Chrome is installed, checked at each open (ADR 06, I17).
        if model.chromeInstalled {
            menu.addItem(menuItem("Open Chrome via Proxy", #selector(openChromeViaProxy)))
        }
        menu.addItem(.separator())
        let helpHost = model.agentHelpURL.replacingOccurrences(of: "http://", with: "").replacingOccurrences(of: "https://", with: "")
        menu.addItem(menuItem("Open Agent Instructions (\(helpHost))", #selector(openAgentHelp)))
        menu.addItem(menuItem("Copy Prompt for a Coding Agent", #selector(copyAgentPrompt)))
        menu.addItem(menuItem("Copy MCP Command", #selector(copyMCPCommand)))
        menu.addItem(.separator())
        menu.addItem(menuItem("Install Command Line Tool…", #selector(installCLI)))
        menu.addItem(menuItem("Install Claude Code Instructions…", #selector(installClaude)))
        menu.addItem(menuItem("Install Codex Instructions…", #selector(installCodex)))
        menu.addItem(menuItem("Check for Updates…", #selector(checkForUpdates)))
        menu.addItem(.separator())
        menu.addItem(menuItem("Help", #selector(openHelp)))
        // Only the agent apps that are installed, checked at each open.
        for app in AgentApp.allCases where app.isInstalled {
            let item = menuItem("Help with \(app.name)", #selector(askForHelp(_:)))
            item.representedObject = app.rawValue
            menu.addItem(item)
        }
        menu.addItem(.separator())
        menu.addItem(menuItem("Settings…", #selector(openSettings)))
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

    @objc private func openSettings() {
        DispatchQueue.main.async { self.showSettings() }
    }

    @objc private func openHelp() {
        DispatchQueue.main.async { self.showSettings(.help) }
    }

    @objc private func askForHelp(_ sender: NSMenuItem) {
        guard let raw = sender.representedObject as? String, let app = AgentApp(rawValue: raw) else { return }
        model.askForHelp(app)
    }

    /// The window opens to show the result: the proxy address, or why
    /// Chrome was not started.
    @objc private func openChromeViaProxy() {
        Task { @MainActor in
            await model.openChromeViaProxy()
            showWindow()
        }
    }

    @objc private func openAgentHelp() { model.open(model.agentHelpURL) }
    @objc private func copyAgentPrompt() { model.copy(model.agentPrompt) }
    @objc private func copyMCPCommand() { model.copy(model.mcpCommand) }

    @objc private func installCLI() {
        let feedback = model.installCLI()
        DispatchQueue.main.async { self.alert(feedback) }
    }

    @objc private func installClaude() {
        let feedback = model.installClaude()
        DispatchQueue.main.async { self.alert(feedback) }
    }

    @objc private func installCodex() {
        let feedback = model.installCodex()
        DispatchQueue.main.async { self.alert(feedback) }
    }

    /// Says the result in an alert; an available update can be installed
    /// from it.
    @objc private func checkForUpdates() {
        Task { @MainActor in
            await model.checkForUpdates(manual: true)
            switch model.update {
            case let .available(release):
                let answer = alert(.updateAvailable(release, current: model.version), buttons: ["Install", "Later"])
                guard answer == .alertFirstButtonReturn else { return }
                await model.installUpdate(release)
                // On success the app quits; it is still here only on failure.
                if case let .failed(why) = model.update { alert(.failed("Could not install the update", why)) }
            case .upToDate:
                alert(.upToDate(version: model.version))
            case let .failed(why):
                alert(.failed("Could not check for updates", why))
            case .idle, .checking, .downloading:
                break
            }
        }
    }

    /// An alert in front of other apps; LocalRouter has no window to attach
    /// it to.
    @discardableResult
    private func alert(_ feedback: Feedback, buttons: [String] = []) -> NSApplication.ModalResponse {
        NSApp.activate()
        let alert = NSAlert()
        alert.messageText = feedback.title
        alert.informativeText = feedback.detail
        alert.alertStyle = feedback.failed ? .warning : .informational
        for title in buttons { alert.addButton(withTitle: title) }
        return alert.runModal()
    }

    @objc private func quit() { NSApp.terminate(nil) }
}
