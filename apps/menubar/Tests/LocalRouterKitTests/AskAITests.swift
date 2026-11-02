import Foundation
import XCTest
@testable import LocalRouterKit

// ADR 11, T12.
final class AskAITests: XCTestCase {
    private let home = "/Users/alice"
    private let sha = "9f2c" + String(repeating: "0", count: 54) + "a8e41a"

    private func canvas(addresses: [AskAI.Address]? = nil) -> AskAI.Subject {
        AskAI.Subject(
            name: "helperd",
            path: "/Users/alice/Library/Application Support/.helperd/helperd",
            startsAs: "LaunchAgent com.helperd.agent",
            signature: .unsigned,
            sha256: sha,
            virusTotal: VirusTotalReport(state: .found, malicious: 3, undetected: 69, label: "Trojan.MacOS.Agent"),
            addresses: addresses ?? [AskAI.Address(address: "203.0.113.45", place: "Netherlands", bytesOut: 4_100_000)])
    }

    // The canvas example. The canvas wrote the hash short; the build writes
    // it whole and shortens it only when the URL would be too long.
    func testTheCanvasExample() {
        let q = AskAI.question(for: canvas(), includeAddresses: true, home: home)
        XCTAssertEqual(q, """
        What is the macOS program "helperd"? It runs from ~/Library/Application Support/.helperd/helperd and starts as \
        LaunchAgent com.helperd.agent. It has no code signature. SHA-256 \(sha). VirusTotal: 3 of 72 engines, \
        "Trojan.MacOS.Agent". It sent 4.1 MB to 203.0.113.45 (Netherlands). Is it safe, and how do I remove it?
        """)
    }

    // I10: the home folder is ~, the user name appears nowhere.
    func testTheHomeFolderIsTilde() {
        var s = canvas()
        s.startsAs = "LaunchAgent /Users/alice/Library/LaunchAgents/com.helperd.agent.plist"
        s.name = "tool"
        let q = AskAI.question(for: s, includeAddresses: true, home: home)
        XCTAssertFalse(q.contains("alice"), q)
        XCTAssertTrue(q.contains("~/Library/LaunchAgents/com.helperd.agent.plist"), q)
        // Another user's folder does not name that user either.
        s.path = "/Users/bob/bin/tool"
        XCTAssertFalse(AskAI.question(for: s, includeAddresses: true, home: home).contains("bob"))
        // A home that is a prefix of another name is not cut into it.
        s.path = "/Users/alicex/bin/tool"
        XCTAssertFalse(AskAI.question(for: s, includeAddresses: true, home: home).contains("~x"))
    }

    func testAddressesOnlyWithTheSetting() {
        let q = AskAI.question(for: canvas(), includeAddresses: false, home: home)
        XCTAssertFalse(q.contains("203.0.113.45"))
        XCTAssertFalse(q.contains("It sent"))
    }

    // I10: 500 addresses still give a URL under 2,000 characters: the list
    // goes first, then the hash is shortened.
    func testTheURLStaysShort() {
        let many = (0..<500).map { AskAI.Address(address: "198.51.100.\($0 % 250)", place: "United States", bytesOut: 1_000_000) }
        let q = AskAI.question(for: canvas(addresses: many), includeAddresses: true, home: home)
        XCTAssertFalse(q.contains("It sent"))
        XCTAssertTrue(q.contains(sha), "the address list goes first, the hash stays when it fits")
        for service in AskAI.Service.allCases {
            XCTAssertLessThan(AskAI.url(question: q, service: service).absoluteString.count, AskAI.maxURLLength)
        }
        var long = canvas()
        long.path = "/opt/" + String(repeating: "very-long-folder-name/", count: 70) + "tool"
        let q2 = AskAI.question(for: long, includeAddresses: true, home: home)
        for service in AskAI.Service.allCases {
            XCTAssertLessThan(AskAI.url(question: q2, service: service).absoluteString.count, AskAI.maxURLLength)
        }
    }

    func testTheLinks() {
        let q = "What is \"a&b\" #1?"
        XCTAssertEqual(AskAI.url(question: q, service: .claude).absoluteString,
                       "https://claude.ai/new?q=What%20is%20%22a%26b%22%20%231%3F")
        XCTAssertEqual(AskAI.url(question: q, service: .chatGPT).absoluteString,
                       "https://chatgpt.com/?q=What%20is%20%22a%26b%22%20%231%3F")
        let back = URLComponents(url: AskAI.url(question: q, service: .claude), resolvingAgainstBaseURL: false)?
            .queryItems?.first { $0.name == "q" }?.value
        XCTAssertEqual(back, q)
    }

    func testSignatureSentences() {
        var s = canvas()
        s.signature = .developerID
        s.signer = "Developer ID Application: Google LLC (EQHXZ8M8AV)"
        s.notarized = true
        XCTAssertTrue(AskAI.question(for: s, includeAddresses: false, home: home)
            .contains("It is signed by Developer ID Application: Google LLC (EQHXZ8M8AV) and notarized by Apple."))
        s.virusTotal = .unknown
        XCTAssertTrue(AskAI.question(for: s, includeAddresses: false, home: home).contains("VirusTotal does not know this file."))
    }
}
