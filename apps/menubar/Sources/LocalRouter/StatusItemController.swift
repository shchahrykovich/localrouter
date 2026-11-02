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
    /// Traffic by app (ADR 11): the details of one app, and the map.
    private let details: AppDetailsWindowController
    private var map: TrafficMapWindowController!
    /// When the popover last closed. A click on the icon first closes a
    /// transient popover, then reaches `clicked`; without this check the same
    /// click would open it again.
    private var closedAt = Date.distantPast

    init(model: AppModel) {
        self.model = model
        settings = SettingsWindowController(model: model)
        details = AppDetailsWindowController(model: model)
        super.init()
        map = TrafficMapWindowController(model: model) { [weak self] app in self?.details.show(app: app) }
        model.openSettings = { [weak self] page in self?.showSettings(page) }
        popover.behavior = .transient
        popover.delegate = self
        let main = MainView(
            openSettings: { [weak self] page in self?.showSettings(page) },
            perform: { [weak self] action in self?.perform(action, fromWindow: true) })
        let content = NSHostingController(rootView: main.environment(model).frame(width: 480, height: 560))
        content.sizingOptions = .preferredContentSize
        // After the controller: setting it resets contentSize.
        popover.contentViewController = content
        popover.contentSize = NSSize(width: 480, height: 560)
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

    /// Redraw the icon whenever `model.running` or the apps to review change.
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
            // An app needs review (ADR 11, 3.4): a red dot on the icon.
            if model.traffic.showRedDot, let base = image {
                image = Self.withRedDot(base, template: !isDev)
            }
            item.button?.image = image
        } onChange: { [weak self] in
            // Weak in both closures: the outer one would otherwise hold self
            // strongly to hand it to the inner one.
            Task { @MainActor [weak self] in self?.trackIcon() }
        }
    }

    /// The icon with a red dot at its top right. Drawn at display time, so a
    /// template symbol still follows the menu bar's light or dark look.
    private static func withRedDot(_ base: NSImage, template: Bool) -> NSImage {
        let size = base.size
        let image = NSImage(size: size, flipped: false) { rect in
            if template {
                NSColor.labelColor.set()
                base.draw(in: rect)
                rect.fill(using: .sourceIn)
            } else {
                base.draw(in: rect)
            }
            let d = size.height * 0.42
            NSColor.systemRed.setFill()
            NSBezierPath(ovalIn: NSRect(x: size.width - d, y: size.height - d, width: d, height: d)).fill()
            return true
        }
        image.isTemplate = false
        image.accessibilityDescription = "LocalRouter: an app needs review"
        return image
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
    /// a left click still opens the window. The items are AppMenu's, read
    /// again at each open (Chrome, the proxy log and the agent apps may come
    /// and go).
    private func showMenu() {
        item.menu = makeMenu(AppMenu.entries(model.menuContext, window: true))
        item.button?.performClick(nil)
        item.menu = nil
    }

    private func makeMenu(_ entries: [AppMenu.Entry]) -> NSMenu {
        let menu = NSMenu()
        for entry in entries {
            switch entry {
            case .separator:
                menu.addItem(.separator())
            case let .header(title):
                menu.addItem(.sectionHeader(title: title))
            case let .item(entry):
                menu.addItem(menuItem(entry))
            case let .submenu(title, symbol, children):
                let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
                item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
                item.submenu = makeMenu(children)
                menu.addItem(item)
            }
        }
        return menu
    }

    private func menuItem(_ entry: AppMenu.Item) -> NSMenuItem {
        let item = NSMenuItem(title: entry.title, action: #selector(menuAction(_:)), keyEquivalent: entry.shortcut ?? "")
        item.target = self
        item.representedObject = entry.action
        if let symbol = entry.symbol { item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil) }
        if entry.checked { item.state = .on }
        if entry.emphasized {
            item.attributedTitle = NSAttributedString(string: entry.title, attributes: [.font: NSFont.boldSystemFont(ofSize: NSFont.systemFontSize)])
        }
        if let detail = entry.detail {
            if #available(macOS 14.4, *) { item.subtitle = detail } else { item.toolTip = detail }
        }
        return item
    }

    @objc private func menuAction(_ sender: NSMenuItem) {
        guard let action = sender.representedObject as? AppMenu.Action else { return }
        perform(action, fromWindow: false)
    }

    /// Runs a command of the icon's menu or of the window's "…" menu. From
    /// the icon's menu the window is closed, so a result is said in an
    /// alert; in the window it goes to the footer.
    func perform(_ action: AppMenu.Action, fromWindow: Bool) {
        switch action {
        case .openWindow:
            // Menu actions run while the menu is closing; open the window after it.
            DispatchQueue.main.async { self.showWindow() }
        case .openChromeViaProxy: openChromeViaProxy()
        case .openProxyLog: Task { @MainActor in await model.openProxyLog() }
        case .showProxyLogFolder: model.showProxyLogFolder()
        case .copyAgentPrompt: model.copy(model.agentPrompt)
        case .copyMCPCommand: model.copy(model.mcpCommand)
        case .openAgentHelp: model.open(model.agentHelpURL)
        case .installCLI: report(model.installCLI(), fromWindow: fromWindow)
        case .installClaude: report(model.installClaude(), fromWindow: fromWindow)
        case .installCodex: report(model.installCodex(), fromWindow: fromWindow)
        case .help: DispatchQueue.main.async { self.showSettings(.help) }
        case let .askForHelp(app): model.askForHelp(app)
        case .checkForUpdates:
            if fromWindow { Task { @MainActor in await model.checkForUpdates(manual: true) } } else { checkForUpdates() }
        case .settings: DispatchQueue.main.async { self.showSettings() }
        case .quit: NSApp.terminate(nil)
        case .showApps:
            model.tab = .traffic
            model.trafficMode = .apps
            DispatchQueue.main.async { self.showWindow() }
        case .openTrafficMap:
            popover.performClose(nil)
            DispatchQueue.main.async { self.map.show() }
        case let .reviewApp(app):
            popover.performClose(nil)
            DispatchQueue.main.async { self.details.show(app: app) }
        }
    }

    /// The install commands set the footer message themselves.
    private func report(_ feedback: Feedback, fromWindow: Bool) {
        guard !fromWindow else { return }
        DispatchQueue.main.async { self.alert(feedback) }
    }

    /// The window opens to show the result: the proxy address, or why
    /// Chrome was not started.
    private func openChromeViaProxy() {
        Task { @MainActor in
            await model.openChromeViaProxy()
            showWindow()
        }
    }

    /// Says the result in an alert; an available update can be installed
    /// from it.
    private func checkForUpdates() {
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
}
