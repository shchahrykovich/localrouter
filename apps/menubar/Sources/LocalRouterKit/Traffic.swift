import Foundation

/// Which requests the Traffic tab lists: all, those of the routes, or
/// those of the forward proxy (ADR 06).
public enum TrafficScope: String, CaseIterable, Identifiable, Sendable {
    case all = "All"
    case routes = "Routes"
    case proxy = "Proxy"

    public var id: String { rawValue }

    public func includes(_ entry: LogEntry) -> Bool {
        switch self {
        case .all: true
        case .routes: !entry.isProxied
        case .proxy: entry.isProxied
        }
    }
}

extension LogEntry {
    /// In the scope, and the host holds the filter text (case and outer
    /// spaces ignored). An empty filter matches every host.
    public func matches(filter: String, scope: TrafficScope) -> Bool {
        guard scope.includes(self) else { return false }
        let f = filter.trimmingCharacters(in: .whitespaces).lowercased()
        return f.isEmpty || host.lowercased().contains(f)
    }
}

/// The color of an HTTP status in the Traffic tab.
public enum StatusKind: Equatable, Sendable {
    case success, neutral, clientError, serverError

    public init(status: UInt16) {
        switch status {
        case 200..<300: self = .success
        case 400..<500: self = .clientError
        case 500...: self = .serverError
        default: self = .neutral
        }
    }
}

/// Short numbers for the Traffic and Proxy tabs. Decimal units, as Finder
/// shows them, and a `.` as decimal point whatever the locale, so the
/// monospaced columns line up.
public enum TrafficFormat {
    public static func duration(ms: UInt64) -> String {
        if ms < 1000 { return "\(ms) ms" }
        if ms < 10_000 { return String(format: "%.1f s", Double(ms) / 1000) }
        return "\(ms / 1000) s"
    }

    public static func bytes(_ n: UInt64) -> String {
        if n < 1000 { return "\(n) B" }
        if n < 1_000_000 { return String(format: "%.1f KB", Double(n) / 1000) }
        return String(format: "%.1f MB", Double(n) / 1_000_000)
    }

    /// The label of the tile that opens Settings › Inspection.
    public static func inspection(hosts: [String]) -> String {
        if hosts.isEmpty { return "Read HTTPS: off" }
        if hosts.contains("*") { return "Read HTTPS: all" }
        return "Read HTTPS: \(hosts.count) \(hosts.count == 1 ? "host" : "hosts")"
    }
}
