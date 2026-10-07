import Foundation
import XCTest
@testable import LocalRouterKit

final class RoutePresentationTests: XCTestCase {
    private func view(_ json: String) throws -> RouteView {
        try Api.decoder.decode(RouteView.self, from: Data(json.utf8))
    }

    private func http(_ host: String, up: Bool? = true, extra: String = "") throws -> RouteView {
        let upField = up.map { #","upstream_up":\#($0)"# } ?? ""
        return try view(#"{"host":"\#(host)","target":"http://127.0.0.1:5173","urls":["https://\#(host).localhost"]\#(upField)\#(extra)}"#)
    }

    // MARK: Target

    func testHTTPTargetDropsTheScheme() throws {
        XCTAssertEqual(try http("shop").route.displayTarget(home: "/Users/me"), "127.0.0.1:5173")
    }

    func testTCPTargetDropsTheScheme() throws {
        let db = try view(#"{"host":"db.shop","protocol":"tcp","target":"tcp://127.0.0.1:55001","listen_port":15432,"urls":["db.shop.localhost:15432"]}"#)
        XCTAssertEqual(db.route.displayTarget(home: "/Users/me"), "127.0.0.1:55001")
    }

    func testFolderTargetIsAPathWithTheHomeShortened() throws {
        let docs = try view(#"{"host":"docs","target":"file:///Users/me/docs/My%20Site","urls":[]}"#)
        XCTAssertEqual(docs.route.displayTarget(home: "/Users/me"), "~/docs/My Site")
        XCTAssertEqual(docs.route.displayTarget(home: "/Users/other"), "/Users/me/docs/My Site")
    }

    func testAnHTTPSTargetKeepsItsScheme() throws {
        let r = try view(#"{"host":"x","target":"https://127.0.0.1:8443","urls":[]}"#)
        XCTAssertEqual(r.route.displayTarget(home: "/Users/me"), "https://127.0.0.1:8443")
    }

    // MARK: Name

    func testNameSuffixOfHTTPAndTCPRoutes() throws {
        XCTAssertEqual(try http("shop").route.nameSuffix, ".localhost")
        let db = try view(#"{"host":"db.shop","protocol":"tcp","target":"tcp://127.0.0.1:55001","listen_port":15432,"urls":[]}"#)
        XCTAssertEqual(db.route.nameSuffix, ".localhost:15432")
    }

    // MARK: Badges

    func testBadges() throws {
        XCTAssertEqual(try http("shop", extra: #","persistent":true"#).route.badges, [])
        XCTAssertEqual(try http("shop").route.badges, [.session])
        XCTAssertEqual(try http("feat.shop", extra: #","owner_pid":42"#).route.badges, [.owned])
        XCTAssertEqual(try http("shop", extra: #","path":"/api","strip_path":true,"persistent":true"#).route.badges, [.stripPath])
        let db = try view(#"{"host":"db","protocol":"tcp","target":"tcp://127.0.0.1:1","persistent":true,"urls":[]}"#)
        XCTAssertEqual(db.route.badges, [.tcp])
        let docs = try view(#"{"host":"docs","target":"file:///d","owner_pid":7,"urls":[]}"#)
        XCTAssertEqual(docs.route.badges, [.folder, .owned])
    }

    // MARK: Health

    func testHealthAndItsNote() throws {
        XCTAssertEqual(try http("a", up: true).health, .online)
        XCTAssertNil(try http("a", up: true).healthNote)
        XCTAssertEqual(try http("a", up: false).health, .down)
        XCTAssertEqual(try http("a", up: false).healthNote, "not answering")
        XCTAssertEqual(try http("a", up: nil).health, .unknown)
        let taken = try http("a", up: true, extra: #","listen_failed":true"#)
        XCTAssertEqual(taken.health, .listenFailed)
        XCTAssertEqual(taken.healthNote, "listen port is taken")
    }

    func testPrimaryURLFallsBackToTheName() throws {
        XCTAssertEqual(try http("shop").primaryURL, "https://shop.localhost")
        let bare = try view(#"{"host":"shop","path":"/blog","target":"http://127.0.0.1:1","urls":[]}"#)
        XCTAssertEqual(bare.primaryURL, "shop.localhost/blog")
    }

    // MARK: Counts

    func testGroupSummary() throws {
        let group = RouteGroup.group([try http("a.shop", up: true), try http("b.shop", up: false), try http("shop", up: true)])[0]
        XCTAssertEqual(group.onlineCount, 2)
        XCTAssertEqual(group.summary, "2 of 3 online")
    }

    func testStatusLine() {
        XCTAssertEqual(StatusLine.text(running: false, online: 0, total: 0), "Not running")
        XCTAssertEqual(StatusLine.text(running: true, online: 0, total: 0), "Running · no routes")
        XCTAssertEqual(StatusLine.text(running: true, online: 1, total: 1), "Running · 1 of 1 route online")
        XCTAssertEqual(StatusLine.text(running: true, online: 6, total: 7), "Running · 6 of 7 routes online")
    }
}
