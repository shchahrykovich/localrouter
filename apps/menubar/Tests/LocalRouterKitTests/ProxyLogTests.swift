// ADR 08, T13: where Open Proxy Log goes, and the state line.

import Foundation
import XCTest
@testable import LocalRouterKit

final class ProxyLogTests: XCTestCase {
    private final class Recorder: URLOpening, @unchecked Sendable {
        var opened: [(URL, URL?)] = []
        func open(_ url: URL, withApplicationAt app: URL?) async throws { opened.append((url, app)) }
    }

    private func log(enabled: Bool = true, current: String? = "proxy-20261002-093512.har", written: UInt64 = 912,
                     dropped: UInt64 = 0, error: String? = nil) -> ProxyLogStatus {
        ProxyLogStatus(enabled: enabled, folder: "/Users/me/Library/Logs/LocalRouter/proxy", url: "http://proxy.localhost",
                       fileMb: 20, fileRequests: 5000, keepFiles: 5, current: current, files: 3, written: written,
                       dropped: dropped, error: error)
    }

    func testChromeInstalledOpensTheViewerInChrome() async {
        let chrome = URL(fileURLWithPath: "/Applications/Google Chrome.app")
        let recorder = Recorder()
        let feedback = await ProxyLog.open(log(), chrome: chrome, opener: recorder)
        XCTAssertEqual(recorder.opened.first?.0, URL(string: "http://proxy.localhost"))
        XCTAssertEqual(recorder.opened.first?.1, chrome)
        XCTAssertEqual(feedback.title, "Opened the proxy log")
    }

    func testWithoutChromeTheDefaultBrowserOpensIt() async {
        let recorder = Recorder()
        _ = await ProxyLog.open(log(), chrome: nil, opener: recorder)
        XCTAssertEqual(recorder.opened.count, 1)
        XCTAssertNil(recorder.opened.first?.1)
    }

    func testTheStateLine() {
        XCTAssertEqual(ProxyLog.stateLine(log()), "Log: on, 912 requests in proxy-20261002-093512.har")
        XCTAssertEqual(ProxyLog.stateLine(log(written: 1)), "Log: on, 1 request in proxy-20261002-093512.har")
        XCTAssertEqual(ProxyLog.stateLine(log(current: nil, written: 0)), "Log: on, no request yet")
        XCTAssertEqual(ProxyLog.stateLine(log(enabled: false)), "Log: off")
        XCTAssertEqual(ProxyLog.stateLine(log(error: "disk full")), "Log: stopped, see the error")
        XCTAssertNil(ProxyLog.stateLine(nil), "a 1.4 daemon: no line")
    }

    func testProblemsNameTheErrorAndDroppedRecords() {
        XCTAssertEqual(ProxyLog.problems(log()), [])
        let p = ProxyLog.problems(log(dropped: 3, error: "No space left on device"))
        XCTAssertEqual(p.count, 2)
        XCTAssertTrue(p[0].contains("No space left on device"))
        XCTAssertTrue(p[1].hasPrefix("3 requests were not written"))
        XCTAssertTrue(ProxyLog.explanation(keepFiles: 5).contains("cookies and API keys"))
    }
}
