// ADR 04, T9, I1, I2, I11: the Swift names of every instance equal the names
// in api/instance-names.json, which the Rust copy is checked against too.

import Foundation
import XCTest
@testable import LocalRouterKit

final class InstanceTests: XCTestCase {
    private var table: [String: Any] {
        get throws {
            let url = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent() // InstanceTests.swift
                .deletingLastPathComponent() // LocalRouterKitTests
                .deletingLastPathComponent() // Tests
                .deletingLastPathComponent() // menubar
                .deletingLastPathComponent() // apps
                .appendingPathComponent("api/instance-names.json")
            return try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
        }
    }

    private func names(_ i: Instance) -> [String: AnyHashable] {
        [
            "app_name": i.appName, "bundle_id": i.bundleID, "daemon_label": i.daemonLabel,
            "daemon_program": i.daemonProgram, "cli": i.cli, "data_folder": i.dataFolder,
            "logs_folder": i.logsFolder, "caches_folder": i.cachesFolder, "note": i.note, "mcp_name": i.cli,
            "ca_name_prefix": i.caNamePrefix, "inspect_ca_name_prefix": i.inspectCaNamePrefix,
            "http_port": Int(i.defaultPorts.http), "https_port": Int(i.defaultPorts.https),
            "proxy_port": Int(i.defaultProxyPort),
            "default_help_url": i.defaultHelpURL,
        ]
    }

    func testEveryRowGivesItsNames() throws {
        let rows = try XCTUnwrap(table["rows"] as? [[String: Any]])
        XCTAssertGreaterThanOrEqual(rows.count, 3)
        for row in rows {
            let suffix = try XCTUnwrap(row["suffix"] as? String)
            let i = try Instance(suffix: suffix)
            let expected = names(i)
            for (key, value) in expected {
                XCTAssertEqual(row[key] as? AnyHashable, value, "suffix \(suffix), \(key)")
            }
            XCTAssertEqual(row.count, expected.count + 1, "suffix \(suffix): a column the Swift names do not check")
        }
    }

    func testTheReleaseKeepsItsNames() {
        let r = Instance.release
        XCTAssertEqual(r.bundleID, "dev.localrouter.app")
        XCTAssertEqual(r.daemonLabel, "dev.localrouter.app.daemon")
        XCTAssertEqual(BundleLayout.cli(for: r), "Contents/Helpers/localrouter")
        XCTAssertEqual(BundleLayout.daemon(for: r), "Contents/MacOS/localrouterd")
        XCTAssertEqual(r.note, "LocalRouter.md")
    }

    func testTwoInstancesShareNoName() throws {
        let rows = try XCTUnwrap(table["rows"] as? [[String: Any]])
        let instances = try rows.map { try Instance(suffix: $0["suffix"] as? String ?? "?") }
        for key in ["app_name", "bundle_id", "daemon_label", "daemon_program", "cli", "data_folder", "logs_folder", "caches_folder", "note"] {
            let values = Set(instances.map { names($0)[key] })
            XCTAssertEqual(values.count, instances.count, "\(key) is shared by two instances")
        }
    }

    func testSuffixRule() throws {
        let invalid = try XCTUnwrap(table["invalid"] as? [String])
        for bad in invalid {
            XCTAssertThrowsError(try Instance(suffix: bad), "\(bad) must be refused")
        }
        XCTAssertNoThrow(try Instance(suffix: "-dev"))
    }

    /// `swift run` has no Info.plist key: the release.
    func testNoInfoPlistKeyIsTheRelease() throws {
        let bundle = Bundle(for: InstanceTests.self)
        XCTAssertEqual(try Instance.of(bundle: bundle), .release)
    }

    func testABundleWithoutItsProgramsIsReported() throws {
        let dev = try Instance(suffix: "-dev")
        let app = FileManager.default.temporaryDirectory.appendingPathComponent("lr-bundle-\(UUID().uuidString).app")
        defer { try? FileManager.default.removeItem(at: app) }
        // A bundle built for the release, opened as -dev.
        for program in [BundleLayout.daemon(for: .release), BundleLayout.cli(for: .release)] {
            let url = app.appendingPathComponent(program)
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            FileManager.default.createFile(atPath: url.path, contents: Data(), attributes: [.posixPermissions: 0o755])
        }
        XCTAssertNil(Instance.release.bundleProblem(app))
        XCTAssertEqual(dev.bundleProblem(app)?.contains("localrouterd-dev is missing"), true)
    }
}
