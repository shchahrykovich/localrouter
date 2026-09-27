// Client for the daemon's socket API. Holds no route state (ADR 01, change 5).

import Foundation

public enum Paths {
    /// $LOCALROUTER_HOME wins, so tests never touch ~/Library. Otherwise each
    /// instance has its own folders: LocalRouter, LocalRouter-dev (ADR 04).
    public static func resolve(instance: Instance, localHome: String?, home: URL) -> (data: URL, logs: URL) {
        if let localHome, !localHome.isEmpty {
            let root = URL(fileURLWithPath: localHome)
            return (root, root.appendingPathComponent("logs"))
        }
        return (home.appendingPathComponent(instance.dataFolder), home.appendingPathComponent(instance.logsFolder))
    }

    private static var resolved: (data: URL, logs: URL) {
        resolve(instance: .current, localHome: ProcessInfo.processInfo.environment["LOCALROUTER_HOME"],
                home: FileManager.default.homeDirectoryForCurrentUser)
    }

    /// The data folder of the running instance.
    public static var dataDir: URL { resolved.data }
    public static var logsDir: URL { resolved.logs }

    public static var socket: URL { dataDir.appendingPathComponent("daemon.sock") }
    public static var caPem: URL { dataDir.appendingPathComponent("ca/ca.pem") }
}

public enum DaemonError: Error, LocalizedError, Equatable {
    case notRunning
    case versionMismatch(daemon: String)
    case api(ApiError)
    case io(String)

    public var errorDescription: String? {
        switch self {
        case .notRunning: "LocalRouter is not running."
        case let .versionMismatch(daemon): "The daemon speaks API \(daemon), this app speaks \(apiVersion). Update the older one."
        case let .api(e): e.message
        case let .io(text): text
        }
    }
}

/// One Unix socket connection, newline-delimited JSON.
final class LineSocket {
    private let fd: Int32
    private var buffer = Data()

    init(path: String, timeout: TimeInterval = 10) throws {
        fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw DaemonError.io("socket() failed") }
        var on: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
        if timeout > 0 {
            var tv = timeval(tv_sec: Int(timeout), tv_usec: 0)
            setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        }
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        let capacity = MemoryLayout.size(ofValue: addr.sun_path)
        guard bytes.count < capacity else {
            close(fd)
            throw DaemonError.io("socket path too long")
        }
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        let result = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        if result != 0 {
            let code = errno
            close(fd)
            throw (code == ENOENT || code == ECONNREFUSED) ? DaemonError.notRunning : DaemonError.io(String(cString: strerror(code)))
        }
    }

    deinit { close(fd) }

    func send(_ data: Data) throws {
        var line = data
        line.append(0x0A)
        try line.withUnsafeBytes { raw in
            var offset = 0
            while offset < raw.count {
                let n = write(fd, raw.baseAddress! + offset, raw.count - offset)
                if n <= 0 { throw DaemonError.io("write failed") }
                offset += n
            }
        }
    }

    func readLine() throws -> Data {
        while true {
            if let nl = buffer.firstIndex(of: 0x0A) {
                let line = buffer[buffer.startIndex..<nl]
                buffer.removeSubrange(buffer.startIndex...nl)
                return Data(line)
            }
            var chunk = [UInt8](repeating: 0, count: 16 * 1024)
            let n = read(fd, &chunk, chunk.count)
            if n <= 0 { throw DaemonError.io(n == 0 ? "the daemon closed the connection" : "read failed") }
            buffer.append(contentsOf: chunk[0..<n])
        }
    }

    /// Unblock a reader on another thread.
    func shutdownBoth() { shutdown(fd, SHUT_RDWR) }
}

private struct RequestEnvelope<P: Encodable>: Encodable {
    var id: UInt64
    var method: String
    var params: P
}

private struct ReplyHeader: Decodable {
    var id: UInt64?
    var error: ApiError?
    var event: String?
}

private struct ReplyResult<R: Decodable>: Decodable {
    var result: R
}

/// Calls the daemon. Each call uses its own short connection, so there is no
/// state to recover when the daemon restarts.
public final class DaemonClient: Sendable {
    public let socketPath: String
    private let queue = DispatchQueue(label: "localrouter.daemon-client")

    public init(socketPath: String = Paths.socket.path) {
        self.socketPath = socketPath
    }

    public func call<P: Encodable & Sendable, R: Decodable & Sendable>(_ method: String, _ params: P, as: R.Type = R.self) async throws -> R {
        let path = socketPath
        return try await withCheckedThrowingContinuation { cont in
            queue.async {
                do {
                    let socket = try LineSocket(path: path)
                    let hello: HelloResult = try Self.exchange(socket, id: 1, method: "hello",
                                                               params: HelloParams(client: "menubar", apiVersion: apiVersion))
                    guard apiMajor(hello.apiVersion) == apiMajor(apiVersion) else {
                        throw DaemonError.versionMismatch(daemon: hello.apiVersion)
                    }
                    let result: R = try Self.exchange(socket, id: 2, method: method, params: params)
                    cont.resume(returning: result)
                } catch let e as DaemonError {
                    cont.resume(throwing: e)
                } catch {
                    cont.resume(throwing: DaemonError.io(error.localizedDescription))
                }
            }
        }
    }

    static func exchange<P: Encodable, R: Decodable>(_ socket: LineSocket, id: UInt64, method: String, params: P) throws -> R {
        try socket.send(Api.encoder.encode(RequestEnvelope(id: id, method: method, params: params)))
        while true {
            let line = try socket.readLine()
            let header = try Api.decoder.decode(ReplyHeader.self, from: line)
            if header.event != nil || header.id != id { continue }
            if let error = header.error {
                if error.code == "version_mismatch" { throw DaemonError.versionMismatch(daemon: "another version") }
                throw DaemonError.api(error)
            }
            return try Api.decoder.decode(ReplyResult<R>.self, from: line).result
        }
    }

    // MARK: Convenience

    public func status() async throws -> StatusResult { try await call("status", Empty()) }
    public func listRoutes() async throws -> [RouteView] { try await call("list_routes", Empty(), as: ListRoutesResult.self).routes }
    /// Removes exactly this route: its host and, for a path route, its path.
    /// Sending the host alone would remove the route without a path instead
    /// (ADR 03, manifest B1).
    public func unregister(_ route: Route) async throws -> Bool {
        try await call("unregister_route", HostParams(route: route), as: UnregisterRouteResult.self).removed
    }
    public func register(_ route: Route) async throws -> RegisterRouteResult { try await call("register_route", route) }
    public func logs(limit: Int = 500) async throws -> [LogEntry] {
        try await call("get_logs", GetLogsParams(limit: limit), as: GetLogsResult.self).entries
    }
    public func config() async throws -> Config { try await call("get_config", Empty()) }
    public func setConfig(_ p: SetConfigParams) async throws -> SetConfigResult { try await call("set_config", p) }
}

/// Live log stream on its own connection and thread.
public final class LogSubscription: @unchecked Sendable {
    private var socket: LineSocket?
    private let lock = NSLock()
    private var stopped = false

    public init() {}

    /// Calls `onEntry` on a background thread for each new entry, until `stop()`.
    /// `onEnd` runs once when the stream ends for any reason.
    public func start(socketPath: String = Paths.socket.path, onEntry: @escaping @Sendable (LogEntry) -> Void,
                      onEnd: @escaping @Sendable () -> Void) {
        Thread.detachNewThread { [self] in
            defer { onEnd() }
            guard let socket = try? LineSocket(path: socketPath, timeout: 0) else { return }
            lock.lock()
            if stopped { lock.unlock(); return }
            self.socket = socket
            lock.unlock()
            do {
                let _: HelloResult = try DaemonClient.exchange(socket, id: 1, method: "hello",
                                                               params: HelloParams(client: "menubar", apiVersion: apiVersion))
                let _: SubscribeLogsResult = try DaemonClient.exchange(socket, id: 2, method: "subscribe_logs",
                                                                       params: SubscribeLogsParams(host: nil))
                while true {
                    let line = try socket.readLine()
                    if let event = try? Api.decoder.decode(LogEvent.self, from: line) {
                        onEntry(event.entry)
                    }
                }
            } catch {}
        }
    }

    public func stop() {
        lock.lock()
        stopped = true
        socket?.shutdownBoth()
        lock.unlock()
    }
}
