import Foundation

extension Route {
    /// The project host: the last label of the host key. `feat-login.shop`
    /// and `api.shop` belong to the project `shop` (see docs/dictionary.md).
    public var project: String {
        host.split(separator: ".").last.map(String.init) ?? host
    }
}

/// The routes of one project, as the Routes tab shows them.
public struct RouteGroup: Equatable, Sendable, Identifiable {
    public var project: String
    public var routes: [RouteView]

    public var id: String { project }

    /// Has at least one route whose target is up.
    public var online: Bool { routes.contains(where: \.online) }

    /// Groups by project: projects with a route whose target is up first,
    /// then by project name. Inside a group the routes whose target is up
    /// come first; otherwise they keep the order the daemon sent.
    public static func group(_ routes: [RouteView]) -> [RouteGroup] {
        var order: [String] = []
        var byProject: [String: [RouteView]] = [:]
        for view in routes {
            let project = view.route.project
            if byProject[project] == nil { order.append(project) }
            byProject[project, default: []].append(view)
        }
        return order
            .map { project in
                let views = byProject[project]!
                return RouteGroup(project: project, routes: views.filter(\.online) + views.filter { !$0.online })
            }
            .sorted { a, b in a.online != b.online ? a.online : a.project < b.project }
    }
}

extension RouteView {
    /// The target answered the daemon's last check, and the listen port is
    /// bound: the green dot.
    public var online: Bool { upstreamUp == true && !listenFailed }
}

/// The projects whose routes the Routes tab hides. Saved in `UserDefaults`
/// as one string, project names joined by newlines: a project is a host
/// label, so it never holds a newline.
public struct CollapsedProjects: Equatable, Sendable, RawRepresentable {
    public private(set) var projects: Set<String>

    public init(_ projects: Set<String> = []) { self.projects = projects }

    public init?(rawValue: String) {
        projects = Set(rawValue.split(separator: "\n").map(String.init))
    }

    public var rawValue: String { projects.sorted().joined(separator: "\n") }

    public func contains(_ project: String) -> Bool { projects.contains(project) }

    public mutating func toggle(_ project: String) {
        if projects.remove(project) == nil { projects.insert(project) }
    }
}
