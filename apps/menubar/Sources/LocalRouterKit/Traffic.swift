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
        if n < 1_000_000_000 { return String(format: "%.1f MB", Double(n) / 1_000_000) }
        return String(format: "%.2f GB", Double(n) / 1_000_000_000)
    }

    /// The label of the tile that opens Settings › Inspection.
    public static func inspection(hosts: [String]) -> String {
        if hosts.isEmpty { return "Read HTTPS: off" }
        if hosts.contains("*") { return "Read HTTPS: all" }
        return "Read HTTPS: \(hosts.count) \(hosts.count == 1 ? "host" : "hosts")"
    }
}

// MARK: - Traffic by app (ADR 11)

/// The two views of the Traffic tab. The last choice is remembered.
public enum TrafficMode: String, CaseIterable, Identifiable, Sendable {
    case apps = "Apps"
    case requests = "Requests"

    public var id: String { rawValue }

    static let key = "trafficMode"

    /// Apps is the default (ADR 11, 5).
    public static func load(_ defaults: UserDefaults = .standard) -> TrafficMode {
        defaults.string(forKey: key).flatMap(TrafficMode.init(rawValue:)) ?? .apps
    }

    public func save(_ defaults: UserDefaults = .standard) {
        defaults.set(rawValue, forKey: Self.key)
    }
}

/// Whether the daemon can answer `get_app_traffic` (I16). A daemon for API
/// 1.8 does not know the method: the view says so instead of "No traffic".
public enum AppsAvailability: Equatable, Sendable {
    case available
    case tooOld(daemon: String)

    public init(apiVersion: String) {
        let parts = apiVersion.split(separator: ".").compactMap { Int($0) }
        let major = parts.first ?? 0, minor = parts.count > 1 ? parts[1] : 0
        self = (major, minor) >= (1, 9) ? .available : .tooOld(daemon: apiVersion)
    }

    public var message: String? {
        switch self {
        case .available: nil
        case let .tooOld(daemon): "The daemon is too old for the Apps view (it speaks API \(daemon)). Update LocalRouter, or restart the daemon in Settings › Daemon."
        }
    }
}

/// What the Apps view, the map and the menu show, made from one reply.
public enum AppTrafficText {
    /// "↓ 431.0 MB, ↑ 96.5 MB last hour" for the menu.
    public static func summary(_ totals: TrafficTotals, range: TrafficRange = .hour) -> String {
        let span = range == .hour ? "last hour" : "last \(range.label)"
        return "↓ \(TrafficFormat.bytes(totals.bytesIn)), ↑ \(TrafficFormat.bytes(totals.bytesOut)) \(span)"
    }

    /// The line under an app's name: "41 addresses · router, proxy".
    public static func subline(_ app: AppTraffic) -> String {
        var parts: [String] = []
        if app.kind == .exited { parts.append("exited") }
        parts.append(app.addresses == 1 ? "1 address" : "\(app.addresses) addresses")
        let vias = TrafficVia.allCases.filter { app.byVia[$0].total > 0 }.map(\.rawValue)
        if !vias.isEmpty { parts.append(vias.joined(separator: ", ")) }
        return parts.joined(separator: " · ")
    }

    /// The display name: "curl (exited)" for an ended process.
    public static func name(_ app: AppTraffic) -> String {
        app.kind == .exited ? "\(app.name) (exited)" : app.name
    }

    /// The bars of the spark, 0 to 1, the largest at 1.
    public static func spark(_ values: [UInt64]) -> [Double] {
        let top = values.max() ?? 0
        guard top > 0 else { return values.map { _ in 0 } }
        return values.map { Double($0) / Double(top) }
    }

    /// The remotes of one app, most bytes first.
    public static func remotes(of app: String, in result: AppTrafficResult) -> [RemoteTraffic] {
        result.remotes.filter { $0.app == app }
    }

    /// The main label of a remote: the host for router and proxy, else the
    /// network owner, else the address.
    public static func label(_ r: RemoteTraffic) -> String {
        r.host ?? r.asnOrg ?? r.address ?? (r.via == .proxy ? "through the proxy" : "no address")
    }

    /// "203.0.113.4:443 · London, GB".
    public static func detail(_ r: RemoteTraffic) -> String {
        var parts: [String] = []
        if let a = r.address {
            let shown = a.contains(":") ? "[\(a)]" : a
            parts.append(r.port.map { "\(shown):\($0)" } ?? shown)
        }
        if r.host != nil, let owner = r.asnOrg { parts.append(owner) }
        if let p = r.place { parts.append(placeName(p)) }
        return parts.joined(separator: " · ")
    }

    public static func placeName(_ p: Place) -> String {
        [p.city, p.countryCode ?? p.country].compactMap { $0 }.joined(separator: ", ")
    }

    /// Rows of a "Top …" list: a label and its bytes in and out together.
    public struct Top: Equatable, Sendable, Identifiable {
        public var label: String
        public var bytes: UInt64
        public var id: String { label }
    }

    /// The network owners with the most bytes. Router and proxy traffic
    /// count under their host names.
    public static func topOwners(_ remotes: [RemoteTraffic], limit: Int = 5) -> [Top] {
        top(remotes, limit: limit) { $0.asnOrg ?? $0.host }
    }

    public static func topCountries(_ remotes: [RemoteTraffic], limit: Int = 5) -> [Top] {
        top(remotes, limit: limit) { $0.place?.country ?? $0.place?.countryCode }
    }

    public static func topApps(_ apps: [AppTraffic], limit: Int = 5) -> [Top] {
        Array(apps.map { Top(label: name($0), bytes: $0.bytesIn + $0.bytesOut) }
            .filter { $0.bytes > 0 }
            .sorted { $0.bytes > $1.bytes }
            .prefix(limit))
    }

    private static func top(_ remotes: [RemoteTraffic], limit: Int, key: (RemoteTraffic) -> String?) -> [Top] {
        var sums: [String: UInt64] = [:]
        for r in remotes {
            guard let k = key(r) else { continue }
            sums[k, default: 0] += r.bytesIn + r.bytesOut
        }
        return Array(sums.map { Top(label: $0.key, bytes: $0.value) }
            .sorted { ($0.bytes, $1.label) > ($1.bytes, $0.label) }
            .prefix(limit))
    }

    /// One point of the map: a place and what went there.
    public struct MapPlace: Equatable, Sendable, Identifiable {
        public var name: String
        public var lat: Double
        public var lon: Double
        public var addresses: Int
        public var topApp: String
        public var bytesIn: UInt64
        public var bytesOut: UInt64
        public var id: String { "\(name)|\(lat)|\(lon)" }
    }

    /// The remotes with a place, grouped by place, most bytes in first.
    /// `appNames` maps an app key to its display name.
    public static func places(_ remotes: [RemoteTraffic], appNames: [String: String]) -> [MapPlace] {
        struct Acc { var place: Place; var addresses = Set<String>(); var byApp: [String: UInt64] = [:]; var bytesIn: UInt64 = 0; var bytesOut: UInt64 = 0 }
        var groups: [String: Acc] = [:]
        for r in remotes {
            guard let p = r.place else { continue }
            let key = "\(placeName(p))|\(p.lat)|\(p.lon)"
            var acc = groups[key] ?? Acc(place: p)
            if let a = r.address { acc.addresses.insert(a) }
            acc.byApp[r.app, default: 0] += r.bytesIn + r.bytesOut
            acc.bytesIn += r.bytesIn
            acc.bytesOut += r.bytesOut
            groups[key] = acc
        }
        return groups.values.map { acc in
            let top = acc.byApp.max { ($0.value, $1.key) < ($1.value, $0.key) }?.key ?? ""
            return MapPlace(name: placeName(acc.place), lat: acc.place.lat, lon: acc.place.lon, addresses: acc.addresses.count,
                            topApp: appNames[top] ?? top, bytesIn: acc.bytesIn, bytesOut: acc.bytesOut)
        }
        .sorted { ($0.bytesIn, $1.name) > ($1.bytesIn, $0.name) }
    }

    /// Bytes that have no point on the map: router traffic, which stays on
    /// this Mac, and remotes without a place.
    public static func notOnMap(_ remotes: [RemoteTraffic]) -> (router: UInt64, noPlace: UInt64) {
        var router: UInt64 = 0, none: UInt64 = 0
        for r in remotes {
            if r.via == .router { router += r.bytesIn + r.bytesOut } else if r.place == nil { none += r.bytesIn + r.bytesOut }
        }
        return (router, none)
    }

    /// Search in the map window: app, address, owner, host or country.
    public static func matches(_ r: RemoteTraffic, appName: String, query: String) -> Bool {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return true }
        return [appName, r.address, r.asnOrg, r.host, r.place?.country, r.place?.city, r.place?.countryCode]
            .compactMap { $0?.lowercased() }
            .contains { $0.contains(q) }
    }
}

// MARK: - The tree of the Apps view

/// One row under an app in the Apps view: a process, a domain, a connection
/// or a request. Ids are unique in the whole tree.
public struct TrafficNode: Identifiable, Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        /// One executable of an app with several (macOS services, helpers).
        case process
        /// A host, a network owner, or "IP Addresses".
        case group
        /// One remote address and port.
        case address
        /// One request of the request log to the group's host.
        case request
    }

    public var id: String
    public var kind: Kind
    public var title: String
    public var detail: String?
    /// The `via` when every connection under the node has the same one.
    public var via: TrafficVia?
    public var bytesIn: UInt64 = 0
    public var bytesOut: UInt64 = 0
    /// The HTTP status of a request.
    public var status: UInt16?
    /// The executable of a process row, for its icon.
    public var path: String?
    public var children: [TrafficNode] = []
}

/// Builds the rows under an app, as Little Snitch shows them: the processes
/// when there are several, then the domains, then the connections and, for
/// a route's or the proxy's host, the requests the log holds.
public enum TrafficTree {
    /// The name of the group of connections with no host and no owner.
    public static let ipAddresses = "IP Addresses"
    /// Requests listed under one host at most.
    public static let maxRequests = 50

    public static func children(of app: AppTraffic, remotes: [RemoteTraffic], logs: [LogEntry]) -> [TrafficNode] {
        let mine = remotes.filter { $0.app == app.app }
        let exes = Set(mine.map { $0.exe ?? "" })
        guard app.kind == .services || exes.count > 1 else {
            return groups(mine, logs: logs, prefix: app.app)
        }
        var byExe: [String: [RemoteTraffic]] = [:]
        for r in mine { byExe[r.exe ?? "", default: []].append(r) }
        return sorted(byExe.map { exe, list in
            let id = "\(app.app)|p|\(exe)"
            var node = TrafficNode(id: id, kind: .process,
                                   title: exe.isEmpty ? "unknown process" : URL(fileURLWithPath: exe).lastPathComponent,
                                   detail: nil, via: commonVia(list), path: exe.isEmpty ? nil : exe,
                                   children: groups(list, logs: logs, prefix: id))
            add(list, to: &node)
            return node
        })
    }

    /// Connections grouped by host, else owner, else the network part of
    /// the server name, else "IP Addresses".
    static func groups(_ remotes: [RemoteTraffic], logs: [LogEntry], prefix: String) -> [TrafficNode] {
        var byKey: [String: [RemoteTraffic]] = [:]
        for r in remotes { byKey[groupKey(r), default: []].append(r) }
        return sorted(byKey.map { key, list in
            let id = "\(prefix)|g|\(key)"
            let addresses = list.map(address(prefix: id)).sorted { ($0.bytesIn + $0.bytesOut) > ($1.bytesIn + $1.bytesOut) }
            let host = list.first { $0.host != nil }?.host
            let requests = host.map { requests(to: $0, logs: logs, prefix: id) } ?? []
            let count = Set(list.compactMap(\.address)).count
            var parts = [count == 1 ? "1 address" : "\(count) addresses"]
            if !requests.isEmpty { parts.append(requests.count == 1 ? "1 request" : "\(requests.count) requests") }
            var node = TrafficNode(id: id, kind: .group, title: key, detail: parts.joined(separator: " · "), via: commonVia(list),
                                   children: addresses + requests)
            add(list, to: &node)
            return node
        })
    }

    public static func groupKey(_ r: RemoteTraffic) -> String {
        r.host ?? r.asnOrg ?? r.serverName.map(baseDomain) ?? ipAddresses
    }

    /// The network part of a server name: the last two labels, or three
    /// after a short second-level label (`co.uk`).
    /// `165.66.149.34.bc.googleusercontent.com` gives `googleusercontent.com`.
    public static func baseDomain(_ name: String) -> String {
        let labels = name.split(separator: ".", omittingEmptySubsequences: false)
        guard labels.count > 2 else { return name }
        let short = labels[labels.count - 2].count <= 3 && labels[labels.count - 1].count == 2
        return labels.suffix(short ? 3 : 2).joined(separator: ".")
    }

    static func address(prefix: String) -> (RemoteTraffic) -> TrafficNode {
        { r in
            var title = r.address.map { $0.contains(":") ? "[\($0)]" : $0 } ?? "no address"
            if let port = r.port { title += ":\(port)" }
            var detail = [r.proto.rawValue]
            if let name = r.serverName { detail.append(name) }
            if r.host != nil, let owner = r.asnOrg { detail.append(owner) }
            if let place = r.place { detail.append(AppTrafficText.placeName(place)) }
            return TrafficNode(id: "\(prefix)|a|\(title)|\(r.via.rawValue)|\(r.proto.rawValue)", kind: .address, title: title,
                               detail: detail.joined(separator: " · "), via: r.via, bytesIn: r.bytesIn, bytesOut: r.bytesOut)
        }
    }

    /// The log's requests to `host`, newest first. The log writes the port
    /// with the host for some requests (`github.com:443`).
    static func requests(to host: String, logs: [LogEntry], prefix: String) -> [TrafficNode] {
        var out: [TrafficNode] = []
        for (i, entry) in logs.sorted(by: { time($0) > time($1) }).enumerated() {
            guard case let .http(time, method, logHost, path, status, duration, _, proxy, _) = entry, bare(logHost) == host else {
                continue
            }
            let via: TrafficVia = proxy == nil ? .router : .proxy
            out.append(TrafficNode(id: "\(prefix)|r|\(time)|\(i)", kind: .request, title: "\(method) \(path)",
                                   detail: "\(status) · \(TrafficFormat.duration(ms: duration))", via: via, status: status))
            if out.count == maxRequests { break }
        }
        return out
    }

    private static func time(_ entry: LogEntry) -> UInt64 {
        switch entry {
        case let .http(t, _, _, _, _, _, _, _, _), let .tcp(t, _, _, _, _, _, _): t
        }
    }

    private static func bare(_ host: String) -> String {
        if host.hasPrefix("["), let end = host.firstIndex(of: "]") { return String(host[host.index(after: host.startIndex)..<end]) }
        if host.filter({ $0 == ":" }).count == 1, let colon = host.firstIndex(of: ":") { return String(host[..<colon]) }
        return host
    }

    private static func commonVia(_ list: [RemoteTraffic]) -> TrafficVia? {
        let vias = Set(list.map(\.via))
        return vias.count == 1 ? vias.first : nil
    }

    private static func add(_ list: [RemoteTraffic], to node: inout TrafficNode) {
        node.bytesIn = list.reduce(0) { $0 + $1.bytesIn }
        node.bytesOut = list.reduce(0) { $0 + $1.bytesOut }
    }

    private static func sorted(_ nodes: [TrafficNode]) -> [TrafficNode] {
        nodes.sorted { ($0.bytesIn + $0.bytesOut, $1.title) > ($1.bytesIn + $1.bytesOut, $0.title) }
    }
}
