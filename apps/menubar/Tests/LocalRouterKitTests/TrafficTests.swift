import Foundation
import XCTest
@testable import LocalRouterKit

final class TrafficTests: XCTestCase {
    private func http(_ host: String, status: UInt16 = 200, proxy: Bool = false) -> LogEntry {
        .http(time: 0, method: "GET", host: host, path: "/", status: status, durationMs: 1, route: nil,
              proxy: proxy ? ProxyTraffic(mode: "inspect") : nil, scripts: nil)
    }

    private let tcp = LogEntry.tcp(time: 0, host: "db.shop.localhost", listenPort: 15432, bytesIn: 1, bytesOut: 2, durationMs: 3, failed: false)

    func testScopes() {
        let route = http("shop.localhost")
        let proxied = http("api.github.com", proxy: true)
        XCTAssertTrue(TrafficScope.all.includes(route))
        XCTAssertTrue(TrafficScope.all.includes(proxied))
        XCTAssertTrue(TrafficScope.routes.includes(route))
        XCTAssertTrue(TrafficScope.routes.includes(tcp))
        XCTAssertFalse(TrafficScope.routes.includes(proxied))
        XCTAssertTrue(TrafficScope.proxy.includes(proxied))
        XCTAssertFalse(TrafficScope.proxy.includes(route))
        XCTAssertFalse(TrafficScope.proxy.includes(tcp))
    }

    func testFilterMatchesThePartOfTheHostIgnoringCaseAndSpaces() {
        let e = http("api.shop.localhost")
        XCTAssertTrue(e.matches(filter: "", scope: .all))
        XCTAssertTrue(e.matches(filter: "  API.Shop ", scope: .all))
        XCTAssertFalse(e.matches(filter: "docs", scope: .all))
        XCTAssertFalse(e.matches(filter: "api", scope: .proxy))
    }

    func testStatusKinds() {
        XCTAssertEqual(StatusKind(status: 101), .neutral)
        XCTAssertEqual(StatusKind(status: 200), .success)
        XCTAssertEqual(StatusKind(status: 204), .success)
        XCTAssertEqual(StatusKind(status: 304), .neutral)
        XCTAssertEqual(StatusKind(status: 404), .clientError)
        XCTAssertEqual(StatusKind(status: 502), .serverError)
    }

    func testDurations() {
        XCTAssertEqual(TrafficFormat.duration(ms: 0), "0 ms")
        XCTAssertEqual(TrafficFormat.duration(ms: 999), "999 ms")
        XCTAssertEqual(TrafficFormat.duration(ms: 2140), "2.1 s")
        XCTAssertEqual(TrafficFormat.duration(ms: 75_000), "75 s")
    }

    func testBytes() {
        XCTAssertEqual(TrafficFormat.bytes(512), "512 B")
        XCTAssertEqual(TrafficFormat.bytes(1200), "1.2 KB")
        XCTAssertEqual(TrafficFormat.bytes(8_400_000), "8.4 MB")
    }

    func testInspectionLabel() {
        XCTAssertEqual(TrafficFormat.inspection(hosts: []), "Read HTTPS: off")
        XCTAssertEqual(TrafficFormat.inspection(hosts: ["*"]), "Read HTTPS: all")
        XCTAssertEqual(TrafficFormat.inspection(hosts: ["api.example.com"]), "Read HTTPS: 1 host")
        XCTAssertEqual(TrafficFormat.inspection(hosts: ["a.com", "*.b.com"]), "Read HTTPS: 2 hosts")
    }
}
