import AppKit
import Foundation

/// Opens a URL, in a given app or in the default browser.
public protocol URLOpening: Sendable {
    func open(_ url: URL, withApplicationAt app: URL?) async throws
}

public struct WorkspaceURLOpener: URLOpening {
    public init() {}

    public func open(_ url: URL, withApplicationAt app: URL?) async throws {
        guard let app else {
            NSWorkspace.shared.open(url)
            return
        }
        _ = try await NSWorkspace.shared.open([url], withApplicationAt: app, configuration: NSWorkspace.OpenConfiguration())
    }
}

/// The proxy log (ADR 08) as the app shows it: where "Open Proxy Log" goes,
/// and the line under the proxy switch.
public enum ProxyLog {
    /// The user's normal Chrome when it is installed, else the default
    /// browser. Never the separate Chrome that uses the proxy: the viewer
    /// would then read its own requests.
    public static func browser(chrome: URL?) -> URL? { chrome }

    /// Open the viewer. `chrome` is `nil` when Google Chrome is not installed.
    public static func open(_ log: ProxyLogStatus, chrome: URL?, opener: URLOpening = WorkspaceURLOpener()) async -> Feedback {
        guard let url = URL(string: log.url) else { return .failed("The proxy log has no address", log.url) }
        do {
            try await opener.open(url, withApplicationAt: browser(chrome: chrome))
            return Feedback(title: "Opened the proxy log", detail: log.url)
        } catch {
            return .failed("The proxy log was not opened", error.localizedDescription)
        }
    }

    /// "Log: on, 912 requests in proxy-20261002-093512.har". `nil` from a
    /// daemon without the log.
    public static func stateLine(_ log: ProxyLogStatus?) -> String? {
        guard let log else { return nil }
        guard log.enabled else { return "Log: off" }
        if log.error != nil { return "Log: stopped, see the error" }
        guard let current = log.current else { return "Log: on, no request yet" }
        let n = log.written == 1 ? "1 request" : "\(log.written) requests"
        return "Log: on, \(n) in \(current)"
    }

    /// Lines to show in red or orange: the writer error and dropped records.
    public static func problems(_ log: ProxyLogStatus?) -> [String] {
        guard let log else { return [] }
        var out: [String] = []
        if let e = log.error { out.append("The log stopped: \(e). Turn it off and on to try again.") }
        if log.dropped > 0 { out.append("\(log.dropped) requests were not written: the disk was too slow.") }
        return out
    }

    /// The explanation under the switch in Settings.
    public static func explanation(keepFiles: Int) -> String {
        "Keeps the \(keepFiles) newest files. Secret headers are written as [redacted]; URLs are written as they are."
    }
}
