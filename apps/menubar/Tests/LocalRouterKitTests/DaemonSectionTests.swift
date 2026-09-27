// ADR 04, T12, I12: when Settings shows the Daemon section.

import Foundation
import XCTest
@testable import LocalRouterKit

final class DaemonSectionTests: XCTestCase {
    private typealias Ports = DaemonSection.Ports
    private let standard = Ports(http: 80, https: 443)
    private let dev = Ports(http: 7080, https: 7443)

    func testTheReleaseOnItsPortsShowsNothing() {
        XCTAssertEqual(DaemonSection.decide(bound: standard, file: standard, hasProblem: false),
                       DaemonSection(visible: false, pendingPorts: nil))
    }

    func testOtherPortsShowTheSection() {
        XCTAssertEqual(DaemonSection.decide(bound: dev, file: dev, hasProblem: false),
                       DaemonSection(visible: true, pendingPorts: nil))
    }

    func testAProblemShowsTheSection() {
        XCTAssertTrue(DaemonSection.decide(bound: standard, file: standard, hasProblem: true).visible)
        XCTAssertTrue(DaemonSection.decide(bound: nil, file: nil, hasProblem: true).visible)
    }

    func testAHandEditWaitsForARestart() {
        let edited = Ports(http: 7081, https: 7444)
        XCTAssertEqual(DaemonSection.decide(bound: dev, file: edited, hasProblem: false),
                       DaemonSection(visible: true, pendingPorts: edited))
        // The release too: it shows the section although it runs on 80 and 443.
        XCTAssertEqual(DaemonSection.decide(bound: standard, file: edited, hasProblem: false).pendingPorts, edited)
    }

    func testPortZeroIsNeverPending() {
        XCTAssertNil(DaemonSection.decide(bound: Ports(http: 53001, https: 53002), file: Ports(http: 0, https: 0),
                                          hasProblem: false).pendingPorts)
    }

    func testFilePortsUseTheInstanceDefaultForAMissingField() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("lr-cfg-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let file = dir.appendingPathComponent("config.json")
        let devInstance = try Instance(suffix: "-dev")

        XCTAssertNil(DaemonSection.filePorts(configURL: file, instance: devInstance), "no file")
        try #"{"https_port": 7444}"#.write(to: file, atomically: true, encoding: .utf8)
        XCTAssertEqual(DaemonSection.filePorts(configURL: file, instance: devInstance), Ports(http: 7080, https: 7444))
        XCTAssertEqual(DaemonSection.filePorts(configURL: file, instance: .release), Ports(http: 80, https: 7444))
        try "{ not json".write(to: file, atomically: true, encoding: .utf8)
        XCTAssertNil(DaemonSection.filePorts(configURL: file, instance: devInstance))
    }

    func testTheRestartCommandNamesTheInstancesDaemon() throws {
        XCTAssertEqual(DaemonSection.restartCommand(for: try Instance(suffix: "-dev")),
                       "launchctl kickstart -k gui/$(id -u)/dev.localrouter.app-dev.daemon")
    }
}
