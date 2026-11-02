import Foundation
import XCTest
@testable import LocalRouterKit

// ADR 11, T11.
final class AppChecksTests: XCTestCase {
    private let a = String(repeating: "a", count: 64)
    private let b = String(repeating: "b", count: 64)

    private func vt(_ malicious: Int) -> VirusTotalReport {
        VirusTotalReport(state: .found, malicious: malicious, undetected: 70 - malicious)
    }

    func testNeedsReviewRule() {
        let review = { (s: SignatureStatus, v: VirusTotalReport?, adhoc: Bool) in
            AppChecks.needsReview(signature: s, virusTotal: v, warnAdhoc: adhoc, trusted: false)
        }
        XCTAssertTrue(review(.unsigned, nil, false))
        XCTAssertTrue(review(.broken, nil, false))
        XCTAssertTrue(review(.developerID, vt(2), false))
        XCTAssertFalse(review(.developerID, vt(1), false))
        XCTAssertFalse(review(.developerID, vt(0), false))
        XCTAssertFalse(review(.developerID, .unknown, false))
        XCTAssertFalse(review(.adhoc, nil, false))
        XCTAssertTrue(review(.adhoc, nil, true))
        XCTAssertFalse(review(.apple, nil, true))
        // One engine is amber, not a review.
        XCTAssertEqual(AppChecks.verdict(signature: .developerID, virusTotal: vt(1), warnAdhoc: false, trusted: false), .caution)
        XCTAssertEqual(AppChecks.verdict(signature: .apple, virusTotal: nil, warnAdhoc: false, trusted: false), .apple)
        XCTAssertEqual(AppChecks.verdict(signature: .unsigned, virusTotal: nil, warnAdhoc: false, trusted: true), .trusted)
        XCTAssertFalse(AppChecks.needsReview(signature: .unsigned, virusTotal: nil, warnAdhoc: false, trusted: true))
    }

    // I12: a trusted mark is for one SHA-256, not for the path.
    func testTrustIsBySha256() throws {
        let suite = "lr-test-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let trusted = TrustedPrograms(defaults: defaults)
        trusted.trust(sha256: a, path: "/tmp/u")
        XCTAssertTrue(trusted.isTrusted(sha256: a))
        XCTAssertTrue(trusted.isTrusted(sha256: a.uppercased()))
        XCTAssertFalse(trusted.isTrusted(sha256: b), "the same path with another hash is not trusted")
        XCTAssertEqual(trusted.all, [a: "/tmp/u"])
        trusted.untrust(sha256: a)
        XCTAssertFalse(trusted.isTrusted(sha256: a))
        trusted.trust(sha256: b, path: "/tmp/u")
        trusted.removeAll()
        XCTAssertEqual(trusted.all, [:])
    }

    func testALookupOlderThanSevenDaysIsDoneAgain() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        var cache = AppChecksCache()
        cache.store(signature: SignatureCheck(path: "/x", status: .developerID, sha256: a), now: now)
        XCTAssertTrue(cache.needsLookup(sha256: a, status: .developerID, now: now))
        cache.store(virusTotal: vt(0), sha256: a, now: now)
        XCTAssertFalse(cache.needsLookup(sha256: a, status: .developerID, now: now.addingTimeInterval(6 * 24 * 3600)))
        XCTAssertTrue(cache.needsLookup(sha256: a, status: .developerID, now: now.addingTimeInterval(7 * 24 * 3600)))
        // Check again.
        XCTAssertTrue(cache.needsLookup(sha256: a, status: .developerID, now: now, force: true))
    }

    func testTheCacheFileRoundTripsAndADamagedOneIsEmpty() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("lr-checks-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: folder) }
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        var cache = AppChecksCache()
        cache.store(signature: SignatureCheck(path: "/x", status: .developerID, signer: "Developer ID Application: X (T1)",
                                              teamID: "T1", notarized: true, sha256: a), now: now)
        cache.store(virusTotal: vt(3), sha256: a, now: now)
        cache.limiter.record(now: now)
        try cache.save(to: folder)
        XCTAssertEqual(AppChecksCache.load(from: folder), cache)

        try Data("{".utf8).write(to: folder.appendingPathComponent(AppChecksCache.fileName))
        XCTAssertEqual(AppChecksCache.load(from: folder), AppChecksCache())
        XCTAssertEqual(AppChecksCache.load(from: folder.appendingPathComponent("none")), AppChecksCache())
    }

    func testTheDefaultFolderIsTheInstancesCaches() throws {
        let dev = try Instance(suffix: "-dev")
        XCTAssertTrue(AppChecksCache.defaultFolder(dev).path.hasSuffix("/Library/Caches/\(dev.bundleID)"))
    }

    // G6: no Quit process for Apple programs.
    func testAppleProgramsCannotBeQuit() {
        XCTAssertFalse(AppChecks.canQuit(.apple))
        XCTAssertFalse(AppChecks.canQuit(nil))
        for s in SignatureStatus.allCases where s != .apple {
            XCTAssertTrue(AppChecks.canQuit(s))
        }
    }

    // G7: more than three within ten minutes give one summary.
    func testFiveProgramsAtOnceGiveOneSummary() {
        var notifier = ReviewNotifier()
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let programs = (1...5).map { ReviewNotifier.Program(name: "p\($0)", path: "/tmp/p\($0)") }
        let notices = notifier.add(programs, now: now)
        XCTAssertEqual(notices.count, 1)
        XCTAssertEqual(notices[0].id, ReviewNotifier.summaryID)
        XCTAssertEqual(notices[0].title, "5 programs need review")
    }

    func testOneByOneTheFourthBecomesTheSummary() {
        var notifier = ReviewNotifier()
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let p = { (i: Int) in [ReviewNotifier.Program(name: "p\(i)", path: "/tmp/p\(i)")] }
        XCTAssertEqual(notifier.add(p(1), now: now).map(\.id), ["/tmp/p1"])
        XCTAssertEqual(notifier.add(p(2), now: now.addingTimeInterval(60)).map(\.id), ["/tmp/p2"])
        XCTAssertEqual(notifier.add(p(3), now: now.addingTimeInterval(120)).map(\.id), ["/tmp/p3"])
        let fourth = notifier.add(p(4), now: now.addingTimeInterval(180))
        XCTAssertEqual(fourth.map(\.title), ["4 programs need review"])
        // After ten quiet minutes, single notifications again.
        XCTAssertEqual(notifier.add(p(5), now: now.addingTimeInterval(180 + 601)).map(\.id), ["/tmp/p5"])
        XCTAssertEqual(notifier.add([], now: now), [])
    }
}
