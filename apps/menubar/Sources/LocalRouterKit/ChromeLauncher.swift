// "Open Chrome via Proxy" (ADR 06, change 3): a separate Chrome instance with
// its own profile and the proxy flag, so the user's normal Chrome does not
// change. The arguments come from the daemon's get_proxy; this file builds
// none of its own (I18).

import AppKit
import Foundation

/// Finding and opening an app. A protocol so the test can record the calls
/// instead of starting Chrome (T14).
public protocol AppOpener: Sendable {
    func appURL(bundleID: String) -> URL?
    /// Opens a new instance of the app with these arguments, even when the
    /// app already runs.
    func openNewInstance(_ app: URL, arguments: [String]) async throws
}

/// The two daemon calls the launcher makes. `DaemonClient` has both.
public protocol ProxySettings: Sendable {
    func proxy() async throws -> GetProxyResult
    func setConfig(_ p: SetConfigParams) async throws -> SetConfigResult
}

extension DaemonClient: ProxySettings {}

public struct WorkspaceOpener: AppOpener {
    public init() {}

    public func appURL(bundleID: String) -> URL? {
        NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID)
    }

    public func openNewInstance(_ app: URL, arguments: [String]) async throws {
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.createsNewApplicationInstance = true
        configuration.arguments = arguments
        _ = try await NSWorkspace.shared.openApplication(at: app, configuration: configuration)
    }
}

public struct ChromeLauncher: Sendable {
    /// Google Chrome only; Canary, Chromium, Brave and Edge are not in ADR 06.
    public static let chromeBundleID = "com.google.Chrome"

    let opener: AppOpener
    let daemon: ProxySettings

    public init(opener: AppOpener = WorkspaceOpener(), daemon: ProxySettings) {
        self.opener = opener
        self.daemon = daemon
    }

    /// The menu shows the item only when this is true; it is asked each time
    /// the menu opens (I17).
    public var chromeInstalled: Bool { opener.appURL(bundleID: Self.chromeBundleID) != nil }

    /// Turn the proxy on if needed, then start Chrome with the daemon's
    /// arguments. A failure starts no Chrome.
    public func launch() async -> Feedback {
        guard let chrome = opener.appURL(bundleID: Self.chromeBundleID) else {
            return .failed("Google Chrome is not installed", "Open Chrome via Proxy needs Google Chrome.")
        }
        do {
            var proxy = try await daemon.proxy()
            if !proxy.enabled {
                _ = try await daemon.setConfig(SetConfigParams(proxyEnabled: true))
                proxy = try await daemon.proxy()
            }
            // Chrome reads --proxy-server only in a new process, and a second
            // launch with the user's own profile joins their running Chrome.
            // A separate profile is the only safe way (I17).
            guard Self.hasOwnProfile(proxy.chromeArgs) else {
                return .failed("Chrome was not started", "The daemon gave no separate Chrome profile.")
            }
            try await opener.openNewInstance(chrome, arguments: proxy.chromeArgs)
            let address = proxy.url.replacingOccurrences(of: "http://", with: "")
            return Feedback(title: "Chrome started with the proxy at \(address)",
                            detail: (["Only this new Chrome window uses the proxy."] + proxy.notes).joined(separator: " "))
        } catch {
            return .failed("Chrome was not started", error.localizedDescription)
        }
    }

    /// True when the arguments name a profile folder that is not Chrome's
    /// default one.
    public static func hasOwnProfile(_ arguments: [String]) -> Bool {
        arguments.contains { arg in
            guard arg.hasPrefix("--user-data-dir=") else { return false }
            let dir = arg.dropFirst("--user-data-dir=".count)
            return !dir.isEmpty && !dir.contains("Library/Application Support/Google/Chrome")
        }
    }
}
