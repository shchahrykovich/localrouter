import Foundation
import XCTest
@testable import LocalRouterKit

final class AgentAppTests: XCTestCase {
    private func query(_ url: URL, _ name: String) -> String? {
        URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == name }?.value
    }

    func testClaudeOpensANewClaudeCodeSessionWithThePrompt() {
        let url = AgentApp.claude.link(prompt: "Help me")
        XCTAssertEqual(url.absoluteString, "claude://code/new?q=Help%20me")
    }

    func testCodexOpensANewThreadWithThePrompt() {
        let url = AgentApp.codex.link(prompt: "Help me")
        XCTAssertEqual(url.absoluteString, "codex://new?prompt=Help%20me")
    }

    /// `&`, `=`, `+`, `#` and `?` would end or change the value if they
    /// stayed as they are.
    func testThePromptComesBackUnchanged() {
        let prompt = "Run curl -s http://router.localhost:7080?a=1&b=2 + #3, then: 100% done\nnext line"
        for app in AgentApp.allCases {
            XCTAssertEqual(query(app.link(prompt: prompt), app.promptParameter), prompt, "\(app)")
        }
    }

    func testSchemeIsTheOneTheAppRegisters() {
        XCTAssertEqual(AgentApp.claude.scheme, "claude")
        XCTAssertEqual(AgentApp.codex.scheme, "codex")
        for app in AgentApp.allCases {
            XCTAssertEqual(app.link(prompt: "x").scheme, app.scheme)
        }
    }

    func testHelpPromptPointsAtTheHelpPage() {
        let prompt = AgentHelp.helpPrompt(url: "http://router.localhost:7080", app: "LocalRouter-dev")
        XCTAssertTrue(prompt.hasPrefix("Run curl -s http://router.localhost:7080 and read it."), prompt)
        XCTAssertTrue(prompt.contains("LocalRouter-dev"), prompt)
    }

    private func entry(_ json: String) throws -> LogEntry {
        try Api.decoder.decode(LogEntry.self, from: Data(json.utf8))
    }

    func testOnlyForwardProxyTrafficIsProxied() throws {
        let proxied = try entry(#"{"kind":"http","time_ms":1,"method":"GET","host":"example.com","path":"/","status":200,"duration_ms":3,"via":"proxy","mode":"tunnel"}"#)
        let routed = try entry(#"{"kind":"http","time_ms":1,"method":"GET","host":"shop.localhost","path":"/","status":200,"duration_ms":3,"route":"shop"}"#)
        let tcp = try entry(#"{"kind":"tcp","time_ms":1,"host":"db.shop","listen_port":15432,"bytes_in":1,"bytes_out":2,"duration_ms":3,"failed":false}"#)
        XCTAssertTrue(proxied.isProxied)
        XCTAssertFalse(routed.isProxied)
        XCTAssertFalse(tcp.isProxied)
    }
}
