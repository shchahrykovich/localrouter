// ADR 04, T10, T11, I2, I5, I10: each instance has its own folders and links,
// installs and removes only its own, and a suffixed instance never updates
// itself.

import Foundation
import XCTest
@testable import LocalRouterKit

final class InstanceFilesTests: XCTestCase {
    private var root: URL!
    private let fm = FileManager.default
    private var dev: Instance!

    override func setUpWithError() throws {
        root = fm.temporaryDirectory.appendingPathComponent("lr-inst-\(UUID().uuidString)")
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
        dev = try Instance(suffix: "-dev")
    }

    override func tearDownWithError() throws {
        try? fm.removeItem(at: root)
    }

    /// A fake bundle of `instance` with its CLI and note, under `root`.
    private func bundle(_ instance: Instance) throws -> URL {
        let app = root.appendingPathComponent("\(instance.appName).app")
        let cli = app.appendingPathComponent(BundleLayout.cli(for: instance))
        try fm.createDirectory(at: cli.deletingLastPathComponent(), withIntermediateDirectories: true)
        fm.createFile(atPath: cli.path, contents: Data(), attributes: [.posixPermissions: 0o755])
        let note = app.appendingPathComponent(BundleLayout.claudeNote)
        try fm.createDirectory(at: note.deletingLastPathComponent(), withIntermediateDirectories: true)
        try "note of \(instance)".write(to: note, atomically: true, encoding: .utf8)
        return app
    }

    private var bin: URL { root.appendingPathComponent("bin") }
    private var claude: URL { root.appendingPathComponent("dot-claude") }

    private func cli(_ instance: Instance) throws -> CLIInstaller {
        CLIInstaller(tool: try bundle(instance).appendingPathComponent(BundleLayout.cli(for: instance)),
                     binDir: bin, pathVariable: "", instance: instance)
    }

    private func claudeInstaller(_ instance: Instance) throws -> ClaudeInstaller {
        try fm.createDirectory(at: claude, withIntermediateDirectories: true)
        return ClaudeInstaller(note: try bundle(instance).appendingPathComponent(BundleLayout.claudeNote),
                               claudeDir: claude, instance: instance)
    }

    private func link(_ url: URL) -> String? { try? fm.destinationOfSymbolicLink(atPath: url.path) }

    // MARK: Folders

    func testFoldersFollowTheInstanceAndLocalRouterHomeWins() {
        let home = URL(fileURLWithPath: "/Users/u")
        let d = Paths.resolve(instance: dev, localHome: nil, home: home)
        XCTAssertEqual(d.data.path, "/Users/u/Library/Application Support/LocalRouter-dev")
        XCTAssertEqual(d.logs.path, "/Users/u/Library/Logs/LocalRouter-dev")
        let r = Paths.resolve(instance: .release, localHome: "", home: home)
        XCTAssertEqual(r.data.path, "/Users/u/Library/Application Support/LocalRouter")
        let t = Paths.resolve(instance: dev, localHome: "/tmp/lr1", home: home)
        XCTAssertEqual(t.data.path, "/tmp/lr1")
        XCTAssertEqual(t.logs.path, "/tmp/lr1/logs")
    }

    // MARK: Command line tool

    func testEachInstanceLinksItsOwnCommand() throws {
        let release = try cli(.release)
        let devCLI = try cli(dev)
        XCTAssertEqual(release.link.lastPathComponent, "localrouter")
        XCTAssertEqual(devCLI.link.lastPathComponent, "localrouter-dev")
        _ = try release.install()
        _ = try devCLI.install()
        XCTAssertEqual(link(release.link), release.tool.path)
        XCTAssertEqual(link(devCLI.link), devCLI.tool.path)
    }

    /// A link at the dev name that points into a release bundle is not ours.
    func testALinkIntoAnotherInstanceIsOccupied() throws {
        let release = try cli(.release)
        let devCLI = try cli(dev)
        try fm.createDirectory(at: bin, withIntermediateDirectories: true)
        try fm.createSymbolicLink(at: devCLI.link, withDestinationURL: release.tool)
        XCTAssertThrowsError(try devCLI.install()) { error in
            XCTAssertEqual(error as? CLIInstaller.Failure, .occupied(devCLI.link))
        }
    }

    // MARK: Claude Code note

    func testEachInstanceHasItsOwnNoteAndLine() throws {
        let release = try claudeInstaller(.release)
        let devNote = try claudeInstaller(dev)
        _ = try release.install()
        _ = try devNote.install()
        XCTAssertEqual(link(release.link), release.note.path)
        XCTAssertEqual(link(devNote.link), devNote.note.path)
        XCTAssertEqual(devNote.link.lastPathComponent, "LocalRouter-dev.md")
        let text = try String(contentsOf: release.claudeMD, encoding: .utf8)
        XCTAssertEqual(text, "@LocalRouter-dev.md\n@LocalRouter.md\n")
    }

    // MARK: Uninstall

    func testUninstallRemovesOnlyItsOwnInstance() throws {
        let home = root.appendingPathComponent("home")
        let releasePaths = Paths.resolve(instance: .release, localHome: nil, home: home)
        let devPaths = Paths.resolve(instance: dev, localHome: nil, home: home)
        for dir in [releasePaths.data, releasePaths.logs, devPaths.data, devPaths.logs] {
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        }
        let releaseCLI = try cli(.release), devCLI = try cli(dev)
        let releaseNote = try claudeInstaller(.release), devNote = try claudeInstaller(dev)
        for installer in [releaseCLI, devCLI] { _ = try installer.install() }
        for installer in [releaseNote, devNote] { _ = try installer.install() }
        let codexDir = root.appendingPathComponent("dot-codex")
        try fm.createDirectory(at: codexDir, withIntermediateDirectories: true)
        let releaseCodex = CodexInstaller(note: releaseNote.note, codexDir: codexDir, instance: .release)
        let devCodex = CodexInstaller(note: devNote.note, codexDir: codexDir, instance: dev)
        for installer in [releaseCodex, devCodex] { _ = try installer.install() }

        Uninstaller(dataDir: devPaths.data, logsDir: devPaths.logs, cli: devCLI, claude: devNote, codex: devCodex).removeFiles()

        XCTAssertFalse(fm.fileExists(atPath: devPaths.data.path))
        XCTAssertFalse(fm.fileExists(atPath: devPaths.logs.path))
        XCTAssertNil(link(devCLI.link))
        XCTAssertNil(link(devNote.link))
        XCTAssertNil(link(devCodex.link))
        XCTAssertEqual(link(releaseCodex.link), releaseCodex.note.path)
        XCTAssertTrue(fm.fileExists(atPath: releasePaths.data.path))
        XCTAssertTrue(fm.fileExists(atPath: releasePaths.logs.path))
        XCTAssertEqual(link(releaseCLI.link), releaseCLI.tool.path)
        XCTAssertEqual(link(releaseNote.link), releaseNote.note.path)
    }

    // MARK: Updates

    func testASuffixedInstanceNeverUpdatesItself() {
        XCTAssertNil(Updater.disabledReason(for: .release))
        XCTAssertEqual(Updater.disabledReason(for: dev), "Updates are off in LocalRouter-dev. Build it again with scripts/install.sh.")
    }

    // MARK: Feedback texts

    func testFeedbackNamesTheInstancesCommandAndLine() {
        let cliDone = Feedback.cliInstalled(link: URL(fileURLWithPath: "/Users/me/.local/bin/localrouter-dev"), onPath: true, pathHint: "")
        XCTAssertTrue(cliDone.detail.hasSuffix("run: localrouter-dev status"), cliDone.detail)
        let noteDone = Feedback.claudeInstalled(link: URL(fileURLWithPath: "/Users/me/.claude/LocalRouter-dev.md"),
                                                claudeMD: URL(fileURLWithPath: "/Users/me/.claude/CLAUDE.md"), importAdded: true)
        XCTAssertTrue(noteDone.detail.contains("@LocalRouter-dev.md is now the first line"), noteDone.detail)
    }
}
