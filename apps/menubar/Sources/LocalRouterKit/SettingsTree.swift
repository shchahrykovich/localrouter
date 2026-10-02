import Foundation

/// One page of the Settings window. The window shows the pages as a tree on
/// the left, with a search field above it, and the selected page on the right.
public enum SettingsPage: String, CaseIterable, Identifiable, Sendable {
    case general, routing, certificates, proxy, inspection, scripts, daemon, updates, help

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .general: "General"
        case .routing: "Routing"
        case .certificates: "HTTPS Certificates"
        case .proxy: "Proxy"
        case .inspection: "Inspection"
        case .scripts: "Scripts"
        case .daemon: "Daemon"
        case .updates: "Updates"
        case .help: "Help"
        }
    }

    /// An SF Symbol name.
    public var symbol: String {
        switch self {
        case .general: "gearshape"
        case .routing: "arrow.triangle.branch"
        case .certificates: "lock.shield"
        case .proxy: "network"
        case .inspection: "eye"
        case .scripts: "scroll"
        case .daemon: "server.rack"
        case .updates: "arrow.down.circle"
        case .help: "questionmark.circle"
        }
    }

    /// The words of the settings on the page, so a search for a setting
    /// finds the page that holds it. Keep them in step with SettingsView.
    public var keywords: [String] {
        switch self {
        case .general: ["open at login", "start", "login items", "uninstall", "delete"]
        case .routing: ["subdomain fallback", "branch", "allow lan access", "network", "ports", "lan networks", "router", "forget"]
        case .certificates: ["https", "certificate authority", "ca", "trust", "untrust", "ca.pem"]
        case .proxy: ["forward proxy", "address", "port", "chrome", "proxy log", "har", "requests per file", "mb per file", "viewer"]
        case .inspection: ["inspect host", "https", "inspection ca", "trust", "untrust", "decrypt"]
        case .scripts: ["lua", "script rules", "intercept", "log", "secrets", "api keys", "cookies"]
        case .daemon: ["version", "http port", "https port", "config.json", "restart", "errors"]
        case .updates: ["check for updates", "automatically", "version"]
        case .help: ["coding agent", "mcp", "claude", "codex", "command line tool", "terminal", "firefox", "node.js", "python"]
        }
    }

    func matches(_ words: [String]) -> Bool {
        let text = ([title] + keywords).joined(separator: " ").lowercased()
        return words.allSatisfy(text.contains)
    }
}

public struct SettingsNode: Identifiable, Equatable, Sendable {
    public let page: SettingsPage
    public let children: [SettingsNode]
    public var id: SettingsPage { page }

    public init(_ page: SettingsPage, _ children: [SettingsNode] = []) {
        self.page = page
        self.children = children
    }
}

public enum SettingsTree {
    public static let roots: [SettingsNode] = [
        SettingsNode(.general),
        SettingsNode(.routing),
        SettingsNode(.certificates),
        SettingsNode(.proxy, [SettingsNode(.inspection), SettingsNode(.scripts)]),
        SettingsNode(.daemon),
        SettingsNode(.updates),
        SettingsNode(.help),
    ]

    /// The tree for a search. Every word of the query must be in the title or
    /// the keywords of one page. A page that matches keeps all its children;
    /// a page that does not match stays only as the parent of a match.
    public static func filter(_ query: String) -> [SettingsNode] {
        let terms = terms(query)
        if terms.isEmpty { return roots }
        return roots.compactMap { filter($0, terms) }
    }

    private static func terms(_ query: String) -> [String] {
        query.lowercased().split(whereSeparator: \.isWhitespace).map(String.init)
    }

    private static func filter(_ node: SettingsNode, _ words: [String]) -> SettingsNode? {
        if node.page.matches(words) { return node }
        let children = node.children.compactMap { filter($0, words) }
        return children.isEmpty ? nil : SettingsNode(node.page, children)
    }

    /// The page to show when the selected one is no longer in the tree: the
    /// first page that matches the query, in tree order. A parent kept only
    /// for its children is not a match.
    public static func firstPage(in nodes: [SettingsNode], query: String) -> SettingsPage? {
        let terms = terms(query)
        for node in nodes {
            if node.page.matches(terms) { return node.page }
            if let child = firstPage(in: node.children, query: query) { return child }
        }
        return nil
    }

    /// The page whose child this page is, for the title "Proxy › Inspection".
    public static func parent(of page: SettingsPage) -> SettingsPage? {
        roots.first { $0.children.contains { $0.page == page } }?.page
    }

    public static func contains(_ page: SettingsPage, in nodes: [SettingsNode]) -> Bool {
        nodes.contains { $0.page == page || contains(page, in: $0.children) }
    }
}
