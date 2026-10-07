import XCTest
@testable import LocalRouterKit

final class AppMenuTests: XCTestCase {
    private func context(chrome: Bool = true, log: Bool = true, apps: [AgentApp] = [.claude, .codex], cli: Bool = false) -> AppMenu.Context {
        AppMenu.Context(appName: "LocalRouter", chromeInstalled: chrome, hasProxyLog: log, agentApps: apps,
                        cliInstalled: cli, agentHelpHost: "router.localhost", onlineSummary: "6 of 7 online")
    }

    private func actions(_ entries: [AppMenu.Entry]) -> [AppMenu.Action] {
        entries.flatMap { entry -> [AppMenu.Action] in
            switch entry {
            case let .item(item): [item.action]
            case let .submenu(_, _, children): actions(children)
            case .header, .separator: []
            }
        }
    }

    private func headers(_ entries: [AppMenu.Entry]) -> [String] {
        entries.compactMap { if case let .header(title) = $0 { title } else { nil } }
    }

    private func submenu(_ title: String, in entries: [AppMenu.Entry]) -> [AppMenu.Entry]? {
        for case let .submenu(t, _, children) in entries where t == title { return children }
        return nil
    }

    func testTheIconMenuStartsWithOpenAndItsOnlineCount() throws {
        let entries = AppMenu.entries(context(), window: true)
        guard case let .item(first) = entries.first else { return XCTFail("no first item") }
        XCTAssertEqual(first.title, "Open LocalRouter")
        XCTAssertEqual(first.action, .openWindow)
        XCTAssertEqual(first.detail, "6 of 7 online")
        XCTAssertTrue(first.emphasized)
    }

    func testTheWindowMenuHasNoOpenItem() {
        XCTAssertFalse(actions(AppMenu.entries(context(), window: false)).contains(.openWindow))
    }

    func testEveryCommandOfTheOldMenuIsStillThere() {
        let all = actions(AppMenu.entries(context(), window: true))
        let expected: [AppMenu.Action] = [
            .openWindow, .openChromeViaProxy, .openProxyLog, .showProxyLogFolder,
            .copyAgentPrompt, .copyMCPCommand, .openAgentHelp,
            .installCLI, .installClaude, .installCodex,
            .help, .askForHelp(.claude), .askForHelp(.codex),
            .checkForUpdates, .settings, .quit,
        ]
        XCTAssertEqual(all, expected)
    }

    func testSectionsAndSubmenus() throws {
        let entries = AppMenu.entries(context(), window: true)
        XCTAssertEqual(headers(entries), ["Proxy", "Coding agents"])
        let install = try XCTUnwrap(submenu("Install", in: entries))
        XCTAssertEqual(actions(install), [.installCLI, .installClaude, .installCodex])
        let help = try XCTUnwrap(submenu("Help", in: entries))
        XCTAssertEqual(actions(help), [.help, .askForHelp(.claude), .askForHelp(.codex)])
    }

    func testTheProxySectionFollowsChromeAndTheLog() {
        XCTAssertFalse(headers(AppMenu.entries(context(chrome: false, log: false), window: true)).contains("Proxy"))
        let noChrome = actions(AppMenu.entries(context(chrome: false, log: true), window: true))
        XCTAssertFalse(noChrome.contains(.openChromeViaProxy))
        XCTAssertTrue(noChrome.contains(.openProxyLog))
        let noLog = actions(AppMenu.entries(context(chrome: true, log: false), window: true))
        XCTAssertTrue(noLog.contains(.openChromeViaProxy))
        XCTAssertFalse(noLog.contains(.showProxyLogFolder))
    }

    func testHelpIsAPlainItemWithoutAgentApps() {
        let entries = AppMenu.entries(context(apps: []), window: true)
        XCTAssertNil(submenu("Help", in: entries))
        XCTAssertTrue(actions(entries).contains(.help))
    }

    func testAnInstalledCommandLineToolIsChecked() throws {
        func cliItem(_ installed: Bool) throws -> AppMenu.Item {
            let install = try XCTUnwrap(submenu("Install", in: AppMenu.entries(context(cli: installed), window: true)))
            guard case let .item(item) = install.first else { throw XCTSkip("no item") }
            return item
        }
        XCTAssertTrue(try cliItem(true).checked)
        XCTAssertFalse(try cliItem(false).checked)
    }

    func testAgentInstructionsShowTheirHostAndSettingsHasItsShortcut() {
        let items = AppMenu.entries(context(), window: true).compactMap { if case let .item(i) = $0 { i } else { nil } }
        XCTAssertEqual(items.first { $0.action == .openAgentHelp }?.detail, "router.localhost")
        XCTAssertEqual(items.first { $0.action == .settings }?.shortcut, ",")
        XCTAssertEqual(items.first { $0.action == .quit }?.title, "Quit LocalRouter")
    }
}
