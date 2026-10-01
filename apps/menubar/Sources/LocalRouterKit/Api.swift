// Swift copy of the socket API shapes in libs/core/src/api.rs.
//
// JSON keys are snake_case; the coders below convert them. Every file in
// api/examples must decode here and in Rust (invariant I11, test T9).

import Foundation

public let apiVersion = "1.3"

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

public enum LogEntry: Codable, Equatable, Sendable, Identifiable {
    /// `route` is the key of the route that answered, for example `shop/blog`.
    /// `proxy` is set for traffic of the forward proxy (ADR 06).
    case http(time: UInt64, method: String, host: String, path: String, status: UInt16, durationMs: UInt64, route: String?,
              proxy: ProxyTraffic?)
    case tcp(time: UInt64, host: String, listenPort: UInt16, bytesIn: UInt64, bytesOut: UInt64, durationMs: UInt64, failed: Bool)

    public var id: String {
        switch self {
        case let .http(time, method, host, path, status, _, _, _): "\(time)-\(method)-\(host)\(path)-\(status)"
        case let .tcp(time, host, port, bytesIn, _, _, _): "\(time)-\(host):\(port)-\(bytesIn)"
        }
    }

    public var host: String {
        switch self {
        case let .http(_, _, host, _, _, _, _, _), let .tcp(_, host, _, _, _, _, _): host
        }
    }

    enum CodingKeys: String, CodingKey {
        case kind, timeMs, method, host, path, status, durationMs, route, listenPort, bytesIn, bytesOut, failed, via, mode
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
            self = .http(time: time, method: try c.decode(String.self, forKey: .method), host: host,
                         path: try c.decode(String.self, forKey: .path), status: try c.decode(UInt16.self, forKey: .status),
                         durationMs: duration, route: try c.decodeIfPresent(String.self, forKey: .route), proxy: proxy)
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
        case let .http(time, method, host, path, status, duration, route, proxy):
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
    public init(fallback: Bool? = nil, allowLan: Bool? = nil, proxyEnabled: Bool? = nil, proxyPort: UInt16? = nil,
                inspectHosts: [String]? = nil) {
        self.fallback = fallback
        self.allowLan = allowLan
        self.proxyEnabled = proxyEnabled
        self.proxyPort = proxyPort
        self.inspectHosts = inspectHosts
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

/// Everything a client needs to use the forward proxy (ADR 06).
public struct GetProxyResult: Codable, Equatable, Sendable {
    public var enabled: Bool
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
}
