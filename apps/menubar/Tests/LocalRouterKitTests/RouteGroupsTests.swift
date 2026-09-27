import Foundation
import XCTest
@testable import LocalRouterKit

final class RouteGroupsTests: XCTestCase {
    private func view(_ host: String, _ path: String? = nil) throws -> RouteView {
        let pathField = path.map { #","path":"\#($0)""# } ?? ""
        let json = #"{"host":"\#(host)"\#(pathField),"target":"http://127.0.0.1:3000","urls":[]}"#
        return try Api.decoder.decode(RouteView.self, from: Data(json.utf8))
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
}
