import XCTest
@testable import LocalRouterKit

final class AgentHelpTests: XCTestCase {
    func testDefaultPortsGiveAPlainName() {
        XCTAssertEqual(AgentHelp.url(httpPort: 80, httpsPort: 443), "http://router.localhost")
    }

    func testOtherHttpPortIsInTheURL() {
        XCTAssertEqual(AgentHelp.url(httpPort: 8080, httpsPort: 8443), "http://router.localhost:8080")
    }

    func testHttpsIsUsedWhenHttpIsOff() {
        XCTAssertEqual(AgentHelp.url(httpPort: nil, httpsPort: 443), "https://router.localhost")
        XCTAssertEqual(AgentHelp.url(httpPort: nil, httpsPort: 8443), "https://router.localhost:8443")
    }

    func testUnknownPortsFallBackToTheDefault() {
        XCTAssertEqual(AgentHelp.url(httpPort: nil, httpsPort: nil), "http://router.localhost")
    }

    func testPromptTellsTheAgentToFetchThePage() {
        XCTAssertEqual(
            AgentHelp.prompt(url: "http://router.localhost"),
            "Run curl -s http://router.localhost and follow it to add LocalRouter to this project."
        )
    }
}
