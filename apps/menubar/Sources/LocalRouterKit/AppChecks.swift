// The results of the app checks and the rules on them (ADR 11, change 3,
// 3.2 and 3.4): the cache file, when a program needs review, the trusted
// list, and the notifications.
//
// The cache lives in the app's Caches folder, not in the daemon's data
// folder: the daemon stays the only writer of its folder. Deleting the cache
// costs only new checks and lookups.

import Foundation

/// What is known about one executable file, by its SHA-256.
public struct AppCheckRecord: Codable, Equatable, Sendable {
    public var signature: SignatureCheck
    /// Nil until looked up (or never, for Apple programs).
    public var virusTotal: VirusTotalReport?
    public var virusTotalAt: Date?
    public var checkedAt: Date

    public init(signature: SignatureCheck, virusTotal: VirusTotalReport? = nil, virusTotalAt: Date? = nil, checkedAt: Date) {
        self.signature = signature
        self.virusTotal = virusTotal
        self.virusTotalAt = virusTotalAt
        self.checkedAt = checkedAt
    }
}

/// `app-checks.json`: SHA-256 → record, and the lookup counter.
public struct AppChecksCache: Codable, Equatable, Sendable {
    public static let fileName = "app-checks.json"
    /// A VirusTotal result is looked up again after this time.
    public static let lookupMaxAge: TimeInterval = 7 * 24 * 3600

    public var records: [String: AppCheckRecord] = [:]
    public var limiter = RateLimiter()

    public init() {}

    /// `~/Library/Caches/<bundle id>/`.
    public static func defaultFolder(_ instance: Instance = .current) -> URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Caches", isDirectory: true)
            .appendingPathComponent(instance.bundleID, isDirectory: true)
    }

    /// The cache in `folder`; empty when the file is missing or damaged.
    public static func load(from folder: URL) -> AppChecksCache {
        let url = folder.appendingPathComponent(fileName)
        guard let data = try? Data(contentsOf: url) else { return AppChecksCache() }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return (try? decoder.decode(AppChecksCache.self, from: data)) ?? AppChecksCache()
    }

    /// Write the whole file, replacing the old one at once.
    public func save(to folder: URL) throws {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.sortedKeys]
        try encoder.encode(self).write(to: folder.appendingPathComponent(Self.fileName), options: .atomic)
    }

    /// Whether this hash needs a VirusTotal lookup now: never for an Apple
    /// program (I7); once per hash, again after 7 days or on Check again.
    public func needsLookup(sha256: String, status: SignatureStatus, now: Date, force: Bool = false) -> Bool {
        guard VirusTotal.shouldLookUp(status) else { return false }
        if force { return true }
        guard let at = records[sha256]?.virusTotalAt else { return true }
        return now.timeIntervalSince(at) >= Self.lookupMaxAge
    }

    public mutating func store(signature: SignatureCheck, now: Date) {
        var record = records[signature.sha256] ?? AppCheckRecord(signature: signature, checkedAt: now)
        record.signature = signature
        record.checkedAt = now
        records[signature.sha256] = record
    }

    public mutating func store(virusTotal: VirusTotalReport, sha256: String, now: Date) {
        guard var record = records[sha256] else { return }
        record.virusTotal = virusTotal
        record.virusTotalAt = now
        records[sha256] = record
    }
}

/// The verdict line of the details window and the color of the shield.
public enum AppVerdict: Equatable, Sendable {
    /// An Apple program: grey shield.
    case apple
    /// Nothing wrong found.
    case ok
    /// One engine on VirusTotal: the number in amber, no review (one engine
    /// alone is often a false positive).
    case caution
    /// Needs review: banner, red dot, menu item, notification.
    case needsReview
    /// The user trusted this exact file.
    case trusted
}

public enum AppChecks {
    /// `malicious` at or above this needs review.
    public static let maliciousThreshold = 2

    /// The rule of ADR 11, 3.4.
    public static func needsReview(signature: SignatureStatus, virusTotal: VirusTotalReport?, warnAdhoc: Bool, trusted: Bool) -> Bool {
        verdict(signature: signature, virusTotal: virusTotal, warnAdhoc: warnAdhoc, trusted: trusted) == .needsReview
    }

    public static func verdict(signature: SignatureStatus, virusTotal: VirusTotalReport?, warnAdhoc: Bool, trusted: Bool) -> AppVerdict {
        let malicious = virusTotal?.state == .found ? virusTotal?.malicious ?? 0 : 0
        let bad = signature == .unsigned || signature == .broken || malicious >= maliciousThreshold
            || (signature == .adhoc && warnAdhoc)
        if bad { return trusted ? .trusted : .needsReview }
        if signature == .apple { return .apple }
        if malicious == 1 { return .caution }
        return .ok
    }

    /// Quit process is not offered for Apple programs: stopping
    /// mDNSResponder stops DNS for the whole Mac (G6).
    public static func canQuit(_ status: SignatureStatus?) -> Bool {
        guard let status else { return false }
        return status != .apple
    }
}

/// The programs the user trusted, by SHA-256 (I12): a changed file is not
/// trusted any more. Kept in `UserDefaults`, shown in Settings > Traffic.
public final class TrustedPrograms: @unchecked Sendable {
    public static let key = "trustedPrograms"
    private let defaults: UserDefaults
    private let lock = NSLock()

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    /// SHA-256 → the path it had when trusted, for the list in Settings.
    public var all: [String: String] {
        lock.withLock { defaults.dictionary(forKey: Self.key) as? [String: String] ?? [:] }
    }

    public func isTrusted(sha256: String) -> Bool {
        all[sha256.lowercased()] != nil
    }

    public func trust(sha256: String, path: String) {
        lock.withLock {
            var list = defaults.dictionary(forKey: Self.key) as? [String: String] ?? [:]
            list[sha256.lowercased()] = path
            defaults.set(list, forKey: Self.key)
        }
    }

    public func untrust(sha256: String) {
        lock.withLock {
            var list = defaults.dictionary(forKey: Self.key) as? [String: String] ?? [:]
            list[sha256.lowercased()] = nil
            defaults.set(list, forKey: Self.key)
        }
    }

    /// For the uninstaller.
    public func removeAll() {
        lock.withLock { defaults.removeObject(forKey: Self.key) }
    }
}

/// Which notifications to post when programs start to need review (ADR 11,
/// 3.4, G7): one per program, but when more than 3 need review within 10
/// minutes, one summary "5 programs need review" instead. The summary has a
/// fixed id, so a newer one replaces the older one.
public struct ReviewNotifier: Sendable {
    public static let window: TimeInterval = 10 * 60
    public static let maxSingles = 3
    public static let summaryID = "review-summary"

    public struct Notice: Equatable, Sendable {
        /// The notification identifier: the executable path, or `summaryID`.
        public var id: String
        public var title: String
        public var body: String
    }

    /// The program to notify about: its app name and executable path.
    public struct Program: Equatable, Sendable {
        public var name: String
        public var path: String

        public init(name: String, path: String) {
            self.name = name
            self.path = path
        }
    }

    private var times: [Date] = []

    public init() {}

    /// Programs that need review now and were not notified before. Returns
    /// what to post.
    public mutating func add(_ programs: [Program], now: Date) -> [Notice] {
        guard !programs.isEmpty else { return [] }
        times = times.filter { now.timeIntervalSince($0) < Self.window } + programs.map { _ in now }
        if times.count > Self.maxSingles {
            let n = times.count
            return [Notice(id: Self.summaryID, title: "\(n) programs need review",
                           body: "Open the Traffic tab to see who signed them and what VirusTotal says.")]
        }
        return programs.map {
            Notice(id: $0.path, title: "\($0.name) needs review",
                   body: "It connected to the network. Open it to see who signed it and what VirusTotal says.")
        }
    }
}
