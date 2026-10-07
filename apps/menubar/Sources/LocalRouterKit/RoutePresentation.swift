import Foundation

/// A small label after a route's name in the Routes tab.
public enum RouteBadge: String, Equatable, Sendable {
    case tcp = "TCP"
    case folder
    case stripPath = "strip path"
    case owned
    case session
}

/// The dot before a route's name.
public enum RouteHealth: Equatable, Sendable {
    /// The target answered the daemon's last check: green.
    case online
    /// The target did not answer: a grey ring.
    case down
    /// Another program holds the TCP route's listen port: orange.
    case listenFailed
    /// Not checked yet: a faint dot.
    case unknown
}

extension Route {
    /// A folder route (ADR 05) serves `file://<folder>` with no dev server.
    public var isFolder: Bool { target.hasPrefix("file://") }

    /// What follows the host key in the name: `.localhost`, and the listen
    /// port of a TCP route, which is chosen by port only.
    public var nameSuffix: String {
        guard `protocol` == .tcp, let port = listenPort else { return ".localhost" }
        return ".localhost:\(port)"
    }

    /// The target as the Routes tab writes it after the arrow: no `http://`
    /// or `tcp://`, and a folder as a path with the home folder as `~`.
    public func displayTarget(home: String = NSHomeDirectory()) -> String {
        if isFolder {
            let path = URL(string: target)?.path ?? String(target.dropFirst("file://".count))
            if path == home { return "~" }
            if path.hasPrefix(home + "/") { return "~" + path.dropFirst(home.count) }
            return path
        }
        for scheme in ["http://", "tcp://"] where target.hasPrefix(scheme) {
            return String(target.dropFirst(scheme.count))
        }
        return target
    }

    /// The kind first (TCP or folder), then strip path, then the lifetime
    /// when it is not persistent (docs/dictionary.md).
    public var badges: [RouteBadge] {
        var out: [RouteBadge] = []
        if `protocol` == .tcp { out.append(.tcp) } else if isFolder { out.append(.folder) }
        if stripPath { out.append(.stripPath) }
        if ownerPid != nil { out.append(.owned) } else if !persistent { out.append(.session) }
        return out
    }
}

extension RouteView {
    public var health: RouteHealth {
        if listenFailed { return .listenFailed }
        switch upstreamUp {
        case true: return .online
        case false: return .down
        default: return .unknown
        }
    }

    /// Said after the target when the dot is not green for a known reason.
    public var healthNote: String? {
        switch health {
        case .down: "not answering"
        case .listenFailed: "listen port is taken"
        case .online, .unknown: nil
        }
    }

    /// The URL that Open and Copy use.
    public var primaryURL: String { urls.first ?? route.fullNameAndPath }
}

extension RouteGroup {
    public var onlineCount: Int { routes.filter(\.online).count }

    /// "4 of 5 online", on the project's header.
    public var summary: String { "\(onlineCount) of \(routes.count) online" }
}

/// The line under the app name in the window header.
public enum StatusLine {
    public static func text(running: Bool, online: Int, total: Int) -> String {
        guard running else { return "Not running" }
        guard total > 0 else { return "Running · no routes" }
        return "Running · \(online) of \(total) \(total == 1 ? "route" : "routes") online"
    }
}
