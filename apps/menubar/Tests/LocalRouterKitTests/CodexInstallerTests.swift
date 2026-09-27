import XCTest
@testable import LocalRouterKit

final class CodexInstallerTests: XCTestCase {
    private var root: URL!
    private let fm = FileManager.default

    override func setUpWithError() throws {
        root = fm.temporaryDirectory.appendingPathComponent("lr-codex-\(UUID().uuidString)")
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws { try? fm.removeItem(at: root) }

    private func installer(_ instance: Instance = .release, bundle: String = "App") throws -> CodexInstaller {
        let note = root.appendingPathComponent("\(bundle).app").appendingPathComponent(BundleLayout.claudeNote)
        try fm.createDirectory(at: note.deletingLastPathComponent(), withIntermediateDirectories: true)
        try "guide".write(to: note, atomically: true, encoding: .utf8)
        let dir = root.appendingPathComponent("codex home")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        return CodexInstaller(note: note, codexDir: dir, instance: instance)
    }

    private func read(_ file: URL) throws -> String { try String(contentsOf: file, encoding: .utf8) }

    func testInstallPreservesInstructionsAndIsIdempotent() throws {
        let i = try installer()
        let file = try i.instructionsFile()
        let original = "# My instructions\n\nKeep this."
        try original.write(to: file, atomically: true, encoding: .utf8)
        XCTAssertEqual(try i.install(), .installed(link: i.link, agentsMD: file, instructionAdded: true))
        XCTAssertEqual(try read(file), i.instructionLine + "\n\n" + original)
        XCTAssertEqual(try fm.destinationOfSymbolicLink(atPath: i.link.path), i.note.path)
        XCTAssertTrue(i.instructionLine.contains(i.link.path))
        XCTAssertEqual(try i.install(), .installed(link: i.link, agentsMD: file, instructionAdded: false))
        XCTAssertEqual(try read(file), i.instructionLine + "\n\n" + original)
    }

    func testUsesNonemptyOverrideAndPreservesItsSymlink() throws {
        let i = try installer()
        let agents = try i.instructionsFile()
        try "base".write(to: agents, atomically: true, encoding: .utf8)
        let real = root.appendingPathComponent("dotfiles.md")
        try "override".write(to: real, atomically: true, encoding: .utf8)
        let override = i.codexDir.appendingPathComponent("AGENTS.override.md")
        try fm.createSymbolicLink(at: override, withDestinationURL: real)
        XCTAssertEqual(try i.instructionsFile(), override)
        _ = try i.install()
        XCTAssertEqual(try read(real), i.instructionLine + "\n\noverride")
        XCTAssertEqual(try read(agents), "base")
        XCTAssertEqual(try fm.destinationOfSymbolicLink(atPath: override.path), real.path)
    }

    func testEmptyOverrideFallsBackToAgents() throws {
        let i = try installer()
        try " \n".write(to: i.codexDir.appendingPathComponent("AGENTS.override.md"), atomically: true, encoding: .utf8)
        _ = try i.install()
        XCTAssertEqual(try i.instructionsFile().lastPathComponent, "AGENTS.md")
        XCTAssertEqual(try read(i.instructionsFile()), i.instructionLine + "\n\n")
    }

    func testMissingHomeDoesNothing() throws {
        let i = try installer()
        try fm.removeItem(at: i.codexDir)
        XCTAssertEqual(try i.install(), .noCodex(i.codexDir))
        XCTAssertFalse(fm.fileExists(atPath: i.codexDir.path))
    }

    func testMissingNoteDoesNotWriteInstructions() throws {
        let i = try installer()
        try fm.removeItem(at: i.note)
        XCTAssertThrowsError(try i.install())
        XCTAssertFalse(fm.fileExists(atPath: try i.instructionsFile().path))
    }

    func testOccupiedNoteAndForeignSymlinkArePreserved() throws {
        let i = try installer()
        try "mine".write(to: i.link, atomically: true, encoding: .utf8)
        XCTAssertThrowsError(try i.install())
        i.uninstall()
        XCTAssertEqual(try read(i.link), "mine")
        try fm.removeItem(at: i.link)
        let foreign = root.appendingPathComponent("missing.md")
        try fm.createSymbolicLink(at: i.link, withDestinationURL: foreign)
        XCTAssertThrowsError(try i.install())
        i.uninstall()
        XCTAssertEqual(try fm.destinationOfSymbolicLink(atPath: i.link.path), foreign.path)
    }

    func testRelocationReplacesBrokenBundleLink() throws {
        let old = try installer(bundle: "Old")
        _ = try old.install()
        try fm.removeItem(at: old.note)
        let new = try installer(bundle: "New")
        _ = try new.install()
        XCTAssertEqual(try fm.destinationOfSymbolicLink(atPath: new.link.path), new.note.path)
    }

    func testUnreadableInstructionsAreNotOverwritten() throws {
        let i = try installer()
        let file = try i.instructionsFile()
        try fm.createDirectory(at: file, withIntermediateDirectories: true)
        XCTAssertThrowsError(try i.install())
        XCTAssertNil(try? fm.destinationOfSymbolicLink(atPath: i.link.path))
    }

    func testInstancesCoexistAndUninstallOnlyRemovesOwnLink() throws {
        let release = try installer()
        let dev = try installer(Instance(suffix: "-dev"), bundle: "Dev")
        _ = try release.install()
        _ = try dev.install()
        let file = try dev.instructionsFile()
        let before = try read(file)
        XCTAssertTrue(before.contains(release.instructionLine))
        XCTAssertTrue(before.contains(dev.instructionLine))
        dev.uninstall()
        XCTAssertNil(try? fm.destinationOfSymbolicLink(atPath: dev.link.path))
        XCTAssertEqual(try read(release.link), "guide")
        XCTAssertEqual(try read(file), before)
    }

    func testCodexHomeResolution() {
        XCTAssertEqual(CodexInstaller.defaultDirectory(environment: [:], home: root), root.appendingPathComponent(".codex"))
        XCTAssertEqual(CodexInstaller.defaultDirectory(environment: ["CODEX_HOME": ""], home: root), root.appendingPathComponent(".codex"))
        XCTAssertEqual(CodexInstaller.defaultDirectory(environment: ["CODEX_HOME": root.path]).path, root.path)
    }
}
