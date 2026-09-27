// The Daemon section of Settings (ADR 04, 02, I12). It shows the version and
// the ports only when the ports are not 80 and 443, so the release looks as
// before; it always shows when something is wrong. Ports are edited by hand
// in config.json, and a restart binds them; the section says so when the
// file names ports the daemon does not use yet.

import Foundation

public struct DaemonSection: Equatable, Sendable {
    public struct Ports: Equatable, Sendable {
        public var http: UInt16
        public var https: UInt16

        public init(http: UInt16, https: UInt16) {
            self.http = http
            self.https = https
        }
    }

    public var visible: Bool
    /// Ports in config.json that the daemon has not bound yet, or nil.
    public var pendingPorts: Ports?

    public static func decide(bound: Ports?, file: Ports?, hasProblem: Bool) -> DaemonSection {
        let customPorts = bound.map { $0.http != 80 || $0.https != 443 } ?? false
        // Port 0 means "any free port" (tests): the bound port always differs.
        let pending = file.flatMap { f in
            bound.flatMap { b in (f != b && f.http != 0 && f.https != 0) ? f : nil }
        }
        return DaemonSection(visible: hasProblem || customPorts || pending != nil, pendingPorts: pending)
    }

    /// The ports of config.json as the daemon reads them: a missing field
    /// gets the instance's default. Nil when there is no readable file.
    public static func filePorts(configURL: URL, instance: Instance) -> Ports? {
        guard let data = try? Data(contentsOf: configURL),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        func port(_ key: String, _ fallback: UInt16) -> UInt16? {
            guard let value = json[key] else { return fallback }
            return (value as? NSNumber).flatMap { UInt16(exactly: $0.intValue) }
        }
        guard let http = port("http_port", instance.defaultPorts.http),
              let https = port("https_port", instance.defaultPorts.https) else { return nil }
        return Ports(http: http, https: https)
    }

    /// Restarts this instance's daemon, so it binds the ports in config.json.
    public static func restartCommand(for instance: Instance) -> String {
        "launchctl kickstart -k gui/$(id -u)/\(instance.daemonLabel)"
    }
}
