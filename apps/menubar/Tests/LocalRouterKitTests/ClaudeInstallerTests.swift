import XCTest
@testable import LocalRouterKit

final class ClaudeInstallerTests: XCTestCase {
    private var root: URL!
    private let fm = FileManager.default

    override func setUpWithError() throws {
        root = fm.temporaryDirectory.appendingPathComponent("lr-claude-\(UUID().uuidString)")
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? fm.removeItem(at: root)
    }

    /// A note inside a fake bundle, and an existing ~/.claude.
    private func makeInstaller(bundle: String = "LocalRouter.app", note text: String = "the note") throws -> ClaudeInstaller {
        let note = root.appendingPathComponent(bundle).appendingPathComponent(BundleLayout.claudeNote)
        try fm.createDirectory(at: note.deletingLastPathComponent(), withIntermediateDirectories: true)
        try text.write(to: note, atomically: true, encoding: .utf8)
        let claude = root.appendingPathComponent("dot-claude")
        try fm.createDirectory(at: claude, withIntermediateDirectories: true)
        return ClaudeInstaller(note: note, claudeDir: claude)
    }

    private func read(_ url: URL) throws -> String { try String(contentsOf: url, encoding: .utf8) }

    // MARK: The line in CLAUDE.md

    func testLineGoesOnTopAndTheRestIsUnchanged() {
        let existing = "# My instructions\n\nBe brief.\n@RTK.md"
        XCTAssertEqual(ClaudeInstaller.withImport(existing), "@LocalRouter.md\n" + existing)
    }

    func testEmptyFileIsJustTheLine() {
        XCTAssertEqual(ClaudeInstaller.withImport(""), "@LocalRouter.md\n")
    }

    func testLineAlreadyThereIsNotAddedAgain() {
        XCTAssertNil(ClaudeInstaller.withImport("@LocalRouter.md\n@RTK.md\n"))
        XCTAssertNil(ClaudeInstaller.withImport("@RTK.md\n  @LocalRouter.md  \n"))
    }

    /// A sentence about the import is not the import.
    func testLineInsideASentenceDoesNotCount() {
        let existing = "Add @LocalRouter.md to turn it on.\n"
        XCTAssertEqual(ClaudeInstaller.withImport(existing), "@LocalRouter.md\n" + existing)
    }

    // MARK: Install

    func testInstallLinksTheNoteAndAddsTheLine() throws {
        let installer = try makeInstaller()
        try "@RTK.md\n".write(to: installer.claudeMD, atomically: true, encoding: .utf8)

        XCTAssertEqual(try installer.install(), .installed(link: installer.link, importAdded: true))
        XCTAssertEqual(try fm.destinationOfSymbolicLink(atPath: installer.link.path), installer.note.path)
        XCTAssertEqual(try read(installer.link), "the note")
        XCTAssertEqual(try read(installer.claudeMD), "@LocalRouter.md\n@RTK.md\n")
    }

    func testMissingClaudeMDIsCreatedWithTheLine() throws {
        let installer = try makeInstaller()
        _ = try installer.install()
        XCTAssertEqual(try read(installer.claudeMD), "@LocalRouter.md\n")
    }

    func testSecondInstallChangesNothing() throws {
        let installer = try makeInstaller()
        _ = try installer.install()
        let before = try read(installer.claudeMD)
        XCTAssertEqual(try installer.install(), .installed(link: installer.link, importAdded: false))
        XCTAssertEqual(try read(installer.claudeMD), before)
    }

    /// No ~/.claude: Claude Code is not here, so nothing is created.
    func testNoClaudeFolderDoesNothing() throws {
        let installer = try makeInstaller()
        try fm.removeItem(at: installer.claudeDir)
        XCTAssertEqual(try installer.install(), .noClaude(installer.claudeDir))
        XCTAssertFalse(fm.fileExists(atPath: installer.claudeDir.path))
    }

    func testMissingNoteFailsAndLeavesClaudeMDAlone() throws {
        let installer = try makeInstaller()
        try fm.removeItem(at: installer.note)
        XCTAssertThrowsError(try installer.install()) {
            XCTAssertEqual($0 as? ClaudeInstaller.Failure, .noteMissing(installer.note))
        }
        XCTAssertFalse(fm.fileExists(atPath: installer.claudeMD.path))
    }

    /// CLAUDE.md is a link into a dotfiles folder: the real file gets the
    /// line, and the link stays a link.
    func testClaudeMDThatIsALinkKeepsTheLink() throws {
        let installer = try makeInstaller()
        let real = root.appendingPathComponent("dotfiles-CLAUDE.md")
        try "@RTK.md\n".write(to: real, atomically: true, encoding: .utf8)
        try fm.createSymbolicLink(at: installer.claudeMD, withDestinationURL: real)

        _ = try installer.install()

        XCTAssertEqual(try fm.destinationOfSymbolicLink(atPath: installer.claudeMD.path), real.path)
        XCTAssertEqual(try read(real), "@LocalRouter.md\n@RTK.md\n")
    }

    func testClaudeMDThatIsAFolderIsRefused() throws {
        let installer = try makeInstaller()
        try fm.createDirectory(at: installer.claudeMD, withIntermediateDirectories: true)
        XCTAssertThrowsError(try installer.install())
        var isDir: ObjCBool = false
        XCTAssertTrue(fm.fileExists(atPath: installer.claudeMD.path, isDirectory: &isDir) && isDir.boolValue)
    }

    // MARK: The link

    /// An update or a second copy of the app: our old link is replaced.
    func testLinkIntoAnotherBundleIsReplaced() throws {
        let old = try makeInstaller(bundle: "Old.app", note: "old")
        _ = try old.install()
        let new = try makeInstaller(bundle: "New.app", note: "new")
        _ = try new.install()
        XCTAssertEqual(try read(new.link), "new")
    }

    /// `fileExists` follows the link and says "nothing there" for this case.
    func testLinkIntoADeletedBundleIsReplaced() throws {
        let installer = try makeInstaller()
        let gone = root.appendingPathComponent("Gone.app").appendingPathComponent(BundleLayout.claudeNote)
        try fm.createSymbolicLink(atPath: installer.link.path, withDestinationPath: gone.path)
        _ = try installer.install()
        XCTAssertEqual(try read(installer.link), "the note")
    }

    func testUsersOwnFileIsLeftAlone() throws {
        let installer = try makeInstaller()
        try "my notes".write(to: installer.link, atomically: true, encoding: .utf8)
        XCTAssertThrowsError(try installer.install()) {
            XCTAssertEqual($0 as? ClaudeInstaller.Failure, .occupied(installer.link))
        }
        XCTAssertEqual(try read(installer.link), "my notes")
        XCTAssertFalse(fm.fileExists(atPath: installer.claudeMD.path))
    }

    func testUsersOwnLinkIsLeftAlone() throws {
        let installer = try makeInstaller()
        let theirs = root.appendingPathComponent("notes/LocalRouter.md")
        try fm.createDirectory(at: theirs.deletingLastPathComponent(), withIntermediateDirectories: true)
        try "theirs".write(to: theirs, atomically: true, encoding: .utf8)
        try fm.createSymbolicLink(at: installer.link, withDestinationURL: theirs)
        XCTAssertThrowsError(try installer.install())
        XCTAssertEqual(try read(installer.link), "theirs")
    }

    // MARK: Uninstall

    func testUninstallRemovesOnlyOurLink() throws {
        let installer = try makeInstaller()
        _ = try installer.install()
        installer.uninstall()
        XCTAssertNil(try? fm.destinationOfSymbolicLink(atPath: installer.link.path))

        try "my notes".write(to: installer.link, atomically: true, encoding: .utf8)
        installer.uninstall()
        XCTAssertEqual(try read(installer.link), "my notes")
    }
}
