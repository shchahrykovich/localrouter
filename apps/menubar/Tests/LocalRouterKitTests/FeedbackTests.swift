import XCTest
@testable import LocalRouterKit

final class FeedbackTests: XCTestCase {
    private let bin = URL(fileURLWithPath: "/Users/me/.local/bin/localrouter")
    private let note = URL(fileURLWithPath: "/Users/me/.claude/LocalRouter.md")
    private let claudeMD = URL(fileURLWithPath: "/Users/me/.claude/CLAUDE.md")

    func testCLIOnPathSaysHowToUseIt() {
        let f = Feedback.cliInstalled(link: bin, onPath: true, pathHint: "hint")
        XCTAssertEqual(f.title, "Command line tool installed")
        XCTAssertTrue(f.detail.contains("localrouter status"))
        XCTAssertFalse(f.detail.contains("hint"))
        XCTAssertFalse(f.failed)
    }

    func testCLINotOnPathGivesTheLineToAdd() {
        let f = Feedback.cliInstalled(link: bin, onPath: false, pathHint: "echo 'export PATH=...' >> ~/.zshrc")
        XCTAssertTrue(f.detail.contains("/Users/me/.local/bin is not on PATH"))
        XCTAssertTrue(f.detail.hasSuffix("echo 'export PATH=...' >> ~/.zshrc"))
    }

    func testClaudeInstalledSaysWhereTheLineWent() {
        let added = Feedback.claudeInstalled(link: note, claudeMD: claudeMD, importAdded: true)
        XCTAssertEqual(added.title, "Claude Code instructions installed")
        XCTAssertTrue(added.detail.contains("@LocalRouter.md is now the first line of /Users/me/.claude/CLAUDE.md"))

        let present = Feedback.claudeInstalled(link: note, claudeMD: claudeMD, importAdded: false)
        XCTAssertTrue(present.detail.contains("already reads it"))
    }

    func testNoClaudeIsAWarning() {
        let f = Feedback.noClaude(URL(fileURLWithPath: "/Users/me/.claude"))
        XCTAssertTrue(f.failed)
        XCTAssertTrue(f.detail.hasPrefix("/Users/me/.claude does not exist"))
    }

    func testUpdateTexts() {
        XCTAssertEqual(Feedback.upToDate(version: "0.1.5").detail, "Version 0.1.5 is the latest version.")
        let release = Updater.Release(version: "0.2.0", tag: "v0.2.0", dmg: URL(string: "https://github.com/x.dmg")!, notes: "")
        let f = Feedback.updateAvailable(release, current: "0.1.5")
        XCTAssertEqual(f.title, "LocalRouter 0.2.0 is available")
        XCTAssertTrue(f.detail.hasPrefix("You have version 0.1.5."))
    }

    func testSummaryJoinsTitleAndDetail() {
        XCTAssertEqual(Feedback(title: "Done", detail: "It worked.").summary, "Done. It worked.")
        XCTAssertEqual(Feedback(title: "Done", detail: "").summary, "Done")
        XCTAssertTrue(Feedback.failed("Could not check for updates", "offline").failed)
    }
}
