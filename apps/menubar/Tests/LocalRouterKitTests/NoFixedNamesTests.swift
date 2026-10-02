// ADR 04, T6, I4: no Swift source writes a fixed CLI name, help URL or note
// name into a text. They come from `Instance`, so the dev app never tells a
// person or an agent to use the release. The scan covers every source file,
// so a new text is checked without a new test case.

import Foundation
import XCTest

final class NoFixedNamesTests: XCTestCase {
    private let words = ["localrouter ", "localrouterd", "router.localhost", "proxy.localhost", "LocalRouter.md", "LocalRouter.app"]

    /// Lines that may name these on purpose, with the reason.
    private let allowed: [(file: String, contains: String)] = [
        // The help page's host name; the port is added by AgentHelp.url.
        ("AgentHelp.swift", "public static let host = \"router.localhost\""),
        // The note's file name inside every bundle; the link in ~/.claude is per instance.
        ("CLIInstaller.swift", "public static let claudeNote = \"Contents/Resources/LocalRouter.md\""),
        // The rule itself.
        ("Instance.swift", "public var daemonProgram: String { \"localrouterd\\(suffix)\" }"),
        // The release DMG; the updater runs only in the release (Updater.disabledReason).
        ("Updater.swift", "SRC=\"$MOUNT/LocalRouter.app\""),
    ]

    func testNoSourceWritesAFixedInstanceName() throws {
        let sources = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // NoFixedNamesTests.swift
            .deletingLastPathComponent() // LocalRouterKitTests
            .deletingLastPathComponent() // Tests
            .appendingPathComponent("Sources")
        let files = try XCTUnwrap(FileManager.default.enumerator(at: sources, includingPropertiesForKeys: nil))
            .compactMap { $0 as? URL }.filter { $0.pathExtension == "swift" }
        XCTAssertGreaterThan(files.count, 10, "the scan found too few files")
        var found: [String] = []
        for file in files {
            let lines = try String(contentsOf: file, encoding: .utf8).components(separatedBy: "\n")
            for (n, line) in lines.enumerated() {
                let code = line.trimmingCharacters(in: .whitespaces)
                if code.hasPrefix("//") || !words.contains(where: line.contains) { continue }
                if allowed.contains(where: { file.lastPathComponent == $0.file && line.contains($0.contains) }) { continue }
                found.append("\(file.lastPathComponent):\(n + 1): \(code)")
            }
        }
        XCTAssertEqual(found, [], "fixed names; use Instance instead")
    }
}
