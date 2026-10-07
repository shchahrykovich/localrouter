import Foundation
import XCTest
@testable import LocalRouterKit

final class RouteGroupsTests: XCTestCase {
    private func view(_ host: String, _ path: String? = nil, up: Bool? = nil, listenFailed: Bool = false) throws -> RouteView {
        let pathField = path.map { #","path":"\#($0)""# } ?? ""
        let upField = up.map { #","upstream_up":\#($0)"# } ?? ""
        let json = #"{"host":"\#(host)"\#(pathField),"target":"http://127.0.0.1:3000","urls":[]\#(upField),"listen_failed":\#(listenFailed)}"#
        return try Api.decoder.decode(RouteView.self, from: Data(json.utf8))
    }

    func testOnlineProjectsComeFirstThenByName() throws {
        let routes = [try view("alpha", up: false), try view("zeta", up: true), try view("beta"), try view("mid", up: true)]
        XCTAssertEqual(RouteGroup.group(routes).map(\.project), ["mid", "zeta", "alpha", "beta"])
    }

    func testOnlineRoutesComeFirstInsideAProject() throws {
        let routes = [
            try view("a.shop", up: false),
            try view("b.shop"),
            try view("c.shop", up: true),
            try view("d.shop", up: false),
            try view("e.shop", up: true),
        ]
        XCTAssertEqual(RouteGroup.group(routes)[0].routes.map(\.id), ["c.shop", "e.shop", "a.shop", "b.shop", "d.shop"])
    }

    // A route whose listen port is taken has an orange dot, not a green one.
    func testATakenListenPortIsNotOnline() throws {
        XCTAssertFalse(try view("db.shop", up: true, listenFailed: true).online)
        XCTAssertTrue(try view("db.shop", up: true).online)
    }

    func testProjectIsTheLastLabel() throws {
        XCTAssertEqual(try view("shop").route.project, "shop")
        XCTAssertEqual(try view("feat-login.shop").route.project, "shop")
        XCTAssertEqual(try view("db.feat-login.shop").route.project, "shop")
    }

    func testBranchAndPathRoutesJoinTheirProject() throws {
        let routes = [
            try view("api.supplements-app"),
            try view("lmsdk-private"),
            try view("supplements-app"),
            try view("supplements-app", "/brands"),
        ]
        let groups = RouteGroup.group(routes)
        XCTAssertEqual(groups.map(\.project), ["lmsdk-private", "supplements-app"])
        XCTAssertEqual(groups[1].routes.map(\.id), ["api.supplements-app", "supplements-app", "supplements-app/brands"])
    }

    func testGroupsAreSortedAndKeepTheDaemonOrderInside() throws {
        let routes = [try view("zeta"), try view("b.alpha"), try view("a.alpha")]
        let groups = RouteGroup.group(routes)
        XCTAssertEqual(groups.map(\.project), ["alpha", "zeta"])
        XCTAssertEqual(groups[0].routes.map(\.id), ["b.alpha", "a.alpha"])
    }

    func testNoRoutesNoGroups() {
        XCTAssertEqual(RouteGroup.group([]), [])
    }

    func testToggleCollapsesAndExpandsAProject() {
        var collapsed = CollapsedProjects()
        XCTAssertFalse(collapsed.contains("shop"))
        collapsed.toggle("shop")
        XCTAssertTrue(collapsed.contains("shop"))
        XCTAssertFalse(collapsed.contains("blog"))
        collapsed.toggle("shop")
        XCTAssertFalse(collapsed.contains("shop"))
    }

    func testCollapsedProjectsRoundTripThroughTheSavedString() {
        let collapsed = CollapsedProjects(["supplements-app", "lmsdk-private"])
        XCTAssertEqual(collapsed.rawValue, "lmsdk-private\nsupplements-app")
        XCTAssertEqual(CollapsedProjects(rawValue: collapsed.rawValue), collapsed)
    }

    func testAnEmptySavedStringMeansNothingCollapsed() {
        XCTAssertEqual(CollapsedProjects().rawValue, "")
        XCTAssertEqual(CollapsedProjects(rawValue: ""), CollapsedProjects())
    }
}
