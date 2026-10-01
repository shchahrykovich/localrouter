// ADR 06, T14, I17, I18: the Chrome launcher with a fake NSWorkspace and a
// fake daemon. That Chrome really starts a new instance and honours
// --proxy-server is manual test M3.

import Foundation
import XCTest
@testable import LocalRouterKit

/// Records what would have been opened.
final class FakeOpener: AppOpener, @unchecked Sendable {
    let chrome: URL?
    var opened: [(URL, [String])] = []
    init(chrome: URL?) { self.chrome = chrome }
    func appURL(bundleID: String) -> URL? { bundleID == "com.google.Chrome" ? chrome : nil }
    func openNewInstance(_ app: URL, arguments: [String]) async throws { opened.append((app, arguments)) }
}

/// A daemon that answers from api/examples/get_proxy.reply.json.
final class FakeDaemon: ProxySettings, @unchecked Sendable {
    var reply: GetProxyResult
    var setConfigError: Error?
    var calls: [String] = []
    init(reply: GetProxyResult) { self.reply = reply }
    func proxy() async throws -> GetProxyResult {
        calls.append("get_proxy")
        return reply
    }
    func setConfig(_ p: SetConfigParams) async throws -> SetConfigResult {
        calls.append("set_config proxy_enabled=\(p.proxyEnabled.map(String.init) ?? "nil")")
        if let setConfigError { throw setConfigError }
        reply.enabled = p.proxyEnabled ?? reply.enabled
        let config = Config(version: 1, httpPort: 80, httpsPort: 443, fallback: true, allowLan: false, logSize: 1000)
        return SetConfigResult(config: config, restartNeeded: false)
    }
}

final class ChromeLauncherTests: XCTestCase {
    private let chrome = URL(fileURLWithPath: "/Applications/Google Chrome.app")

    private func example() throws -> GetProxyResult {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("api/examples/get_proxy.reply.json")
        let reply = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any]
        let result = try JSONSerialization.data(withJSONObject: reply?["result"] ?? [:])
        return try Api.decoder.decode(GetProxyResult.self, from: result)
    }

    func testNoChromeMeansNoItemAndNoLaunch() async throws {
        let opener = FakeOpener(chrome: nil)
        let daemon = FakeDaemon(reply: try example())
        let launcher = ChromeLauncher(opener: opener, daemon: daemon)
        XCTAssertFalse(launcher.chromeInstalled)
        let feedback = await launcher.launch()
        XCTAssertTrue(feedback.failed)
        XCTAssertTrue(opener.opened.isEmpty)
        XCTAssertTrue(daemon.calls.isEmpty, "nothing is asked of the daemon")
    }

    func testArgumentsAreTheDaemonChromeArgs() async throws {
        let opener = FakeOpener(chrome: chrome)
        let reply = try example()
        let launcher = ChromeLauncher(opener: opener, daemon: FakeDaemon(reply: reply))
        XCTAssertTrue(launcher.chromeInstalled)
        let feedback = await launcher.launch()
        XCTAssertFalse(feedback.failed, feedback.summary)
        XCTAssertEqual(feedback.title, "Chrome started with the proxy at 127.0.0.1:8877")
        XCTAssertEqual(opener.opened.count, 1)
        XCTAssertEqual(opener.opened.first?.0, chrome)
        XCTAssertEqual(opener.opened.first?.1, reply.chromeArgs, "I18: exactly get_proxy.chrome_args")
        XCTAssertTrue(feedback.detail.contains("not trusted"), "the notes of get_proxy are shown")
    }

    func testAProxyThatIsOffIsTurnedOnFirst() async throws {
        var reply = try example()
        reply.enabled = false
        let daemon = FakeDaemon(reply: reply)
        let opener = FakeOpener(chrome: chrome)
        _ = await ChromeLauncher(opener: opener, daemon: daemon).launch()
        XCTAssertEqual(daemon.calls, ["get_proxy", "set_config proxy_enabled=true", "get_proxy"])
        XCTAssertEqual(opener.opened.count, 1)
    }

    func testAFailedTurnOnStartsNoChrome() async throws {
        var reply = try example()
        reply.enabled = false
        let daemon = FakeDaemon(reply: reply)
        daemon.setConfigError = ApiError(code: "port_in_use", message: "port 8877 is in use by another program")
        let opener = FakeOpener(chrome: chrome)
        let feedback = await ChromeLauncher(opener: opener, daemon: daemon).launch()
        XCTAssertTrue(feedback.failed)
        XCTAssertTrue(feedback.detail.contains("8877"), feedback.detail)
        XCTAssertTrue(opener.opened.isEmpty)
    }

    func testChromesOwnProfileIsNeverUsed() async throws {
        XCTAssertTrue(ChromeLauncher.hasOwnProfile(try example().chromeArgs))
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        XCTAssertFalse(ChromeLauncher.hasOwnProfile(["--user-data-dir=\(home)/Library/Application Support/Google/Chrome"]))
        XCTAssertFalse(ChromeLauncher.hasOwnProfile(["--proxy-server=http://127.0.0.1:8877"]))
        var reply = try example()
        reply.chromeArgs = ["--proxy-server=http://127.0.0.1:8877"]
        let opener = FakeOpener(chrome: chrome)
        let feedback = await ChromeLauncher(opener: opener, daemon: FakeDaemon(reply: reply)).launch()
        XCTAssertTrue(feedback.failed)
        XCTAssertTrue(opener.opened.isEmpty, "without its own profile, Chrome is not started")
    }
}
