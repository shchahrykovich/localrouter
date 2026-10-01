// ADR 03, T8: a route is identified by host plus path, in the list and when it
// is removed (manifest B1 and B3).

import Foundation
import XCTest
@testable import LocalRouterKit

final class RouteKeyTests: XCTestCase {
    private func view(_ json: String) throws -> RouteView {
        try Api.decoder.decode(RouteView.self, from: Data(json.utf8))
    }

    func testRoutesOfOneHostHaveDifferentIds() throws {
        let main = try view(#"{"host":"shop","target":"http://127.0.0.1:5173","urls":[]}"#)
        let blog = try view(#"{"host":"shop","path":"/blog","target":"http://127.0.0.1:3001","urls":[]}"#)
        XCTAssertNotEqual(main.id, blog.id)
        XCTAssertEqual(main.id, "shop")
        XCTAssertEqual(blog.id, "shop/blog")
        XCTAssertEqual(blog.route.fullNameAndPath, "shop.localhost/blog")
    }

    func testRemovingAPathRouteSendsItsPath() throws {
        let blog = try view(#"{"host":"shop","path":"/blog","target":"http://127.0.0.1:3001","urls":[]}"#)
        let sent = try JSONSerialization.jsonObject(with: Api.encoder.encode(HostParams(route: blog.route))) as? [String: String]
        XCTAssertEqual(sent, ["host": "shop", "path": "/blog"])
    }

    func testRemovingTheDefaultRouteSendsNoPath() throws {
        let main = try view(#"{"host":"shop","target":"http://127.0.0.1:5173","urls":[]}"#)
        let sent = try JSONSerialization.jsonObject(with: Api.encoder.encode(HostParams(route: main.route))) as? [String: String]
        XCTAssertEqual(sent, ["host": "shop"])
    }

    func testLogEntryKeepsTheRoute() throws {
        let json = #"{"kind":"http","time_ms":1,"method":"GET","host":"shop.localhost","path":"/blog","status":200,"duration_ms":2,"route":"shop/blog"}"#
        let entry = try Api.decoder.decode(LogEntry.self, from: Data(json.utf8))
        guard case let .http(_, _, _, _, _, _, route, _) = entry else { return XCTFail("not http") }
        XCTAssertEqual(route, "shop/blog")
    }
}
