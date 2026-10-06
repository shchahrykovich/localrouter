import Foundation

extension Route {
    /// The project host: the last label of the host key. `feat-login.shop`
    /// and `api.shop` belong to the project `shop` (see docs/dictionary.md).
    public var project: String {
        host.split(separator: ".").last.map(String.init) ?? host
    }
}

/// The routes of one project, as the Domains tab shows them.
public struct RouteGroup: Equatable, Sendable, Identifiable {
    public var project: String
    public var routes: [RouteView]

    public var id: String { project }

    /// Groups by project, sorted by project name. Inside a group the routes
    /// keep the order the daemon sent.
    public static func group(_ routes: [RouteView]) -> [RouteGroup] {
        var order: [String] = []
        var byProject: [String: [RouteView]] = [:]
        for view in routes {
            let project = view.route.project
            if byProject[project] == nil { order.append(project) }
            byProject[project, default: []].append(view)
        }
        return order.sorted().map { RouteGroup(project: $0, routes: byProject[$0]!) }
    }
}

/// The projects whose routes the Domains tab hides. Saved in `UserDefaults`
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
