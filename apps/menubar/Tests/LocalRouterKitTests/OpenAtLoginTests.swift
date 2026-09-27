import Foundation
import ServiceManagement
import XCTest
@testable import LocalRouterKit

private final class FakeLoginItem: LoginItemService {
    var status: SMAppService.Status
    var failure: Error?
    var calls: [String] = []

    init(_ status: SMAppService.Status) { self.status = status }

    func register() throws {
        calls.append("register")
        if let failure { throw failure }
        status = .enabled
    }

    func unregister() throws {
        calls.append("unregister")
        if let failure { throw failure }
        status = .notRegistered
    }
}

final class OpenAtLoginTests: XCTestCase {
    func testOffByDefault() {
        let item = FakeLoginItem(.notRegistered)
        XCTAssertFalse(OpenAtLogin(service: item).isOn)
        XCTAssertNil(OpenAtLogin(service: item).note)
    }

    func testTurningOnRegisters() {
        let item = FakeLoginItem(.notRegistered)
        let login = OpenAtLogin(service: item)
        XCTAssertNil(login.set(true))
        XCTAssertEqual(item.calls, ["register"])
        XCTAssertTrue(login.isOn)
    }

    func testTurningOffUnregisters() {
        let item = FakeLoginItem(.enabled)
        let login = OpenAtLogin(service: item)
        XCTAssertNil(login.set(false))
        XCTAssertEqual(item.calls, ["unregister"])
        XCTAssertFalse(login.isOn)
    }

    func testSameStateDoesNothing() {
        let item = FakeLoginItem(.enabled)
        XCTAssertNil(OpenAtLogin(service: item).set(true))
        XCTAssertEqual(item.calls, [])
    }

    func testWaitingForApprovalIsOnWithANote() {
        let item = FakeLoginItem(.requiresApproval)
        let login = OpenAtLogin(service: item)
        XCTAssertTrue(login.isOn)
        XCTAssertEqual(login.note, "Allow LocalRouter in System Settings → General → Login Items.")
        // Turning it off must still unregister.
        XCTAssertNil(login.set(false))
        XCTAssertEqual(item.calls, ["unregister"])
    }

    private func freshDefaults() -> UserDefaults {
        let name = "OpenAtLoginTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        addTeardownBlock { UserDefaults.standard.removePersistentDomain(forName: name) }
        return defaults
    }

    func testFirstLaunchTurnsItOn() {
        let item = FakeLoginItem(.notRegistered)
        let defaults = freshDefaults()
        XCTAssertNil(OpenAtLogin(service: item).turnOnAtFirstLaunch(defaults))
        XCTAssertEqual(item.calls, ["register"])
        XCTAssertTrue(OpenAtLogin(service: item).isOn)
    }

    func testLaterLaunchesKeepTheUsersOff() {
        let item = FakeLoginItem(.notRegistered)
        let defaults = freshDefaults()
        let login = OpenAtLogin(service: item)
        login.turnOnAtFirstLaunch(defaults)
        XCTAssertNil(login.choose(false, defaults))
        login.turnOnAtFirstLaunch(defaults)
        XCTAssertEqual(item.calls, ["register", "unregister"])
        XCTAssertFalse(login.isOn)
    }

    func testUserChoiceBeforeFirstLaunchIsKept() {
        let item = FakeLoginItem(.notRegistered)
        let defaults = freshDefaults()
        let login = OpenAtLogin(service: item)
        XCTAssertNil(login.choose(false, defaults))
        login.turnOnAtFirstLaunch(defaults)
        XCTAssertEqual(item.calls, [])
    }

    func testFirstLaunchRetriesAfterARefusal() {
        let item = FakeLoginItem(.notRegistered)
        item.failure = NSError(domain: "test", code: 1, userInfo: [NSLocalizedDescriptionKey: "Operation not permitted"])
        let defaults = freshDefaults()
        let login = OpenAtLogin(service: item)
        XCTAssertNotNil(login.turnOnAtFirstLaunch(defaults))
        item.failure = nil
        XCTAssertNil(login.turnOnAtFirstLaunch(defaults))
        XCTAssertEqual(item.calls, ["register", "register"])
        XCTAssertTrue(login.isOn)
    }

    func testFailureIsReported() {
        let item = FakeLoginItem(.notRegistered)
        item.failure = NSError(domain: "test", code: 1, userInfo: [NSLocalizedDescriptionKey: "Operation not permitted"])
        let login = OpenAtLogin(service: item)
        XCTAssertEqual(login.set(true), "Could not turn on Open at login: Operation not permitted")
        XCTAssertFalse(login.isOn)
    }
}
