import XCTest
@testable import LocalRouterKit

final class SettingsTreeTests: XCTestCase {
    private func pages(_ nodes: [SettingsNode]) -> [SettingsPage] {
        nodes.flatMap { [$0.page] + pages($0.children) }
    }

    func testEveryPageIsInTheTreeOnce() {
        let all = pages(SettingsTree.filter(""))
        XCTAssertEqual(all.count, Set(all).count)
        XCTAssertEqual(Set(all), Set(SettingsPage.allCases))
    }

    func testEmptyOrBlankQueryShowsTheWholeTree() {
        XCTAssertEqual(SettingsTree.filter(""), SettingsTree.roots)
        XCTAssertEqual(SettingsTree.filter("   "), SettingsTree.roots)
    }

    func testTitleMatchIgnoresCase() {
        XCTAssertEqual(pages(SettingsTree.filter("ROUTING")), [.routing])
    }

    func testKeywordMatchFindsThePageThatHoldsTheSetting() {
        XCTAssertEqual(pages(SettingsTree.filter("lan")), [.routing])
        XCTAssertEqual(pages(SettingsTree.filter("uninstall")), [.general])
    }

    func testChildMatchKeepsItsParentButNotItsSiblings() {
        let result = SettingsTree.filter("lua")
        XCTAssertEqual(result.map(\.page), [.proxy])
        XCTAssertEqual(result[0].children.map(\.page), [.scripts])
    }

    func testParentMatchKeepsAllItsChildren() {
        let result = SettingsTree.filter("proxy")
        let proxy = try? XCTUnwrap(result.first { $0.page == .proxy })
        XCTAssertEqual(proxy?.children.map(\.page), [.inspection, .scripts])
    }

    func testEveryWordMustMatchTheSamePage() {
        XCTAssertEqual(pages(SettingsTree.filter("trust inspection")), [.proxy, .inspection])
        XCTAssertEqual(pages(SettingsTree.filter("lan lua")), [])
    }

    func testNoMatchShowsNothing() {
        XCTAssertEqual(SettingsTree.filter("zzz"), [])
        XCTAssertNil(SettingsTree.firstPage(in: [], query: "zzz"))
    }

    func testFirstPageIsTheFirstMatchInTreeOrder() {
        XCTAssertEqual(SettingsTree.firstPage(in: SettingsTree.filter(""), query: ""), .general)
        XCTAssertEqual(SettingsTree.firstPage(in: SettingsTree.filter("lua"), query: "lua"), .scripts)
        XCTAssertEqual(SettingsTree.firstPage(in: SettingsTree.filter("proxy"), query: "proxy"), .proxy)
    }

    func testContainsLooksInsideChildren() {
        let result = SettingsTree.filter("lua")
        XCTAssertTrue(SettingsTree.contains(.scripts, in: result))
        XCTAssertFalse(SettingsTree.contains(.inspection, in: result))
    }

    func testParentIsSetOnlyForChildPages() {
        XCTAssertEqual(SettingsTree.parent(of: .inspection), .proxy)
        XCTAssertEqual(SettingsTree.parent(of: .scripts), .proxy)
        XCTAssertNil(SettingsTree.parent(of: .proxy))
        XCTAssertNil(SettingsTree.parent(of: .general))
    }

    /// ADR 08, T13: the proxy log settings are on the Proxy page; the
    /// networks of LAN access on the Routing page.
    func testTheProxyLogAndLanNetworksAreFound() {
        XCTAssertEqual(SettingsTree.firstPage(in: SettingsTree.filter("har"), query: "har"), .proxy)
        XCTAssertEqual(SettingsTree.firstPage(in: SettingsTree.filter("requests per file"), query: "requests per file"), .proxy)
        XCTAssertEqual(SettingsTree.firstPage(in: SettingsTree.filter("proxy log"), query: "proxy log"), .proxy)
        XCTAssertEqual(pages(SettingsTree.filter("lan networks")), [.routing])
        XCTAssertEqual(pages(SettingsTree.filter("forget")), [.routing])
    }
}
