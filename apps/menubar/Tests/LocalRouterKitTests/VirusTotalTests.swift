import Foundation
import XCTest
@testable import LocalRouterKit

/// Answers every request of its session and records it.
final class StubProtocol: URLProtocol, @unchecked Sendable {
    struct Seen {
        var method: String
        var url: URL
        var headers: [String: String]
        var bodyBytes: Int
    }

    nonisolated(unsafe) static var answer: (URLRequest) -> (Int, Data) = { _ in (500, Data()) }
    nonisolated(unsafe) static var seen: [Seen] = []
    static let lock = NSLock()

    static func reset(_ answer: @escaping (URLRequest) -> (Int, Data)) {
        lock.withLock {
            self.answer = answer
            seen = []
        }
    }

    static var requests: [Seen] { lock.withLock { seen } }

    static func session() -> URLSession {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StubProtocol.self]
        return URLSession(configuration: config)
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        var bytes = request.httpBody?.count ?? 0
        if let stream = request.httpBodyStream {
            stream.open()
            var buffer = [UInt8](repeating: 0, count: 65_536)
            while stream.hasBytesAvailable {
                let n = stream.read(&buffer, maxLength: buffer.count)
                if n <= 0 { break }
                bytes += n
            }
            stream.close()
        }
        let (code, data) = Self.lock.withLock {
            Self.seen.append(Seen(method: request.httpMethod ?? "GET", url: request.url!,
                                  headers: request.allHTTPHeaderFields ?? [:], bodyBytes: bytes))
            return Self.answer(request)
        }
        let response = HTTPURLResponse(url: request.url!, statusCode: code, httpVersion: "HTTP/1.1", headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}

// ADR 11, T10.
final class VirusTotalTests: XCTestCase {
    private let sha = String(repeating: "ab", count: 32)
    private let found = """
    {"data":{"id":"x","attributes":{"last_analysis_stats":{"malicious":3,"suspicious":1,"undetected":60,"harmless":8},
     "popular_threat_classification":{"suggested_threat_label":"Trojan.MacOS.Agent"},"first_submission_date":1700000000}}}
    """

    private func client() -> VirusTotalClient {
        VirusTotalClient(apiKey: "test-key", session: StubProtocol.session())
    }

    func testLookupIsAGetOfTheHashWithTheKeyAndNoBody() async throws {
        StubProtocol.reset { _ in (200, Data(self.found.utf8)) }
        _ = try await client().lookup(sha256: sha)
        let seen = StubProtocol.requests
        XCTAssertEqual(seen.count, 1)
        XCTAssertEqual(seen[0].method, "GET")
        XCTAssertEqual(seen[0].url.absoluteString, "https://www.virustotal.com/api/v3/files/\(sha)")
        XCTAssertEqual(seen[0].headers["x-apikey"], "test-key")
        XCTAssertEqual(seen[0].bodyBytes, 0)
    }

    func testAnswersAreParsed() async throws {
        StubProtocol.reset { _ in (200, Data(self.found.utf8)) }
        let r = try await client().lookup(sha256: sha)
        XCTAssertEqual(r.state, .found)
        XCTAssertEqual([r.malicious, r.suspicious, r.undetected, r.harmless, r.engines], [3, 1, 60, 8, 72])
        XCTAssertEqual(r.label, "Trojan.MacOS.Agent")
        XCTAssertEqual(r.firstSubmission, Date(timeIntervalSince1970: 1_700_000_000))

        StubProtocol.reset { _ in (404, Data(#"{"error":{"code":"NotFoundError"}}"#.utf8)) }
        let unknown = try await client().lookup(sha256: sha)
        XCTAssertEqual(unknown, .unknown)

        StubProtocol.reset { _ in (429, Data()) }
        do {
            _ = try await client().lookup(sha256: sha)
            XCTFail("429 must throw")
        } catch {
            XCTAssertEqual(error as? VirusTotalError, .rateLimited)
        }
        StubProtocol.reset { _ in (401, Data()) }
        do {
            _ = try await client().lookup(sha256: sha)
            XCTFail("401 must throw")
        } catch {
            XCTAssertEqual(error as? VirusTotalError, .badKey)
        }
    }

    func testNotAHashIsRefusedBeforeAnyRequest() async {
        StubProtocol.reset { _ in (200, Data()) }
        do {
            _ = try await client().lookup(sha256: "../../users/me")
            XCTFail("must throw")
        } catch {}
        XCTAssertEqual(StubProtocol.requests.count, 0)
    }

    // I7: an Apple program is never looked up.
    func testAppleProgramsAreNeverLookedUp() {
        XCTAssertFalse(VirusTotal.shouldLookUp(.apple))
        for status in SignatureStatus.allCases where status != .apple {
            XCTAssertTrue(VirusTotal.shouldLookUp(status))
        }
        let cache = AppChecksCache()
        XCTAssertFalse(cache.needsLookup(sha256: sha, status: .apple, now: Date(), force: true))
        XCTAssertTrue(cache.needsLookup(sha256: sha, status: .developerID, now: Date()))
    }

    func testTheFifthLookupInAMinuteWaitsAndThe501stInADayIsRefused() {
        var limiter = RateLimiter()
        let start = Date(timeIntervalSince1970: 1_800_000_000) // 2027-01-15 08:00 UTC
        for i in 0..<4 {
            XCTAssertEqual(limiter.decide(now: start.addingTimeInterval(Double(i))), .now)
            limiter.record(now: start.addingTimeInterval(Double(i)))
        }
        XCTAssertEqual(limiter.decide(now: start.addingTimeInterval(10)), .wait(until: start.addingTimeInterval(60)))
        XCTAssertEqual(limiter.decide(now: start.addingTimeInterval(61)), .now)
        XCTAssertEqual(limiter.lookupsToday(now: start), 4)

        var day = RateLimiter()
        for i in 0..<500 {
            day.record(now: start.addingTimeInterval(Double(i) * 20))
        }
        let later = start.addingTimeInterval(500 * 20)
        XCTAssertEqual(day.decide(now: later), .refused)
        XCTAssertEqual(day.lookupsToday(now: later), 500)
        // The next UTC day starts again.
        XCTAssertEqual(day.decide(now: start.addingTimeInterval(24 * 3600)), .now)
        XCTAssertEqual(day.lookupsToday(now: start.addingTimeInterval(24 * 3600)), 0)
    }

    // I8: the key goes to the Keychain only; never to UserDefaults or the cache file.
    func testTheKeyIsOnlyInTheKeychain() throws {
        let store = KeychainStore(service: "lr-test-\(UUID().uuidString)", account: "api-key")
        defer { try? store.delete() }
        let key = "vt-test-key-\(UUID().uuidString)"
        do {
            try store.save(key)
        } catch {
            throw XCTSkip("no usable login Keychain here: \(error.localizedDescription)")
        }
        XCTAssertEqual(store.read(), key)

        let suite = "lr-test-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let trusted = TrustedPrograms(defaults: defaults)
        trusted.trust(sha256: sha, path: "/tmp/x")
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("lr-vt-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: folder) }
        var cache = AppChecksCache()
        cache.store(signature: SignatureCheck(path: "/tmp/x", status: .unsigned, sha256: sha), now: Date())
        cache.store(virusTotal: .unknown, sha256: sha, now: Date())
        try cache.save(to: folder)

        let file = try String(contentsOf: folder.appendingPathComponent(AppChecksCache.fileName), encoding: .utf8)
        XCTAssertFalse(file.contains(key))
        let all = defaults.dictionaryRepresentation().description
        XCTAssertFalse(all.contains(key))

        try store.save("")
        XCTAssertNil(store.read())
    }

    func testTheKeychainServiceIsNamedByTheInstance() throws {
        XCTAssertEqual(KeychainStore.virusTotal(.release).service, Instance.release.bundleID + ".virustotal")
        let dev = try Instance(suffix: "-dev")
        XCTAssertEqual(KeychainStore.virusTotal(dev).service, dev.bundleID + ".virustotal")
    }

    // I9: an upload needs the confirmation of the dialog, for this file.
    func testUploadNeedsTheConfirmationOfThisFile() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("lr-up-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let file = folder.appendingPathComponent("tool")
        try Data(repeating: 1, count: 1000).write(to: file)
        XCTAssertNil(UploadConfirmation.userConfirmed(warningShown: "Upload?", file: file))
        let ok = try XCTUnwrap(UploadConfirmation.userConfirmed(warningShown: VirusTotal.uploadWarning, file: file))

        StubProtocol.reset { _ in (200, Data(#"{"data":{"type":"analysis","id":"an-1"}}"#.utf8)) }
        let id = try await client().upload(file: file, confirmation: ok)
        XCTAssertEqual(id, "an-1")
        let seen = StubProtocol.requests
        XCTAssertEqual(seen.map(\.method), ["POST"])
        XCTAssertEqual(seen[0].url.absoluteString, "https://www.virustotal.com/api/v3/files")
        XCTAssertGreaterThan(seen[0].bodyBytes, 1000)

        let other = folder.appendingPathComponent("other")
        try Data([1]).write(to: other)
        StubProtocol.reset { _ in (200, Data()) }
        do {
            _ = try await client().upload(file: other, confirmation: ok)
            XCTFail("a confirmation is for one file")
        } catch {}
        XCTAssertEqual(StubProtocol.requests.count, 0)
    }

    func testALargeFileAsksForAnUploadURLFirst() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("lr-up-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let file = folder.appendingPathComponent("big")
        try Data(count: VirusTotal.directUploadLimit + 1).write(to: file)
        let ok = try XCTUnwrap(UploadConfirmation.userConfirmed(warningShown: VirusTotal.uploadWarning, file: file))
        StubProtocol.reset { req in
            if req.url?.path == "/api/v3/files/upload_url" {
                return (200, Data(#"{"data":"https://upload.example.test/u/1"}"#.utf8))
            }
            return (200, Data(#"{"data":{"id":"an-2"}}"#.utf8))
        }
        let id = try await client().upload(file: file, confirmation: ok)
        XCTAssertEqual(id, "an-2")
        XCTAssertEqual(StubProtocol.requests.map(\.method), ["GET", "POST"])
        XCTAssertEqual(StubProtocol.requests.last?.url.host, "upload.example.test")
    }
}
