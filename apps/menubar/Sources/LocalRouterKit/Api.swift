// Swift copy of the socket API shapes in libs/core/src/api.rs.
//
// JSON keys are snake_case; the coders below convert them. Every file in
// api/examples must decode here and in Rust (invariant I11, test T9).

import Foundation

public let apiVersion = "1.9"

public func apiMajor(_ version: String) -> Int? {
    version.split(separator: ".").first.flatMap { Int($0) }
}

public enum Api {
    public static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    public static let encoder: JSONEncoder = {
        let e = JSONEncoder()
        e.keyEncodingStrategy = .convertToSnakeCase
        e.outputFormatting = [.sortedKeys]
        return e
    }()
}

// MARK: - Envelope

public struct ApiError: Codable, Error, Equatable, Sendable, LocalizedError {
    public var code: String
    public var message: String
    public var errorDescription: String? { message }
}

// MARK: - Routes

public enum RouteProtocol: String, Codable, Sendable {
    case http
    case tcp
}

public struct Route: Codable, Equatable, Sendable, Identifiable {
    public var host: String
    /// Path prefix of a path route, for example `/blog`; nil for the route
    /// without a path (ADR 03).
    public var path: String?
    public var `protocol`: RouteProtocol
    public var target: String
    public var listenPort: UInt16?
    public var httpsOnly: Bool
    public var stripPath: Bool
    public var note: String
    public var ownerPid: UInt32?
    public var persistent: Bool

    /// The route key: host plus path, for example `shop/blog`. Two routes of
    /// one host have different ids.
    public var id: String { host + (path ?? "") }

    public init(host: String, path: String? = nil, protocol: RouteProtocol = .http, target: String, listenPort: UInt16? = nil,
                httpsOnly: Bool = false, stripPath: Bool = false, note: String = "", ownerPid: UInt32? = nil,
                persistent: Bool = false) {
        self.host = host
        self.path = path
        self.protocol = `protocol`
        self.target = target
        self.listenPort = listenPort
        self.httpsOnly = httpsOnly
        self.stripPath = stripPath
        self.note = note
        self.ownerPid = ownerPid
        self.persistent = persistent
    }

    enum CodingKeys: String, CodingKey {
        case host, path, `protocol`, target, listenPort, httpsOnly, stripPath, note, ownerPid, persistent
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        host = try c.decode(String.self, forKey: .host)
        path = try c.decodeIfPresent(String.self, forKey: .path)
        self.protocol = try c.decodeIfPresent(RouteProtocol.self, forKey: .protocol) ?? .http
        target = try c.decode(String.self, forKey: .target)
        listenPort = try c.decodeIfPresent(UInt16.self, forKey: .listenPort)
        httpsOnly = try c.decodeIfPresent(Bool.self, forKey: .httpsOnly) ?? false
        stripPath = try c.decodeIfPresent(Bool.self, forKey: .stripPath) ?? false
        note = try c.decodeIfPresent(String.self, forKey: .note) ?? ""
        ownerPid = try c.decodeIfPresent(UInt32.self, forKey: .ownerPid)
        persistent = try c.decodeIfPresent(Bool.self, forKey: .persistent) ?? false
    }

    // Same omissions as the Rust side, so a round trip gives the same JSON.
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(host, forKey: .host)
        try c.encodeIfPresent(path, forKey: .path)
        try c.encode(self.protocol, forKey: .protocol)
        try c.encode(target, forKey: .target)
        try c.encodeIfPresent(listenPort, forKey: .listenPort)
        if httpsOnly { try c.encode(true, forKey: .httpsOnly) }
        if stripPath { try c.encode(true, forKey: .stripPath) }
        if !note.isEmpty { try c.encode(note, forKey: .note) }
        try c.encodeIfPresent(ownerPid, forKey: .ownerPid)
        if persistent { try c.encode(true, forKey: .persistent) }
    }

    public var fullName: String { "\(host).localhost" }

    /// `shop.localhost/blog`, or `shop.localhost` for the route without a path.
    public var fullNameAndPath: String { fullName + (path ?? "") }
}

/// A route as clients see it. The route's fields are flattened into the same object.
public struct RouteView: Codable, Equatable, Sendable, Identifiable {
    public var route: Route
    public var urls: [String]
    public var upstreamUp: Bool?
    public var listenFailed: Bool

    public var id: String { route.id }

    enum CodingKeys: String, CodingKey { case urls, upstreamUp, listenFailed }

    public init(from decoder: Decoder) throws {
        route = try Route(from: decoder)
        let c = try decoder.container(keyedBy: CodingKeys.self)
        urls = try c.decode([String].self, forKey: .urls)
        upstreamUp = try c.decodeIfPresent(Bool.self, forKey: .upstreamUp)
        listenFailed = try c.decodeIfPresent(Bool.self, forKey: .listenFailed) ?? false
    }

    public func encode(to encoder: Encoder) throws {
        try route.encode(to: encoder)
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(urls, forKey: .urls)
        try c.encodeIfPresent(upstreamUp, forKey: .upstreamUp)
        if listenFailed { try c.encode(true, forKey: .listenFailed) }
    }
}

// MARK: - Method parameters and results

public struct Empty: Codable, Equatable, Sendable { public init() {} }

public struct HelloParams: Codable, Equatable, Sendable {
    public var client: String
    public var apiVersion: String
}

public struct HelloResult: Codable, Equatable, Sendable {
    public var apiVersion: String
    public var daemonVersion: String
}

public struct PortStatus: Codable, Equatable, Sendable {
    public var configured: UInt16
    public var port: UInt16?
    public var bound: [String]
    public var errors: [String]
}

public struct CaStatus: Codable, Equatable, Sendable {
    public var state: String
    public var problem: String?
    public var pemPath: String
    public var commonName: String?
    public var trusted: Bool?
}

public struct StatusResult: Codable, Equatable, Sendable {
    public var daemonVersion: String
    public var apiVersion: String
    public var pid: UInt32
    public var dataDir: String
    public var http: PortStatus
    public var https: PortStatus
    public var ca: CaStatus
    public var routes: Int
    public var routesFileProblem: String?
    public var listenFailed: [String]
    /// The forward proxy (ADR 06). Nil from daemons before API 1.3.
    public var proxy: ProxyStatus?
    /// The network LAN access sees (ADR 08). Nil when it cannot be
    /// recognised, and from daemons before API 1.5.
    public var network: NetworkStatus?
    /// Plain sentences for the user (ADR 08). Nil when there are none.
    public var notes: [String]?
    /// The traffic sampler (ADR 11). Nil from daemons before API 1.9.
    public var traffic: SamplerState?
    /// Why the sampler is restarting.
    public var trafficError: String?
    /// Bytes of traffic history on disk; nil when there is none.
    public var trafficHistoryBytes: UInt64?
    /// The places databases, when they are on this Mac.
    public var geoDb: GeoDbInfo?
}

/// The network the Mac is on, known by its router's MAC address (ADR 08).
public struct NetworkStatus: Codable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var router: String
    public var interface: String
    public var lanAllowed: Bool
}

/// One network where LAN access is allowed (ADR 08).
public struct LanNetwork: Codable, Equatable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var router: String
    public init(id: String, name: String, router: String) {
        self.id = id
        self.name = name
        self.router = router
    }
}

/// The forward proxy port and the inspection CA (ADR 06).
public struct ProxyStatus: Codable, Equatable, Sendable {
    public var enabled: Bool
    public var configured: UInt16
    public var port: UInt16?
    public var bound: [String]
    public var errors: [String]
    /// Nil until the inspection CA exists.
    public var inspectCa: CaStatus?
    /// The proxy clients' ports (ADR 09). Nil when there are none, and from
    /// daemons before API 1.6.
    public var clients: [ProxyClientStatus]?
}

/// One more proxy port, for one client (ADR 09). The log marks each request
/// with the name of the port that carried it.
public struct ProxyClient: Codable, Equatable, Sendable, Identifiable {
    public var name: String
    public var port: UInt16
    /// A phone client: its port listens on the LAN (ADR 10). Nil means false.
    public var lan: Bool?
    /// A phone client whose PAC file sends the phone direct. Nil means false.
    public var paused: Bool?
    public var id: String { name }
    public init(name: String, port: UInt16, lan: Bool? = nil, paused: Bool? = nil) {
        self.name = name
        self.port = port
        self.lan = lan
        self.paused = paused
    }

    enum CodingKeys: String, CodingKey { case name, port, lan, paused }
}

/// One proxy client's port and whether it is bound (ADR 09).
public struct ProxyClientStatus: Codable, Equatable, Sendable {
    public var name: String
    public var configured: UInt16
    public var port: UInt16?
    public var bound: [String]
    public var errors: [String]
    /// A phone client (ADR 10). Nil means false.
    public var lan: Bool?
    /// Devices waiting for Allow or Deny (ADR 10). Nil when none.
    public var pending: [PendingDevice]?
}

/// A device that asked to use a phone client and is not allowed yet.
public struct PendingDevice: Codable, Equatable, Sendable, Identifiable {
    public var address: String
    /// The host it asked for, to help the user recognise it.
    public var host: String
    public var atMs: UInt64
    public var id: String { address }
}

/// What a phone needs to use its proxy client (ADR 10).
public struct LanProxyInfo: Codable, Equatable, Sendable {
    /// This Mac's IPv4 address on the current network; nil when unknown.
    public var address: String?
    public var port: UInt16
    /// What the QR code holds: opening it allows the device. Nil when the
    /// address is unknown.
    public var setupUrl: String?
    /// The URL for Configure Proxy → Automatic (the PAC file).
    public var pacUrl: String?
    public var devices: [String]
    public var pending: [PendingDevice]
    public var problems: [String]
}

public struct NewSetupCodeParams: Codable, Equatable, Sendable {
    public var client: String
    public init(client: String) {
        self.client = client
    }
}

public struct NewSetupCodeResult: Codable, Equatable, Sendable {
    public var setupUrl: String?
}

public struct SetPhoneDeviceParams: Codable, Equatable, Sendable {
    public var client: String
    public var address: String
    public var allow: Bool
    public init(client: String, address: String, allow: Bool) {
        self.client = client
        self.address = address
        self.allow = allow
    }
}

public struct SetPhoneDeviceResult: Codable, Equatable, Sendable {
    public var devices: [String]
}

public struct RegisterRouteResult: Codable, Equatable, Sendable {
    public var route: RouteView
    public var replaced: Bool
    public var oldTarget: String?
}

/// Names one route: host plus the path of a path route. Without a path,
/// `unregister_route` removes only the route without a path (ADR 03).
public struct HostParams: Codable, Equatable, Sendable {
    public var host: String
    public var path: String?
    public init(host: String, path: String? = nil) {
        self.host = host
        self.path = path
    }

    public init(route: Route) { self.init(host: route.host, path: route.path) }
}

public struct UnregisterRouteResult: Codable, Equatable, Sendable {
    public var removed: Bool
}

public struct ListRoutesResult: Codable, Equatable, Sendable {
    public var routes: [RouteView]
}

public struct FindFreePortParams: Codable, Equatable, Sendable {
    public var near: UInt16?
    public init(near: UInt16? = nil) {
        self.near = near
    }
}

public struct FindFreePortResult: Codable, Equatable, Sendable {
    public var port: UInt16
}

public struct GetLogsParams: Codable, Equatable, Sendable {
    public var host: String?
    public var limit: Int?
    public init(host: String? = nil, limit: Int? = nil) {
        self.host = host
        self.limit = limit
    }
}

/// How the forward proxy carried a request (ADR 06): `via: "proxy"` with
/// `mode` `http`, `inspect` or `tunnel`, and bytes for a tunnel.
public struct ProxyTraffic: Equatable, Sendable {
    public var mode: String
    public var bytesIn: UInt64?
    public var bytesOut: UInt64?
    public init(mode: String, bytesIn: UInt64? = nil, bytesOut: UInt64? = nil) {
        self.mode = mode
        self.bytesIn = bytesIn
        self.bytesOut = bytesOut
    }
}

/// The script rules that ran on a request, and the one that failed (ADR 07).
public struct ScriptRun: Equatable, Sendable {
    public var rules: [String]
    public var error: String?
    public init(rules: [String], error: String? = nil) {
        self.rules = rules
        self.error = error
    }
}

public enum LogEntry: Codable, Equatable, Sendable, Identifiable {
    /// `route` is the key of the route that answered, for example `shop/blog`.
    /// `proxy` is set for traffic of the forward proxy (ADR 06), `scripts`
    /// when script rules ran (ADR 07).
    case http(time: UInt64, method: String, host: String, path: String, status: UInt16, durationMs: UInt64, route: String?,
              proxy: ProxyTraffic?, scripts: ScriptRun?)
    case tcp(time: UInt64, host: String, listenPort: UInt16, bytesIn: UInt64, bytesOut: UInt64, durationMs: UInt64, failed: Bool)

    public var id: String {
        switch self {
        case let .http(time, method, host, path, status, _, _, _, _): "\(time)-\(method)-\(host)\(path)-\(status)"
        case let .tcp(time, host, port, bytesIn, _, _, _): "\(time)-\(host):\(port)-\(bytesIn)"
        }
    }

    public var host: String {
        switch self {
        case let .http(_, _, host, _, _, _, _, _, _), let .tcp(_, host, _, _, _, _, _): host
        }
    }

    /// Traffic of the forward proxy (ADR 06), for the Proxy tab.
    public var isProxied: Bool {
        if case let .http(_, _, _, _, _, _, _, proxy, _) = self { return proxy != nil }
        return false
    }

    enum CodingKeys: String, CodingKey {
        case kind, timeMs, method, host, path, status, durationMs, route, listenPort, bytesIn, bytesOut, failed, via, mode
        case rules, scriptError
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let time = try c.decode(UInt64.self, forKey: .timeMs)
        let host = try c.decode(String.self, forKey: .host)
        let duration = try c.decode(UInt64.self, forKey: .durationMs)
        switch try c.decode(String.self, forKey: .kind) {
        case "http":
            var proxy: ProxyTraffic?
            if try c.decodeIfPresent(String.self, forKey: .via) == "proxy" {
                proxy = ProxyTraffic(mode: try c.decodeIfPresent(String.self, forKey: .mode) ?? "",
                                     bytesIn: try c.decodeIfPresent(UInt64.self, forKey: .bytesIn),
                                     bytesOut: try c.decodeIfPresent(UInt64.self, forKey: .bytesOut))
            }
            let rules = try c.decodeIfPresent([String].self, forKey: .rules) ?? []
            let failed = try c.decodeIfPresent(String.self, forKey: .scriptError)
            let scripts = rules.isEmpty && failed == nil ? nil : ScriptRun(rules: rules, error: failed)
            self = .http(time: time, method: try c.decode(String.self, forKey: .method), host: host,
                         path: try c.decode(String.self, forKey: .path), status: try c.decode(UInt16.self, forKey: .status),
                         durationMs: duration, route: try c.decodeIfPresent(String.self, forKey: .route), proxy: proxy,
                         scripts: scripts)
        case "tcp":
            self = .tcp(time: time, host: host, listenPort: try c.decode(UInt16.self, forKey: .listenPort),
                        bytesIn: try c.decode(UInt64.self, forKey: .bytesIn), bytesOut: try c.decode(UInt64.self, forKey: .bytesOut),
                        durationMs: duration, failed: try c.decode(Bool.self, forKey: .failed))
        case let other:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "unknown log kind \(other)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .http(time, method, host, path, status, duration, route, proxy, scripts):
            try c.encode("http", forKey: .kind)
            try c.encode(time, forKey: .timeMs)
            try c.encode(method, forKey: .method)
            try c.encode(host, forKey: .host)
            try c.encode(path, forKey: .path)
            try c.encode(status, forKey: .status)
            try c.encode(duration, forKey: .durationMs)
            try c.encodeIfPresent(route, forKey: .route)
            if let proxy {
                try c.encode("proxy", forKey: .via)
                try c.encode(proxy.mode, forKey: .mode)
                try c.encodeIfPresent(proxy.bytesIn, forKey: .bytesIn)
                try c.encodeIfPresent(proxy.bytesOut, forKey: .bytesOut)
            }
            if let scripts {
                if !scripts.rules.isEmpty { try c.encode(scripts.rules, forKey: .rules) }
                try c.encodeIfPresent(scripts.error, forKey: .scriptError)
            }
        case let .tcp(time, host, port, bytesIn, bytesOut, duration, failed):
            try c.encode("tcp", forKey: .kind)
            try c.encode(time, forKey: .timeMs)
            try c.encode(host, forKey: .host)
            try c.encode(port, forKey: .listenPort)
            try c.encode(bytesIn, forKey: .bytesIn)
            try c.encode(bytesOut, forKey: .bytesOut)
            try c.encode(duration, forKey: .durationMs)
            try c.encode(failed, forKey: .failed)
        }
    }
}

public struct GetLogsResult: Codable, Equatable, Sendable {
    public var entries: [LogEntry]
}

public struct SubscribeLogsParams: Codable, Equatable, Sendable {
    public var host: String?
}

public struct SubscribeLogsResult: Codable, Equatable, Sendable {
    public var subscribed: Bool
}

public struct LogEvent: Codable, Equatable, Sendable {
    public var event: String
    public var entry: LogEntry
}

public struct Config: Codable, Equatable, Sendable {
    public var version: UInt32
    public var httpPort: UInt16
    public var httpsPort: UInt16
    public var fallback: Bool
    public var allowLan: Bool
    public var logSize: Int
    /// The forward proxy (ADR 06). Optional: a daemon before API 1.3 has none.
    public var proxyEnabled: Bool?
    public var proxyPort: UInt16?
    public var inspectHosts: [String]?
    /// The proxy log (ADR 08). Optional: a daemon before API 1.5 has none.
    public var proxyLog: Bool?
    public var proxyLogFileMb: UInt64?
    public var proxyLogFileRequests: UInt64?
    /// The networks where LAN access applies (ADR 08).
    public var lanNetworks: [LanNetwork]?
    /// More proxy ports, one per client (ADR 09). Nil from a daemon before 1.6.
    public var proxyClients: [ProxyClient]?
    /// Traffic by app (ADR 11). Left out of the file while unset: nil means
    /// off, 7 days, no places.
    public var traffic: Bool?
    public var trafficHistoryDays: UInt32?
    public var trafficPlaces: Bool?
    public var trafficNames: Bool?
}

public struct SetConfigParams: Codable, Equatable, Sendable {
    public var httpPort: UInt16?
    public var httpsPort: UInt16?
    public var fallback: Bool?
    public var allowLan: Bool?
    public var logSize: Int?
    public var proxyEnabled: Bool?
    public var proxyPort: UInt16?
    /// Replaces the whole list.
    public var inspectHosts: [String]?
    /// The proxy log (ADR 08). The daemon refuses limits out of range.
    public var proxyLog: Bool?
    public var proxyLogFileMb: UInt64?
    public var proxyLogFileRequests: UInt64?
    /// Replaces the whole list (ADR 08).
    public var lanNetworks: [LanNetwork]?
    /// Replaces the whole list of proxy clients (ADR 09).
    public var proxyClients: [ProxyClient]?
    /// Traffic by app (ADR 11).
    public var traffic: Bool?
    /// 1, 7 or 30.
    public var trafficHistoryDays: UInt32?
    public var trafficPlaces: Bool?
    /// Look up server names by reverse DNS.
    public var trafficNames: Bool?
    public init(fallback: Bool? = nil, allowLan: Bool? = nil, proxyEnabled: Bool? = nil, proxyPort: UInt16? = nil,
                inspectHosts: [String]? = nil, proxyLog: Bool? = nil,
                proxyLogFileMb: UInt64? = nil, proxyLogFileRequests: UInt64? = nil, lanNetworks: [LanNetwork]? = nil,
                proxyClients: [ProxyClient]? = nil, traffic: Bool? = nil, trafficHistoryDays: UInt32? = nil,
                trafficPlaces: Bool? = nil, trafficNames: Bool? = nil) {
        self.trafficNames = trafficNames
        self.traffic = traffic
        self.trafficHistoryDays = trafficHistoryDays
        self.trafficPlaces = trafficPlaces
        self.fallback = fallback
        self.allowLan = allowLan
        self.proxyEnabled = proxyEnabled
        self.proxyPort = proxyPort
        self.inspectHosts = inspectHosts
        self.proxyLog = proxyLog
        self.proxyLogFileMb = proxyLogFileMb
        self.proxyLogFileRequests = proxyLogFileRequests
        self.lanNetworks = lanNetworks
        self.proxyClients = proxyClients
    }
}

public struct SetConfigResult: Codable, Equatable, Sendable {
    public var config: Config
    public var restartNeeded: Bool
}

public struct ResetCaResult: Codable, Equatable, Sendable {
    public var pemPath: String
    public var commonName: String
}

/// `get_proxy` for one proxy client's port (ADR 09); nil for the main port.
public struct GetProxyParams: Codable, Equatable, Sendable {
    public var client: String?
    public init(client: String? = nil) {
        self.client = client
    }
}

/// Everything a client needs to use the forward proxy (ADR 06).
public struct GetProxyResult: Codable, Equatable, Sendable {
    public var enabled: Bool
    /// The proxy client this reply is for (ADR 09); nil for the main port.
    public var client: String?
    public var url: String
    public var port: UInt16
    public var bound: [String]
    public var errors: [String]
    public var inspectHosts: [String]
    public var inspectSet: [String]
    public var inspectCa: CaStatus?
    /// Keys are variable names such as `HTTPS_PROXY`; JSONDecoder leaves
    /// dictionary keys as they are.
    public var env: [String: String]
    /// The arguments every client gives Chrome; none builds its own (I18).
    public var chromeArgs: [String]
    public var notes: [String]
    /// Every script rule with its state (ADR 07). Empty from older daemons.
    public var scriptRules: [ScriptRuleView]
    /// The HAR log of proxy traffic (ADR 08). Nil from a 1.4 daemon: the app
    /// then hides the log controls.
    public var log: ProxyLogStatus?
    /// Every proxy client's port (ADR 09). Nil when there are none.
    public var clients: [ProxyClientStatus]?
    /// For a phone client (ADR 10); nil otherwise.
    public var lan: LanProxyInfo?

    enum CodingKeys: String, CodingKey {
        case enabled, client, url, port, bound, errors, inspectHosts, inspectSet, inspectCa, env, chromeArgs, notes, scriptRules, log,
             clients, lan
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        enabled = try c.decode(Bool.self, forKey: .enabled)
        client = try c.decodeIfPresent(String.self, forKey: .client)
        url = try c.decode(String.self, forKey: .url)
        port = try c.decode(UInt16.self, forKey: .port)
        bound = try c.decode([String].self, forKey: .bound)
        errors = try c.decode([String].self, forKey: .errors)
        inspectHosts = try c.decode([String].self, forKey: .inspectHosts)
        inspectSet = try c.decode([String].self, forKey: .inspectSet)
        inspectCa = try c.decodeIfPresent(CaStatus.self, forKey: .inspectCa)
        env = try c.decode([String: String].self, forKey: .env)
        chromeArgs = try c.decode([String].self, forKey: .chromeArgs)
        notes = try c.decode([String].self, forKey: .notes)
        scriptRules = try c.decodeIfPresent([ScriptRuleView].self, forKey: .scriptRules) ?? []
        log = try c.decodeIfPresent(ProxyLogStatus.self, forKey: .log)
        clients = try c.decodeIfPresent([ProxyClientStatus].self, forKey: .clients)
        lan = try c.decodeIfPresent(LanProxyInfo.self, forKey: .lan)
    }
}

/// The proxy log (ADR 08): where the HAR files are, the viewer, the limits.
public struct ProxyLogStatus: Codable, Equatable, Sendable {
    public var enabled: Bool
    public var folder: String
    public var url: String
    public var fileMb: UInt64
    public var fileRequests: UInt64
    public var keepFiles: Int
    public var current: String?
    public var files: Int
    public var written: UInt64
    public var dropped: UInt64
    public var error: String?
}

// MARK: - Script rules (ADR 07)

/// A script rule: which `.lua` file runs on which traffic. Its kind
/// (intercept or log) comes from the script.
public struct ScriptRule: Codable, Equatable, Sendable, Identifiable {
    public var id: String
    public var host: String
    public var path: String?
    public var methods: [String]
    public var script: String
    public var outputDir: String?
    public var order: Int64
    /// `fail` or `pass`; nil means `fail`.
    public var onError: String?
    public var maxCaptureBytes: UInt64?
    public var enabled: Bool
    public var note: String
    public var ownerPid: UInt32?
    public var persistent: Bool

    enum CodingKeys: String, CodingKey {
        case id, host, path, methods, script, outputDir, order, onError, maxCaptureBytes, enabled, note, ownerPid,
             persistent
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        host = try c.decode(String.self, forKey: .host)
        path = try c.decodeIfPresent(String.self, forKey: .path)
        methods = try c.decodeIfPresent([String].self, forKey: .methods) ?? []
        script = try c.decode(String.self, forKey: .script)
        outputDir = try c.decodeIfPresent(String.self, forKey: .outputDir)
        order = try c.decodeIfPresent(Int64.self, forKey: .order) ?? 100
        onError = try c.decodeIfPresent(String.self, forKey: .onError)
        maxCaptureBytes = try c.decodeIfPresent(UInt64.self, forKey: .maxCaptureBytes)
        enabled = try c.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        note = try c.decodeIfPresent(String.self, forKey: .note) ?? ""
        ownerPid = try c.decodeIfPresent(UInt32.self, forKey: .ownerPid)
        persistent = try c.decodeIfPresent(Bool.self, forKey: .persistent) ?? false
    }

    // Same omissions as the Rust side, so a round trip gives the same JSON.
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(host, forKey: .host)
        try c.encodeIfPresent(path, forKey: .path)
        if !methods.isEmpty { try c.encode(methods, forKey: .methods) }
        try c.encode(script, forKey: .script)
        try c.encodeIfPresent(outputDir, forKey: .outputDir)
        try c.encode(order, forKey: .order)
        try c.encodeIfPresent(onError, forKey: .onError)
        try c.encodeIfPresent(maxCaptureBytes, forKey: .maxCaptureBytes)
        try c.encode(enabled, forKey: .enabled)
        if !note.isEmpty { try c.encode(note, forKey: .note) }
        try c.encodeIfPresent(ownerPid, forKey: .ownerPid)
        if persistent { try c.encode(true, forKey: .persistent) }
    }

    /// `shop.localhost/v1`, or the host alone.
    public var target: String { host + (path ?? "") }
}

/// The time and message of a rule's last failure.
public struct LastError: Codable, Equatable, Sendable {
    public var message: String
    public var timeMs: UInt64
}

/// A script rule and its state. The rule's fields are flattened into the same object.
public struct ScriptRuleView: Codable, Equatable, Sendable, Identifiable {
    public var rule: ScriptRule
    /// `intercept` or `log`; nil when the script never loaded.
    public var kind: String?
    public var matched: UInt64
    public var answered: UInt64
    public var errors: UInt64
    public var lastError: LastError?
    public var dropped: UInt64
    public var bytesWritten: UInt64
    public var scriptLoadedAt: UInt64?
    public var scriptSha256: String?

    public var id: String { rule.id }

    enum CodingKeys: String, CodingKey {
        case kind, matched, answered, errors, lastError, dropped, bytesWritten, scriptLoadedAt, scriptSha256
    }

    public init(from decoder: Decoder) throws {
        rule = try ScriptRule(from: decoder)
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decodeIfPresent(String.self, forKey: .kind)
        matched = try c.decode(UInt64.self, forKey: .matched)
        answered = try c.decode(UInt64.self, forKey: .answered)
        errors = try c.decode(UInt64.self, forKey: .errors)
        lastError = try c.decodeIfPresent(LastError.self, forKey: .lastError)
        dropped = try c.decode(UInt64.self, forKey: .dropped)
        bytesWritten = try c.decode(UInt64.self, forKey: .bytesWritten)
        scriptLoadedAt = try c.decodeIfPresent(UInt64.self, forKey: .scriptLoadedAt)
        scriptSha256 = try c.decodeIfPresent(String.self, forKey: .scriptSha256)
    }

    public func encode(to encoder: Encoder) throws {
        try rule.encode(to: encoder)
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encodeIfPresent(kind, forKey: .kind)
        try c.encode(matched, forKey: .matched)
        try c.encode(answered, forKey: .answered)
        try c.encode(errors, forKey: .errors)
        try c.encodeIfPresent(lastError, forKey: .lastError)
        try c.encode(dropped, forKey: .dropped)
        try c.encode(bytesWritten, forKey: .bytesWritten)
        try c.encodeIfPresent(scriptLoadedAt, forKey: .scriptLoadedAt)
        try c.encodeIfPresent(scriptSha256, forKey: .scriptSha256)
    }
}

/// Parameters of `set_script_rule`: the rule, and `check_only`.
public struct SetScriptRuleParams: Codable, Equatable, Sendable {
    public var rule: ScriptRule
    public var checkOnly: Bool

    public init(rule: ScriptRule, checkOnly: Bool = false) {
        self.rule = rule
        self.checkOnly = checkOnly
    }

    enum CodingKeys: String, CodingKey { case checkOnly }

    public init(from decoder: Decoder) throws {
        rule = try ScriptRule(from: decoder)
        checkOnly = try decoder.container(keyedBy: CodingKeys.self).decodeIfPresent(Bool.self, forKey: .checkOnly) ?? false
    }

    public func encode(to encoder: Encoder) throws {
        try rule.encode(to: encoder)
        var c = encoder.container(keyedBy: CodingKeys.self)
        if checkOnly { try c.encode(true, forKey: .checkOnly) }
    }
}

public struct InspectChange: Codable, Equatable, Sendable {
    public var hostAdded: Bool
    public var caCreated: Bool
    public var caTrusted: Bool?
}

public struct SetScriptRuleResult: Codable, Equatable, Sendable {
    public var rule: ScriptRuleView
    public var replaced: Bool
    public var checkOnly: Bool
    public var inspect: InspectChange?
    public var notes: [String]

    enum CodingKeys: String, CodingKey { case rule, replaced, checkOnly, inspect, notes }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        rule = try c.decode(ScriptRuleView.self, forKey: .rule)
        replaced = try c.decode(Bool.self, forKey: .replaced)
        checkOnly = try c.decodeIfPresent(Bool.self, forKey: .checkOnly) ?? false
        inspect = try c.decodeIfPresent(InspectChange.self, forKey: .inspect)
        notes = try c.decodeIfPresent([String].self, forKey: .notes) ?? []
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(rule, forKey: .rule)
        try c.encode(replaced, forKey: .replaced)
        if checkOnly { try c.encode(true, forKey: .checkOnly) }
        try c.encodeIfPresent(inspect, forKey: .inspect)
        try c.encode(notes, forKey: .notes)
    }
}

public struct IdParams: Codable, Equatable, Sendable {
    public var id: String
    public init(id: String) { self.id = id }
}

public struct RemoveScriptRuleResult: Codable, Equatable, Sendable {
    public var removed: Bool
}

public struct ListScriptRulesResult: Codable, Equatable, Sendable {
    public var rules: [ScriptRuleView]
}

// MARK: - Traffic by app (ADR 11)

/// The time a `get_app_traffic` reply covers.
public enum TrafficRange: String, Codable, CaseIterable, Identifiable, Sendable {
    case hour = "1h"
    case day = "24h"
    case week = "7d"
    case month = "30d"

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .hour: "1 hour"
        case .day: "24 hours"
        case .week: "7 days"
        case .month: "30 days"
        }
    }
}

public enum SamplerState: String, Codable, Sendable {
    case off, running, restarting
}

/// How an app's connection reached the network: through a `.localhost`
/// route, through the LocalRouter proxy, or neither.
public enum TrafficVia: String, Codable, CaseIterable, Sendable {
    case router, proxy, direct
}

public enum TrafficProto: String, Codable, Sendable {
    case tcp, udp
}

public enum AppKind: String, Codable, Sendable {
    /// An `.app` bundle.
    case app
    /// An executable outside a bundle.
    case program
    /// The programs of macOS itself, in one row.
    case services
    /// The process ended before its path was read.
    case exited
}

public struct GetAppTrafficParams: Codable, Equatable, Sendable {
    public var range: TrafficRange
    public var app: String?
    public init(range: TrafficRange = .hour, app: String? = nil) {
        self.range = range
        self.app = app
    }
}

public struct InOut: Codable, Equatable, Sendable {
    public var bytesIn: UInt64
    public var bytesOut: UInt64
    enum CodingKeys: String, CodingKey {
        case bytesIn = "in", bytesOut = "out"
    }
    public init(bytesIn: UInt64 = 0, bytesOut: UInt64 = 0) {
        self.bytesIn = bytesIn
        self.bytesOut = bytesOut
    }
    public var total: UInt64 { bytesIn + bytesOut }
}

public struct ByVia: Codable, Equatable, Sendable {
    public var router: InOut
    public var proxy: InOut
    public var direct: InOut
    public init(router: InOut = InOut(), proxy: InOut = InOut(), direct: InOut = InOut()) {
        self.router = router
        self.proxy = proxy
        self.direct = direct
    }
    public subscript(via: TrafficVia) -> InOut {
        switch via {
        case .router: router
        case .proxy: proxy
        case .direct: direct
        }
    }
}

public struct TrafficBucket: Codable, Equatable, Sendable {
    /// Unix seconds of the bucket's start.
    public var t: UInt64
    public var bytesIn: UInt64
    public var bytesOut: UInt64
    enum CodingKeys: String, CodingKey {
        case t, bytesIn = "in", bytesOut = "out"
    }
}

/// One row of the Apps view.
public struct AppTraffic: Codable, Equatable, Sendable, Identifiable {
    /// The app path, the executable path, `macOS services`, or `exited:<name>`.
    public var app: String
    public var name: String
    public var kind: AppKind
    /// The executables seen under the app, for the app checks.
    public var paths: [String]
    /// Its processes seen in the last 30 seconds.
    public var pids: [UInt32]
    public var bytesIn: UInt64
    public var bytesOut: UInt64
    public var byVia: ByVia
    /// Different remote addresses.
    public var addresses: Int
    /// In and out together, in 12 parts of the range, oldest first.
    public var spark: [UInt64]

    public var id: String { app }

    enum CodingKeys: String, CodingKey {
        case app, name, kind, paths, pids, bytesIn = "in", bytesOut = "out", byVia, addresses, spark
    }
}

/// Where an address is, from the DB-IP database on this Mac. A guess.
public struct Place: Codable, Equatable, Sendable {
    public var city: String?
    public var country: String?
    public var countryCode: String?
    public var lat: Double
    public var lon: Double
}

public struct RemoteTraffic: Codable, Equatable, Sendable {
    public var app: String
    /// The executable that made the connection.
    public var exe: String?
    public var address: String?
    public var port: UInt16?
    public var proto: TrafficProto
    /// The route's name for `router`, the host for `proxy`; nil for `direct`.
    public var host: String?
    /// The network owner: `GitHub, Inc.`.
    public var asnOrg: String?
    /// How the server calls itself, by reverse DNS: not the domain the app
    /// asked for. Only while `traffic_names` is on.
    public var serverName: String?
    public var place: Place?
    public var via: TrafficVia
    public var bytesIn: UInt64
    public var bytesOut: UInt64

    public init(app: String, exe: String? = nil, address: String?, port: UInt16?, proto: TrafficProto, host: String? = nil,
                asnOrg: String? = nil, serverName: String? = nil, place: Place? = nil, via: TrafficVia, bytesIn: UInt64,
                bytesOut: UInt64) {
        self.serverName = serverName
        self.app = app
        self.exe = exe
        self.address = address
        self.port = port
        self.proto = proto
        self.host = host
        self.asnOrg = asnOrg
        self.place = place
        self.via = via
        self.bytesIn = bytesIn
        self.bytesOut = bytesOut
    }

    enum CodingKeys: String, CodingKey {
        case app, exe, address, port, proto, host, asnOrg, serverName, place, via, bytesIn = "in", bytesOut = "out"
    }
}

public struct TrafficTotals: Codable, Equatable, Sendable {
    public var bytesIn: UInt64
    public var bytesOut: UInt64
    public var byVia: ByVia
    enum CodingKeys: String, CodingKey {
        case bytesIn = "in", bytesOut = "out", byVia
    }
}

public struct AppTrafficResult: Codable, Equatable, Sendable {
    public var range: TrafficRange
    public var sampler: SamplerState
    public var error: String?
    public var bucketSecs: UInt64
    public var buckets: [TrafficBucket]
    /// Sorted by bytes in, most first.
    public var apps: [AppTraffic]
    /// Sorted by bytes, most first; at most 2,000.
    public var remotes: [RemoteTraffic]
    public var totals: TrafficTotals
    /// Places are on and the databases are on this Mac.
    public var places: Bool
}

/// The DB-IP Lite databases on this Mac.
public struct GeoDbInfo: Codable, Equatable, Sendable {
    /// `2026-11`.
    public var date: String
    public var bytes: UInt64
}

public typealias UpdateGeoDbResult = GeoDbInfo

public struct ClearTrafficHistoryResult: Codable, Equatable, Sendable {
    public var removed: Int
}
