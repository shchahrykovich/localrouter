// VirusTotal by hash, with the user's own key (ADR 11, change 3, 3.2 and
// 3.3). A lookup sends the SHA-256 only: a GET with no body (I6). A file
// leaves the Mac only through `upload(file:confirmation:)`, and only the
// confirm dialog can make the confirmation (I9). The key lives in the
// Keychain only (I8).

import Foundation
import Security

public enum VirusTotal {
    public static let base = URL(string: "https://www.virustotal.com/api/v3")!
    /// Where a user gets a free key.
    public static let keyPage = URL(string: "https://www.virustotal.com/gui/my-apikey")!
    /// Files up to this size go in one POST; larger ones need an upload URL.
    public static let directUploadLimit = 32 * 1024 * 1024
    /// The largest file VirusTotal takes.
    public static let uploadLimit = 650 * 1024 * 1024
    /// The text of the confirm dialog. The dialog shows exactly this before
    /// it makes an `UploadConfirmation`.
    public static let uploadWarning =
        "VirusTotal shares uploaded files with security companies. Do not upload a file that holds your own or your company's data."

    /// An Apple program is never sent to VirusTotal (I7).
    public static func shouldLookUp(_ status: SignatureStatus) -> Bool {
        status != .apple
    }
}

/// What VirusTotal knows about one hash.
public struct VirusTotalReport: Codable, Equatable, Sendable {
    public enum State: String, Codable, Sendable {
        /// VirusTotal has analysed this file.
        case found
        /// "VirusTotal does not know this file" (404).
        case unknown
    }

    public var state: State
    public var malicious: Int
    public var suspicious: Int
    public var undetected: Int
    public var harmless: Int
    /// `popular_threat_classification.suggested_threat_label`.
    public var label: String?
    public var firstSubmission: Date?

    public init(state: State, malicious: Int = 0, suspicious: Int = 0, undetected: Int = 0, harmless: Int = 0,
                label: String? = nil, firstSubmission: Date? = nil) {
        self.state = state
        self.malicious = malicious
        self.suspicious = suspicious
        self.undetected = undetected
        self.harmless = harmless
        self.label = label
        self.firstSubmission = firstSubmission
    }

    public static let unknown = VirusTotalReport(state: .unknown)

    /// Engines that gave a verdict: "3 of 72 engines".
    public var engines: Int { malicious + suspicious + undetected + harmless }
}

public enum VirusTotalError: Error, LocalizedError, Equatable {
    /// 429, or the local limiter: try again after the next minute.
    case rateLimited
    /// The local daily limit of 500 lookups is used up.
    case dailyLimit
    case noKey
    /// 401 or 403.
    case badKey
    case tooLarge(Int)
    case badResponse(Int, String)

    public var errorDescription: String? {
        switch self {
        case .rateLimited: "VirusTotal asks to wait. The lookup runs again after the next minute."
        case .dailyLimit: "The 500 lookups of today are used. Lookups start again tomorrow."
        case .noKey: "Save a VirusTotal API key in Settings > Traffic first."
        case .badKey: "VirusTotal did not accept the API key."
        case let .tooLarge(bytes): "The file has \(bytes / 1_000_000) MB; VirusTotal takes files up to 650 MB."
        case let .badResponse(code, text): "VirusTotal answered \(code): \(text)"
        }
    }
}

/// Proof that the user read `VirusTotal.uploadWarning` and pressed Upload
/// for this file.
///
/// REVIEW CHECKPOINT (ADR 11, I9): the only way to make one is
/// `UploadConfirmation.userConfirmed(warningShown:file:)`, and the only
/// caller of `userConfirmed(` must be the Upload button of the confirm
/// dialog in the app. `grep -rn "userConfirmed(" apps/menubar/Sources` must
/// find that one call and this definition.
public struct UploadConfirmation: Sendable, Equatable {
    public let file: URL

    private init(file: URL) {
        self.file = file
    }

    /// Call this only from the confirm dialog's Upload button. Nil when the
    /// dialog did not show the warning text.
    public static func userConfirmed(warningShown: String, file: URL) -> UploadConfirmation? {
        guard warningShown == VirusTotal.uploadWarning else { return nil }
        return UploadConfirmation(file: file)
    }
}

public struct VirusTotalClient: Sendable {
    public var apiKey: String
    public var session: URLSession
    public var base: URL

    public init(apiKey: String, session: URLSession = .shared, base: URL = VirusTotal.base) {
        self.apiKey = apiKey
        self.session = session
        self.base = base
    }

    /// `GET /files/{sha256}`: the hash only, never the file (I6).
    public func lookup(sha256: String) async throws -> VirusTotalReport {
        guard sha256.count == 64, sha256.allSatisfy(\.isHexDigit) else {
            throw VirusTotalError.badResponse(0, "not a SHA-256: \(sha256)")
        }
        var request = URLRequest(url: base.appendingPathComponent("files").appendingPathComponent(sha256.lowercased()))
        request.httpMethod = "GET"
        request.setValue(apiKey, forHTTPHeaderField: "x-apikey")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        let (data, response) = try await session.data(for: request)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        switch code {
        case 200: return try Self.parse(data)
        case 404: return .unknown
        default: throw Self.error(code, data)
        }
    }

    /// Send the file to VirusTotal. Returns the analysis id. Look the hash
    /// up again a minute later for the result.
    public func upload(file: URL, confirmation: UploadConfirmation) async throws -> String {
        guard confirmation.file.standardizedFileURL == file.standardizedFileURL else {
            throw VirusTotalError.badResponse(0, "the confirmation is for another file")
        }
        let size = (try? FileManager.default.attributesOfItem(atPath: file.path)[.size] as? Int) ?? 0
        guard size <= VirusTotal.uploadLimit else { throw VirusTotalError.tooLarge(size) }
        var target = base.appendingPathComponent("files")
        if size > VirusTotal.directUploadLimit {
            target = try await uploadURL()
        }
        let boundary = "lr-\(UUID().uuidString)"
        var request = URLRequest(url: target)
        request.httpMethod = "POST"
        request.setValue(apiKey, forHTTPHeaderField: "x-apikey")
        request.setValue("multipart/form-data; boundary=\(boundary)", forHTTPHeaderField: "Content-Type")
        let body = try Self.multipart(file: file, boundary: boundary)
        let (data, response) = try await session.upload(for: request, from: body)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard code == 200 else { throw Self.error(code, data) }
        struct Reply: Decodable {
            struct Inner: Decodable { var id: String }
            var data: Inner
        }
        guard let id = try? JSONDecoder().decode(Reply.self, from: data).data.id else {
            throw VirusTotalError.badResponse(code, "no analysis id")
        }
        return id
    }

    /// `GET /files/upload_url`, for files above 32 MB.
    func uploadURL() async throws -> URL {
        var request = URLRequest(url: base.appendingPathComponent("files").appendingPathComponent("upload_url"))
        request.setValue(apiKey, forHTTPHeaderField: "x-apikey")
        let (data, response) = try await session.data(for: request)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard code == 200 else { throw Self.error(code, data) }
        struct Reply: Decodable { var data: String }
        guard let text = try? JSONDecoder().decode(Reply.self, from: data).data, let url = URL(string: text),
              url.scheme == "https" else {
            throw VirusTotalError.badResponse(code, "no upload URL")
        }
        return url
    }

    static func multipart(file: URL, boundary: String) throws -> Data {
        var body = Data()
        let name = file.lastPathComponent.replacingOccurrences(of: "\"", with: "")
        body.append(Data("--\(boundary)\r\nContent-Disposition: form-data; name=\"file\"; filename=\"\(name)\"\r\n".utf8))
        body.append(Data("Content-Type: application/octet-stream\r\n\r\n".utf8))
        body.append(try Data(contentsOf: file))
        body.append(Data("\r\n--\(boundary)--\r\n".utf8))
        return body
    }

    static func error(_ code: Int, _ data: Data) -> VirusTotalError {
        switch code {
        case 429: return .rateLimited
        case 401, 403: return .badKey
        default: return .badResponse(code, String(decoding: data.prefix(300), as: UTF8.self))
        }
    }

    /// The parts of `GET /files/{id}` the details window shows.
    public static func parse(_ data: Data) throws -> VirusTotalReport {
        struct Stats: Decodable {
            var malicious: Int?
            var suspicious: Int?
            var undetected: Int?
            var harmless: Int?
        }
        struct Classification: Decodable {
            var suggested_threat_label: String?
        }
        struct Attributes: Decodable {
            var last_analysis_stats: Stats?
            var popular_threat_classification: Classification?
            var first_submission_date: Double?
        }
        struct Inner: Decodable { var attributes: Attributes }
        struct Reply: Decodable { var data: Inner }
        let a: Attributes
        do {
            a = try JSONDecoder().decode(Reply.self, from: data).data.attributes
        } catch {
            throw VirusTotalError.badResponse(200, "not a file report")
        }
        let s = a.last_analysis_stats
        return VirusTotalReport(
            state: .found,
            malicious: s?.malicious ?? 0,
            suspicious: s?.suspicious ?? 0,
            undetected: s?.undetected ?? 0,
            harmless: s?.harmless ?? 0,
            label: a.popular_threat_classification?.suggested_threat_label,
            firstSubmission: a.first_submission_date.map { Date(timeIntervalSince1970: $0) })
    }
}

/// The public API's limits: 4 lookups a minute, 500 a day (UTC days, as
/// VirusTotal counts them). Codable, so the count of today survives a
/// restart of the app.
public struct RateLimiter: Codable, Equatable, Sendable {
    public static let perMinute = 4
    public static let perDay = 500

    public enum Decision: Equatable, Sendable {
        case now
        /// The minute is full: try again at this time.
        case wait(until: Date)
        /// Today's 500 are used.
        case refused
    }

    /// The times of the lookups of the last minute.
    var recent: [Date] = []
    /// The UTC day (`2026-11-02`) and its count.
    var day: String = ""
    var count: Int = 0

    public init() {}

    public func decide(now: Date) -> Decision {
        if Self.dayKey(now) == day && count >= Self.perDay { return .refused }
        let inMinute = recent.filter { now.timeIntervalSince($0) < 60 }
        if inMinute.count >= Self.perMinute, let oldest = inMinute.min() {
            return .wait(until: oldest.addingTimeInterval(60))
        }
        return .now
    }

    /// Count one lookup made at `now`.
    public mutating func record(now: Date) {
        recent = recent.filter { now.timeIntervalSince($0) < 60 } + [now]
        let key = Self.dayKey(now)
        if key != day {
            day = key
            count = 0
        }
        count += 1
    }

    /// "112 of 500 lookups today".
    public func lookupsToday(now: Date) -> Int {
        Self.dayKey(now) == day ? count : 0
    }

    static func dayKey(_ date: Date) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }
}

/// One generic password in the login Keychain. The VirusTotal key is kept
/// only here (I8): never in config.json, UserDefaults, the cache file, a log
/// or a socket reply.
public struct KeychainStore: Sendable {
    public let service: String
    public let account: String

    public init(service: String, account: String) {
        self.service = service
        self.account = account
    }

    /// The VirusTotal key of this instance: the service is named by the
    /// instance rule (ADR 04), so `-dev` has its own item.
    public static func virusTotal(_ instance: Instance = .current) -> KeychainStore {
        KeychainStore(service: "\(instance.bundleID).virustotal", account: "api-key")
    }

    public enum Failure: Error, LocalizedError, Equatable {
        case keychain(OSStatus)

        public var errorDescription: String? {
            switch self {
            case let .keychain(status):
                "The Keychain refused: \((SecCopyErrorMessageString(status, nil) as String?) ?? "error \(status)")"
            }
        }
    }

    private var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: account]
    }

    public func read() -> String? {
        var q = query
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &item) == errSecSuccess, let data = item as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Save or replace the value. An empty value deletes it.
    public func save(_ value: String) throws {
        let value = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if value.isEmpty { return try delete() }
        let data = Data(value.utf8)
        let update = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if update == errSecSuccess { return }
        guard update == errSecItemNotFound else { throw Failure.keychain(update) }
        var add = query
        add[kSecValueData as String] = data
        add[kSecAttrLabel as String] = service
        let status = SecItemAdd(add as CFDictionary, nil)
        guard status == errSecSuccess else { throw Failure.keychain(status) }
    }

    public func delete() throws {
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw Failure.keychain(status) }
    }
}
