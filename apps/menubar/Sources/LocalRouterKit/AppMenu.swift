import Foundation

/// The app's commands, in groups: the right-click menu of the menu bar icon
/// and the "…" menu in the window footer show the same list. The views only
/// draw it; what each command does is `Action`, run by StatusItemController.
public enum AppMenu {
    public enum Action: Hashable, Sendable {
        case openWindow
        case openChromeViaProxy, openProxyLog, showProxyLogFolder
        case copyAgentPrompt, copyMCPCommand, openAgentHelp
        case installCLI, installClaude, installCodex
        case help, askForHelp(AgentApp)
        case checkForUpdates, settings, quit
        /// Traffic by app (ADR 11): the Apps view, the map window, and the
        /// details window of the app with this key.
        case showApps, openTrafficMap, reviewApp(String)
    }

    /// An app that needs review, for the menu (ADR 11, 3.4).
    public struct AppReview: Equatable, Sendable {
        /// The app's key, as in the traffic reply.
        public var app: String
        public var name: String
        public init(app: String, name: String) {
            self.app = app
            self.name = name
        }
    }

    public struct Item: Equatable, Sendable {
        public var title: String
        /// An SF Symbol name.
        public var symbol: String?
        public var action: Action
        /// Small text with the title, for example the help page's host.
        public var detail: String?
        /// A key that runs the item with ⌘.
        public var shortcut: String?
        public var checked = false
        public var emphasized = false
    }

    public indirect enum Entry: Equatable, Sendable {
        case item(Item)
        case header(String)
        case separator
        case submenu(title: String, symbol: String, entries: [Entry])
    }

    /// What the menu depends on; read again at each open.
    public struct Context: Sendable {
        public var appName: String
        public var chromeInstalled: Bool
        /// The daemon reports a proxy log (ADR 08).
        public var hasProxyLog: Bool
        /// The agent apps that are installed.
        public var agentApps: [AgentApp]
        public var cliInstalled: Bool
        public var agentHelpHost: String
        /// "6 of 7 online"; nil when the daemon is not running.
        public var onlineSummary: String?
        /// "↓ 431 MB, ↑ 96.5 MB last hour"; nil while the sampler is off,
        /// which hides the Traffic group (ADR 11, 5.2).
        public var trafficSummary: String?
        /// Apps that need review; the menu shows at most three.
        public var appsToReview: [AppReview]

        public init(appName: String, chromeInstalled: Bool, hasProxyLog: Bool, agentApps: [AgentApp], cliInstalled: Bool,
                    agentHelpHost: String, onlineSummary: String?, trafficSummary: String? = nil,
                    appsToReview: [AppReview] = []) {
            self.trafficSummary = trafficSummary
            self.appsToReview = appsToReview
            self.appName = appName
            self.chromeInstalled = chromeInstalled
            self.hasProxyLog = hasProxyLog
            self.agentApps = agentApps
            self.cliInstalled = cliInstalled
            self.agentHelpHost = agentHelpHost
            self.onlineSummary = onlineSummary
        }
    }

    /// `window`: the icon's menu, which starts with the item that opens the
    /// window. The footer menu is inside the window and leaves it out.
    public static func entries(_ c: Context, window: Bool) -> [Entry] {
        var out: [Entry] = []
        if window {
            out.append(.item(Item(title: "Open \(c.appName)", symbol: "point.3.connected.trianglepath.dotted", action: .openWindow,
                                  detail: c.onlineSummary, emphasized: true)))
            out.append(.separator)
        }
        if let summary = c.trafficSummary {
            out.append(.header("Traffic"))
            out.append(.item(Item(title: "Show Apps", symbol: "chart.bar", action: .showApps, detail: summary)))
            out.append(.item(Item(title: "Open Traffic Map", symbol: "globe.europe.africa", action: .openTrafficMap)))
            for review in c.appsToReview.prefix(3) {
                out.append(.item(Item(title: "Review \(review.name)…", symbol: "exclamationmark.shield", action: .reviewApp(review.app),
                                      emphasized: true)))
            }
            out.append(.separator)
        }
        if c.chromeInstalled || c.hasProxyLog {
            out.append(.header("Proxy"))
            if c.chromeInstalled {
                out.append(.item(Item(title: "Open Chrome via Proxy", symbol: "globe", action: .openChromeViaProxy)))
            }
            if c.hasProxyLog {
                out.append(.item(Item(title: "Open Proxy Log", symbol: "doc.text", action: .openProxyLog)))
                out.append(.item(Item(title: "Show Proxy Log Folder", symbol: "folder", action: .showProxyLogFolder)))
            }
            out.append(.separator)
        }
        out.append(.header("Coding agents"))
        out.append(.item(Item(title: "Copy Prompt for a Coding Agent", symbol: "doc.on.doc", action: .copyAgentPrompt)))
        out.append(.item(Item(title: "Copy MCP Command", symbol: "terminal", action: .copyMCPCommand)))
        out.append(.item(Item(title: "Open Agent Instructions", symbol: "arrow.up.right.square", action: .openAgentHelp,
                              detail: c.agentHelpHost)))
        out.append(.submenu(title: "Install", symbol: "square.and.arrow.down", entries: [
            .item(Item(title: "Command Line Tool…", action: .installCLI, checked: c.cliInstalled)),
            .item(Item(title: "Claude Code Instructions…", action: .installClaude)),
            .item(Item(title: "Codex Instructions…", action: .installCodex)),
        ]))
        out.append(.separator)
        if c.agentApps.isEmpty {
            out.append(.item(Item(title: "Help", symbol: "questionmark.circle", action: .help)))
        } else {
            out.append(.submenu(title: "Help", symbol: "questionmark.circle", entries:
                [.item(Item(title: "Help", action: .help))]
                    + c.agentApps.map { .item(Item(title: "Help with \($0.name)", action: .askForHelp($0))) }))
        }
        out.append(.item(Item(title: "Check for Updates…", symbol: "arrow.clockwise", action: .checkForUpdates)))
        out.append(.item(Item(title: "Settings…", symbol: "slider.horizontal.3", action: .settings, shortcut: ",")))
        out.append(.separator)
        out.append(.item(Item(title: "Quit \(c.appName)", action: .quit, shortcut: "q")))
        return out
    }
}
