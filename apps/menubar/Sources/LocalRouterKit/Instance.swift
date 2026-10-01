// An instance is one complete copy of LocalRouter: app, daemon, CLI, data
// folder, CA, links and ports (ADR 04). Its suffix is fixed when the bundle is
// built (`LRInstanceSuffix` in Info.plist): empty for the release, `-dev` for
// a local build. Every name is the release name plus the suffix.
//
// The Rust copy of these rules is libs/core/src/instance.rs. Both are checked
// against api/instance-names.json.

import Foundation

public struct Instance: Equatable, Hashable, Sendable, CustomStringConvertible {
    public let suffix: String

    /// The Info.plist key build-app.sh writes.
    public static let infoKey = "LRInstanceSuffix"
    /// Longest suffix, dash included. Keeps the socket path under the macOS limit.
    public static let maxSuffixLength = 16

    public static let release = Instance(checked: "")

    public enum Failure: Error, LocalizedError, Equatable {
        case badSuffix(String)

        public var errorDescription: String? {
            switch self {
            case let .badSuffix(s):
                "Invalid instance suffix \"\(s)\": use \"\" or \"-\" followed by 1 to 15 of a-z and 0-9, like \"-dev\"."
            }
        }
    }

    public init(suffix: String) throws {
        guard Self.isValid(suffix) else { throw Failure.badSuffix(suffix) }
        self.suffix = suffix
    }

    private init(checked suffix: String) {
        self.suffix = suffix
    }

    public static func isValid(_ s: String) -> Bool {
        if s.isEmpty { return true }
        guard s.hasPrefix("-"), s.count >= 2, s.utf8.count <= maxSuffixLength else { return false }
        return s.utf8.dropFirst().allSatisfy { (0x61...0x7a).contains($0) || (0x30...0x39).contains($0) }
    }

    /// The instance of a bundle, from its Info.plist. No key (for example
    /// `swift run`) is the release.
    public static func of(bundle: Bundle) throws -> Instance {
        try Instance(suffix: bundle.object(forInfoDictionaryKey: infoKey) as? String ?? "")
    }

    /// The running app's instance. A bad suffix in Info.plist is a broken
    /// build; the app reports it (see `bundleProblem`) and this falls back to
    /// the release names.
    public static let current: Instance = (try? of(bundle: .main)) ?? .release

    /// Why the running app cannot tell its instance, or nil.
    public static var currentProblem: String? {
        do {
            _ = try of(bundle: .main)
            return nil
        } catch {
            return "This build is broken: \(error.localizedDescription)"
        }
    }

    public var isRelease: Bool { suffix.isEmpty }
    public var description: String { appName }

    /// `LocalRouter-dev`: the app's name, and the name of its folders.
    public var appName: String { "LocalRouter\(suffix)" }
    public var bundleID: String { "dev.localrouter.app\(suffix)" }
    public var daemonLabel: String { "\(bundleID).daemon" }
    public var daemonProgram: String { "localrouterd\(suffix)" }
    /// The CLI's name, also the MCP server's name.
    public var cli: String { "localrouter\(suffix)" }
    /// Relative to the home folder.
    public var dataFolder: String { "Library/Application Support/\(appName)" }
    /// Relative to the home folder.
    public var logsFolder: String { "Library/Logs/\(appName)" }
    /// Relative to the home folder. Chrome writes the proxy profile there (ADR 06).
    public var cachesFolder: String { "Library/Caches/\(appName)" }
    /// The Claude Code note, linked into ~/.claude.
    public var note: String { "\(appName).md" }
    public var caNamePrefix: String { "\(appName) CA" }
    /// The common name of a new inspection CA starts with this (ADR 06).
    public var inspectCaNamePrefix: String { "\(appName) Inspection" }
    /// Ports a new config.json gets. 8080 is avoided: many dev servers use it.
    public var defaultPorts: (http: UInt16, https: UInt16) { isRelease ? (80, 443) : (7080, 7443) }
    /// The forward proxy port a new config.json gets (ADR 06).
    public var defaultProxyPort: UInt16 { isRelease ? 8877 : 7877 }
    public var defaultHelpURL: String { AgentHelp.url(httpPort: defaultPorts.http, httpsPort: nil) }

    /// Why this bundle cannot run as `self`, or nil. The daemon and the CLI
    /// read their instance from their own file names, so they must carry this
    /// instance's names (ADR 04, 01).
    public func bundleProblem(_ bundleURL: URL) -> String? {
        let fm = FileManager.default
        for program in [BundleLayout.daemon(for: self), BundleLayout.cli(for: self)] {
            let url = bundleURL.appendingPathComponent(program)
            if !fm.isExecutableFile(atPath: url.path) {
                return "This \(appName) build is broken: \(program) is missing. Build it again with scripts/build-app.sh."
            }
        }
        return nil
    }
}
