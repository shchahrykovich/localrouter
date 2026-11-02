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
        XCTAssertEqual(TrafficFormat.bytes(4_100_000), "4.1 MB")
        XCTAssertEqual(TrafficFormat.bytes(1_250_000_000), "1.25 GB")
    }

    func testInspectionLabel() {
        XCTAssertEqual(TrafficFormat.inspection(hosts: []), "Read HTTPS: off")
        XCTAssertEqual(TrafficFormat.inspection(hosts: ["*"]), "Read HTTPS: all")
        XCTAssertEqual(TrafficFormat.inspection(hosts: ["api.example.com"]), "Read HTTPS: 1 host")
        XCTAssertEqual(TrafficFormat.inspection(hosts: ["a.com", "*.b.com"]), "Read HTTPS: 2 hosts")
    }

    // ADR 11, T14, I16: a 1.8 daemon gets the "too old" view, not "No traffic".
    func testTheAppsViewNeedsAPI19() {
        XCTAssertEqual(AppsAvailability(apiVersion: "1.8"), .tooOld(daemon: "1.8"))
        XCTAssertNotNil(AppsAvailability(apiVersion: "1.8").message)
        XCTAssertEqual(AppsAvailability(apiVersion: "1.9"), .available)
        XCTAssertEqual(AppsAvailability(apiVersion: "1.10"), .available)
        XCTAssertEqual(AppsAvailability(apiVersion: "2.0"), .available)
    }

    func testTheModeIsAppsByDefaultAndRemembered() {
        let defaults = UserDefaults(suiteName: "TrafficTests-\(UUID().uuidString)")!
        XCTAssertEqual(TrafficMode.load(defaults), .apps)
        TrafficMode.requests.save(defaults)
        XCTAssertEqual(TrafficMode.load(defaults), .requests)
    }

    private func remote(_ app: String, _ via: TrafficVia, address: String?, host: String? = nil, owner: String? = nil,
                        place: Place? = nil, bytesIn: UInt64, bytesOut: UInt64 = 0) -> RemoteTraffic {
        RemoteTraffic(app: app, address: address, port: 443, proto: .tcp, host: host, asnOrg: owner, place: place, via: via,
                      bytesIn: bytesIn, bytesOut: bytesOut)
    }

    private let london = Place(city: "London", country: "United Kingdom", countryCode: "GB", lat: 51.5, lon: -0.12)
    private let ashburn = Place(city: "Ashburn", country: "United States", countryCode: "US", lat: 39.0, lon: -77.5)

    func testAppRowTexts() {
        let app = AppTraffic(app: "exited:curl", name: "curl", kind: .exited, paths: [], pids: [], bytesIn: 10, bytesOut: 0,
                             byVia: ByVia(proxy: InOut(bytesIn: 10)), addresses: 41, spark: [0, 5, 10])
        XCTAssertEqual(AppTrafficText.name(app), "curl (exited)")
        XCTAssertEqual(AppTrafficText.subline(app), "exited · 41 addresses · proxy")
        XCTAssertEqual(AppTrafficText.spark([0, 5, 10]), [0, 0.5, 1])
        XCTAssertEqual(AppTrafficText.spark([0, 0]), [0, 0])
        let totals = TrafficTotals(bytesIn: 431_000_000, bytesOut: 96_500_000, byVia: ByVia())
        XCTAssertEqual(AppTrafficText.summary(totals), "↓ 431.0 MB, ↑ 96.5 MB last hour")
    }

    func testRemoteLabelsTopListsAndPlaces() {
        let remotes = [
            remote("/A.app", .direct, address: "203.0.113.4", owner: "GitHub, Inc.", place: london, bytesIn: 900),
            remote("/A.app", .direct, address: "203.0.113.5", owner: "GitHub, Inc.", place: london, bytesIn: 100),
            remote("/B", .direct, address: "198.51.100.1", owner: "Amazon.com, Inc.", place: ashburn, bytesIn: 500),
            remote("/A.app", .router, address: "::1", host: "shop.localhost", bytesIn: 70),
            remote("/B", .proxy, address: nil, bytesIn: 30),
        ]
        XCTAssertEqual(AppTrafficText.label(remotes[0]), "GitHub, Inc.")
        XCTAssertEqual(AppTrafficText.label(remotes[3]), "shop.localhost")
        XCTAssertEqual(AppTrafficText.label(remotes[4]), "through the proxy")
        XCTAssertEqual(AppTrafficText.detail(remotes[0]), "203.0.113.4:443 · London, GB")
        XCTAssertEqual(AppTrafficText.detail(remotes[3]), "[::1]:443")
        XCTAssertEqual(AppTrafficText.topOwners(remotes).map(\.label), ["GitHub, Inc.", "Amazon.com, Inc.", "shop.localhost"])
        XCTAssertEqual(AppTrafficText.topCountries(remotes).map(\.label), ["United Kingdom", "United States"])
        let places = AppTrafficText.places(remotes, appNames: ["/A.app": "A", "/B": "B"])
        XCTAssertEqual(places.map(\.name), ["London, GB", "Ashburn, US"])
        XCTAssertEqual(places[0].addresses, 2)
        XCTAssertEqual(places[0].topApp, "A")
        XCTAssertEqual(places[0].bytesIn, 1000)
        let off = AppTrafficText.notOnMap(remotes)
        XCTAssertEqual(off.router, 70)
        XCTAssertEqual(off.noPlace, 30)
        XCTAssertTrue(AppTrafficText.matches(remotes[0], appName: "A", query: "github"))
        XCTAssertTrue(AppTrafficText.matches(remotes[2], appName: "B", query: "united states"))
        XCTAssertFalse(AppTrafficText.matches(remotes[2], appName: "B", query: "github"))
    }

    // MARK: The tree of the Apps view

    private func app(_ key: String, kind: AppKind = .app, paths: [String]) -> AppTraffic {
        AppTraffic(app: key, name: key, kind: kind, paths: paths, pids: [], bytesIn: 0, bytesOut: 0, byVia: ByVia(),
                   addresses: 0, spark: [])
    }

    func testAnAppWithSeveralProcessesOpensToItsProcesses() {
        let services = app("macOS services", kind: .services, paths: ["/usr/sbin/mDNSResponder", "/usr/libexec/trustd"])
        let remotes = [
            RemoteTraffic(app: "macOS services", exe: "/usr/sbin/mDNSResponder", address: "1.1.1.1", port: 443, proto: .tcp,
                          asnOrg: "Cloudflare, Inc.", via: .direct, bytesIn: 900, bytesOut: 10),
            RemoteTraffic(app: "macOS services", exe: "/usr/libexec/trustd", address: "17.253.1.1", port: 80, proto: .tcp,
                          asnOrg: "Apple Inc.", via: .direct, bytesIn: 50, bytesOut: 5),
        ]
        let nodes = TrafficTree.children(of: services, remotes: remotes, logs: [])
        XCTAssertEqual(nodes.map(\.title), ["mDNSResponder", "trustd"])
        XCTAssertEqual(nodes.map(\.kind), [.process, .process])
        XCTAssertEqual(nodes[0].bytesIn, 900)
        XCTAssertEqual(nodes[0].children.map(\.title), ["Cloudflare, Inc."])
        XCTAssertEqual(nodes[0].children[0].children.map(\.title), ["1.1.1.1:443"])
        // Ids are unique across the whole tree.
        let ids = nodes.flatMap { [$0.id] + $0.children.flatMap { [$0.id] + $0.children.map(\.id) } }
        XCTAssertEqual(ids.count, Set(ids).count)
    }

    func testOneProcessOpensToDomainsAndAHostToItsRequests() {
        let chrome = app("/A.app", paths: ["/A.app/Contents/MacOS/A"])
        let exe = "/A.app/Contents/MacOS/A"
        let remotes = [
            RemoteTraffic(app: "/A.app", exe: exe, address: "203.0.113.4", port: 443, proto: .tcp, host: "github.com",
                          via: .proxy, bytesIn: 500, bytesOut: 50),
            RemoteTraffic(app: "/A.app", exe: exe, address: "203.0.113.5", port: 443, proto: .tcp, host: "github.com",
                          via: .proxy, bytesIn: 100, bytesOut: 10),
            RemoteTraffic(app: "/A.app", exe: exe, address: "198.51.100.9", port: 443, proto: .udp, via: .direct,
                          bytesIn: 300, bytesOut: 30),
            RemoteTraffic(app: "/A.app", exe: exe, address: "::1", port: 443, proto: .tcp, host: "shop.localhost", via: .router,
                          bytesIn: 40, bytesOut: 4),
        ]
        let logs: [LogEntry] = [
            .http(time: 2, method: "GET", host: "github.com:443", path: "/b", status: 200, durationMs: 5, route: nil,
                  proxy: ProxyTraffic(mode: "inspect"), scripts: nil),
            .http(time: 1, method: "POST", host: "github.com", path: "/a", status: 404, durationMs: 9, route: nil,
                  proxy: ProxyTraffic(mode: "http"), scripts: nil),
            .http(time: 3, method: "GET", host: "shop.localhost", path: "/", status: 200, durationMs: 1, route: "shop",
                  proxy: nil, scripts: nil),
            .http(time: 4, method: "GET", host: "gitlab.com", path: "/", status: 200, durationMs: 1, route: nil,
                  proxy: ProxyTraffic(mode: "http"), scripts: nil),
        ]
        let nodes = TrafficTree.children(of: chrome, remotes: remotes, logs: logs)
        XCTAssertEqual(nodes.map(\.title), ["github.com", "IP Addresses", "shop.localhost"])
        XCTAssertEqual(nodes.map(\.kind), [.group, .group, .group])
        let github = nodes[0]
        XCTAssertEqual(github.bytesIn, 600)
        XCTAssertEqual(github.via, .proxy)
        XCTAssertEqual(github.children.filter { $0.kind == .address }.map(\.title), ["203.0.113.4:443", "203.0.113.5:443"])
        // The requests of the host, newest first, the port of the log's host ignored.
        XCTAssertEqual(github.children.filter { $0.kind == .request }.map(\.title), ["GET /b", "POST /a"])
        XCTAssertEqual(github.children.last?.status, 404)
        XCTAssertEqual(nodes[1].children.map(\.title), ["198.51.100.9:443"])
        XCTAssertEqual(nodes[1].children[0].detail, "udp")
        XCTAssertEqual(nodes[2].children.filter { $0.kind == .request }.map(\.title), ["GET /"])
    }

    // Reverse DNS: a server name groups addresses when there is no host and
    // no owner, and shows on the address row.
    func testServerNamesGroupAndLabelAddresses() {
        let a = app("/K.app", paths: ["/K.app/Contents/MacOS/K"])
        let remotes = [
            RemoteTraffic(app: "/K.app", exe: "/K.app/Contents/MacOS/K", address: "34.149.66.165", port: 443, proto: .tcp,
                          serverName: "165.66.149.34.bc.googleusercontent.com", via: .direct, bytesIn: 10, bytesOut: 1),
            RemoteTraffic(app: "/K.app", exe: "/K.app/Contents/MacOS/K", address: "34.149.66.166", port: 443, proto: .tcp,
                          serverName: "166.66.149.34.bc.googleusercontent.com", via: .direct, bytesIn: 5, bytesOut: 1),
            RemoteTraffic(app: "/K.app", exe: "/K.app/Contents/MacOS/K", address: "160.79.104.10", port: 443, proto: .tcp,
                          asnOrg: "Anthropic, PBC", serverName: "x.example.net", via: .direct, bytesIn: 1, bytesOut: 1),
        ]
        let nodes = TrafficTree.children(of: a, remotes: remotes, logs: [])
        XCTAssertEqual(nodes.map(\.title), ["googleusercontent.com", "Anthropic, PBC"])
        XCTAssertEqual(nodes[0].children.count, 2)
        XCTAssertEqual(nodes[0].children[0].detail, "tcp · 165.66.149.34.bc.googleusercontent.com")
        XCTAssertEqual(TrafficTree.baseDomain("server-1.cdn.example.co.uk"), "example.co.uk")
        XCTAssertEqual(TrafficTree.baseDomain("lhr25s34-in-f14.1e100.net"), "1e100.net")
        XCTAssertEqual(TrafficTree.baseDomain("localhost"), "localhost")
    }
}
