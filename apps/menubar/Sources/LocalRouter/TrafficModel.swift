import AppKit
import LocalRouterKit
import Observation
import UserNotifications

/// Traffic by app (ADR 11) on the app side: the last hour from the daemon,
/// the app checks of each executable (code signature, VirusTotal by hash),
/// and which apps need review. The checks run here, not in the daemon: they
/// need Security.framework, the Keychain and dialogs (ADR 11, change 3).
@MainActor
@Observable
final class TrafficModel {
    /// The settings of Settings › Traffic that live in the app.
    enum Key {
        static let checkSignatures = "traffic.checkSignatures"
        static let lookUpVirusTotal = "traffic.lookUpVirusTotal"
        static let warnAdhoc = "traffic.warnAdhoc"
        static let redDot = "traffic.redDot"
        static let notify = "traffic.notify"
        static let askAddresses = "traffic.askAddresses"
        static let askService = "traffic.askService"
        static let thisMac = "traffic.thisMac"
    }

    /// The last hour, refreshed with the status while the sampler is on.
    var hour: AppTrafficResult?
    /// The signature check of each executable path.
    private(set) var checks: [String: SignatureCheck] = [:]
    /// Paths whose check failed (the file is gone): no new attempt each poll.
    private var failed: Set<String> = []
    private(set) var cache: AppChecksCache
    /// SHA-256s with a lookup running.
    private var lookingUp: Set<String> = []
    /// Bumped when the trusted list changes, so views read it again.
    private(set) var trustedVersion = 0
    var keySaved: Bool
    private var notifier = ReviewNotifier()
    /// Executables already announced in a notification.
    private var notified: Set<String> = []
    private var checkQueue: [String] = []
    private var checking = false

    let trusted = TrustedPrograms()
    let keychain = KeychainStore.virusTotal()
    private let cacheFolder = AppChecksCache.defaultFolder()
    private let defaults = UserDefaults.standard

    init() {
        cache = AppChecksCache.load(from: cacheFolder)
        keySaved = KeychainStore.virusTotal().read() != nil
        defaults.register(defaults: [
            Key.checkSignatures: true, Key.lookUpVirusTotal: false, Key.warnAdhoc: false, Key.redDot: true, Key.notify: true,
            Key.askAddresses: true, Key.askService: "claude",
        ])
    }

    private func flag(_ key: String) -> Bool { defaults.bool(forKey: key) }

    // MARK: Data

    /// A new reply: check the executables not seen before.
    func update(_ result: AppTrafficResult?, inBundle: Bool) {
        hour = result
        guard let result, flag(Key.checkSignatures) else { return }
        let paths = Set(result.apps.flatMap(\.paths))
        let new = paths.filter { checks[$0] == nil && !failed.contains($0) && !checkQueue.contains($0) }
        guard !new.isEmpty else { return }
        checkQueue.append(contentsOf: new.sorted())
        runChecks(inBundle: inBundle)
    }

    /// One file at a time, off the main thread: hashing reads the whole file.
    private func runChecks(inBundle: Bool) {
        guard !checking else { return }
        checking = true
        Task {
            while let path = checkQueue.first {
                checkQueue.removeFirst()
                let result = await Task.detached(priority: .utility) { try? AppIdentity.check(path: path) }.value
                if let result {
                    checks[path] = result
                    cache.store(signature: result, now: Date())
                } else {
                    failed.insert(path)
                }
            }
            checking = false
            try? cache.save(to: cacheFolder)
            await lookUpNew()
            notifyNew(inBundle: inBundle)
        }
    }

    /// Check one executable again: its signature and, with a key, VirusTotal.
    func checkAgain(_ path: String) async {
        failed.remove(path)
        guard let result = await Task.detached(priority: .userInitiated, operation: { try? AppIdentity.check(path: path) }).value else {
            checks[path] = nil
            failed.insert(path)
            return
        }
        checks[path] = result
        cache.store(signature: result, now: Date())
        await lookUp(result, force: true)
        try? cache.save(to: cacheFolder)
    }

    // MARK: VirusTotal

    /// Look up every checked hash that needs it, within the limits. Only
    /// with the setting on and a key saved (ADR 11, 3.2).
    private func lookUpNew() async {
        guard flag(Key.lookUpVirusTotal), keySaved else { return }
        for check in checks.values where cache.needsLookup(sha256: check.sha256, status: check.status, now: Date()) {
            guard case .now = cache.limiter.decide(now: Date()) else { break }
            await lookUp(check, force: false)
        }
        try? cache.save(to: cacheFolder)
    }

    /// One lookup by hash: a GET, never the file (I6). Never for Apple (I7).
    func lookUp(_ check: SignatureCheck, force: Bool) async {
        guard keySaved, let key = keychain.read(),
              cache.needsLookup(sha256: check.sha256, status: check.status, now: Date(), force: force),
              !lookingUp.contains(check.sha256) else { return }
        guard case .now = cache.limiter.decide(now: Date()) else {
            lastError = "VirusTotal: the limit of the free key is reached. Try again later."
            return
        }
        lookingUp.insert(check.sha256)
        defer { lookingUp.remove(check.sha256) }
        cache.limiter.record(now: Date())
        do {
            let report = try await VirusTotalClient(apiKey: key).lookup(sha256: check.sha256)
            cache.store(virusTotal: report, sha256: check.sha256, now: Date())
            lastError = nil
        } catch {
            lastError = "VirusTotal: \(error.localizedDescription)"
        }
    }

    var lastError: String?

    func isLookingUp(_ sha256: String) -> Bool { lookingUp.contains(sha256) }

    var lookupsToday: Int { cache.limiter.lookupsToday(now: Date()) }

    func report(for sha256: String) -> (VirusTotalReport, Date?)? {
        guard let r = cache.records[sha256], let vt = r.virusTotal else { return nil }
        return (vt, r.virusTotalAt)
    }

    func saveKey(_ key: String) -> String? {
        do {
            try keychain.save(key.trimmingCharacters(in: .whitespacesAndNewlines))
            keySaved = keychain.read() != nil
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    /// Upload one file after the warning dialog. The confirmation can only
    /// be made by that dialog (I9).
    func upload(_ confirmation: UploadConfirmation, check: SignatureCheck) async -> String {
        guard let key = keychain.read() else { return "Save a VirusTotal API key in Settings › Traffic first." }
        do {
            _ = try await VirusTotalClient(apiKey: key).upload(file: confirmation.file, confirmation: confirmation)
            // VirusTotal needs time to scan; look the hash up again in a minute.
            Task {
                try? await Task.sleep(for: .seconds(60))
                await lookUp(check, force: true)
                try? cache.save(to: cacheFolder)
            }
            return "Uploaded. The result appears in about a minute."
        } catch {
            return "Upload failed: \(error.localizedDescription)"
        }
    }

    // MARK: Verdicts

    func isTrusted(_ sha256: String) -> Bool {
        _ = trustedVersion
        return trusted.isTrusted(sha256: sha256)
    }

    func trust(_ check: SignatureCheck) {
        trusted.trust(sha256: check.sha256, path: check.path)
        trustedVersion += 1
    }

    func untrust(_ sha256: String) {
        trusted.untrust(sha256: sha256)
        trustedVersion += 1
    }

    func verdict(_ check: SignatureCheck) -> AppVerdict {
        AppChecks.verdict(signature: check.status, virusTotal: cache.records[check.sha256]?.virusTotal,
                          warnAdhoc: flag(Key.warnAdhoc), trusted: isTrusted(check.sha256))
    }

    /// The worst verdict of an app's executables; nil before any is checked.
    func verdict(_ app: AppTraffic) -> AppVerdict? {
        let verdicts = app.paths.compactMap { checks[$0] }.map(verdict)
        if verdicts.contains(.needsReview) { return .needsReview }
        if verdicts.contains(.caution) { return .caution }
        if verdicts.contains(.trusted) { return .trusted }
        if verdicts.contains(.ok) { return .ok }
        return verdicts.first
    }

    /// The apps of the last hour that need review (ADR 11, 3.4).
    var appsToReview: [AppTraffic] {
        (hour?.apps ?? []).filter { verdict($0) == .needsReview }
    }

    var showRedDot: Bool { flag(Key.redDot) && !appsToReview.isEmpty }

    // MARK: Notifications

    /// One notification per new executable that needs review; above three
    /// in ten minutes, one summary (G7).
    private func notifyNew(inBundle: Bool) {
        guard inBundle, flag(Key.notify) else { return }
        var programs: [ReviewNotifier.Program] = []
        for app in appsToReview {
            for path in app.paths where !notified.contains(path) {
                guard let check = checks[path], verdict(check) == .needsReview else { continue }
                notified.insert(path)
                programs.append(.init(name: app.name, path: path))
            }
        }
        let notices = notifier.add(programs, now: Date())
        guard !notices.isEmpty else { return }
        Task {
            let center = UNUserNotificationCenter.current()
            guard (try? await center.requestAuthorization(options: [.alert, .sound])) == true else { return }
            for n in notices {
                let content = UNMutableNotificationContent()
                content.title = n.title
                content.body = n.body
                try? await center.add(UNNotificationRequest(identifier: n.id, content: content, trigger: nil))
            }
        }
    }

    // MARK: Ask AI

    var askServices: [AskAI.Service] {
        switch defaults.string(forKey: Key.askService) {
        case "chatgpt": [.chatGPT]
        case "both": [.claude, .chatGPT]
        default: [.claude]
        }
    }

    /// The question about one executable, shown before any button is pressed.
    func question(app: AppTraffic, check: SignatureCheck) -> String {
        let remotes = hour.map { AppTrafficText.remotes(of: app.app, in: $0) } ?? []
        // Addresses of this Mac tell a chat nothing.
        let addresses = remotes.filter { r in
            guard r.via != .router, let a = r.address else { return false }
            return !(a.hasPrefix("127.") || a == "::1" || a.hasPrefix("::ffff:127."))
        }.prefix(20).map {
            AskAI.Address(address: $0.address ?? "", place: $0.place.map { $0.country ?? AppTrafficText.placeName($0) }, bytesOut: $0.bytesOut)
        }
        let subject = AskAI.Subject(name: app.name, path: check.path, signature: check.status, signer: check.signer,
                                    notarized: check.notarized, sha256: check.sha256,
                                    virusTotal: cache.records[check.sha256]?.virusTotal, addresses: Array(addresses))
        return AskAI.question(for: subject, includeAddresses: flag(Key.askAddresses))
    }

    /// Opens the chat; for a service that ignores `q`, copies the question
    /// first (U3). Returns a line for the window.
    func ask(_ service: AskAI.Service, question: String) -> String? {
        if service.copyFallback {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(question, forType: .string)
            if let base = URL(string: service.base) { NSWorkspace.shared.open(base) }
            return "Question copied: paste it with ⌘V."
        }
        NSWorkspace.shared.open(AskAI.url(question: question, service: service))
        return nil
    }

    // MARK: This Mac on the map

    /// The point of this Mac: the city set in Settings, else the time zone's.
    var thisMac: TimeZonePlace? {
        if let name = defaults.string(forKey: Key.thisMac), let p = MapProjection.knownPlaces.first(where: { $0.name == name }) {
            return p
        }
        return MapProjection.place(forTimeZone: TimeZone.current.identifier)
    }
}
