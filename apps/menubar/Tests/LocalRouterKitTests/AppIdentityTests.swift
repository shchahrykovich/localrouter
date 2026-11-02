import Foundation
import XCTest
@testable import LocalRouterKit

// ADR 11, T9: the signature statuses and the SHA-256, on real binaries made
// in the test with codesign.
final class AppIdentityTests: XCTestCase {
    private var root: URL!
    private let fm = FileManager.default

    override func setUpWithError() throws {
        root = fm.temporaryDirectory.appendingPathComponent("lr-ident-\(UUID().uuidString)")
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? fm.removeItem(at: root)
    }

    @discardableResult
    private func run(_ tool: String, _ args: [String]) throws -> String {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: tool)
        p.arguments = args
        let out = Pipe()
        p.standardOutput = out
        p.standardError = out
        try p.run()
        let data = out.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        let text = String(decoding: data, as: UTF8.self)
        XCTAssertEqual(p.terminationStatus, 0, "\(tool) \(args): \(text)")
        return text
    }

    /// One architecture of /usr/bin/true, signed ad hoc: every byte of it
    /// before the signature is covered by a page hash.
    private func adhocTrue(_ name: String) throws -> String {
        let path = root.appendingPathComponent(name).path
        let arch = try run("/usr/bin/lipo", ["-archs", "/usr/bin/true"]).split(separator: " ").map {
            $0.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        let thin = arch.contains("arm64e") ? "arm64e" : "x86_64"
        try run("/usr/bin/lipo", ["/usr/bin/true", "-thin", thin, "-output", path])
        try run("/usr/bin/codesign", ["-s", "-", "-f", path])
        return path
    }

    func testAnAppleProgramIsApple() throws {
        let r = try AppIdentity.check(path: "/bin/ls")
        XCTAssertEqual(r.status, .apple)
        XCTAssertNil(r.notarized)
        XCTAssertEqual(r.sha256.count, 64)
    }

    func testAnAdhocCopyIsAdhoc() throws {
        let path = try adhocTrue("adhoc")
        XCTAssertEqual(try AppIdentity.check(path: path).status, .adhoc)
    }

    func testARemovedSignatureIsUnsigned() throws {
        let path = try adhocTrue("unsigned")
        try run("/usr/bin/codesign", ["--remove-signature", path])
        XCTAssertEqual(try AppIdentity.check(path: path).status, .unsigned)
    }

    func testOneChangedByteIsBroken() throws {
        let path = try adhocTrue("broken")
        var data = try Data(contentsOf: URL(fileURLWithPath: path))
        data[data.count / 4] ^= 0xff
        try data.write(to: URL(fileURLWithPath: path))
        let r = try AppIdentity.check(path: path)
        XCTAssertEqual(r.status, .broken)
        XCTAssertNotNil(r.failure)
    }

    func testSha256EqualsShasum() throws {
        let path = try adhocTrue("hash")
        // More than one 1 MB piece, so the pieces are joined right.
        let big = root.appendingPathComponent("big").path
        var data = Data(count: 2_500_000)
        data[1_048_576] = 7
        try data.write(to: URL(fileURLWithPath: big))
        for file in [path, big] {
            let expected = try run("/usr/bin/shasum", ["-a", "256", file]).split(separator: " ").first.map(String.init)
            XCTAssertEqual(try AppIdentity.sha256(path: file), expected)
        }
    }

    func testAMissingFileThrows() {
        XCTAssertThrowsError(try AppIdentity.check(path: root.appendingPathComponent("nothing").path))
    }
}
