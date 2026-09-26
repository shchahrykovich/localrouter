import Foundation
import XCTest
@testable import LocalRouterKit

final class UpdaterTests: XCTestCase {
    func testVersionOrder() {
        XCTAssertTrue(Updater.isNewer("0.1.10", than: "0.1.9"))
        XCTAssertTrue(Updater.isNewer("v1.0.0", than: "0.9.99"))
        XCTAssertTrue(Updater.isNewer("0.2", than: "0.1.9"))
        XCTAssertFalse(Updater.isNewer("0.1.0", than: "0.1.0"))
        XCTAssertFalse(Updater.isNewer("0.1.0-beta", than: "0.1.0"))
        XCTAssertFalse(Updater.isNewer("0.0.9", than: "0.1.0"))
    }

    func testOnlyGitHubHttpsIsTrusted() {
        XCTAssertTrue(Updater.isTrusted(URL(string: "https://github.com/shchahrykovich/localrouter/releases/download/v0.1.0/localrouter-0.1.0.dmg")!))
        XCTAssertTrue(Updater.isTrusted(URL(string: "https://objects.githubusercontent.com/x")!))
        XCTAssertTrue(Updater.isTrusted(URL(string: "https://release-assets.githubusercontent.com/x")!))
        XCTAssertFalse(Updater.isTrusted(URL(string: "http://github.com/x.dmg")!))
        XCTAssertFalse(Updater.isTrusted(URL(string: "https://github.com.evil.example/x.dmg")!))
        XCTAssertFalse(Updater.isTrusted(URL(string: "https://user@github.com/x.dmg")!))
        XCTAssertFalse(Updater.isTrusted(URL(string: "https://github.com:8443/x.dmg")!))
    }

    func testParseLatestRelease() throws {
        let json = """
        {"tag_name":"v0.2.0","body":"Fixes","assets":[
          {"name":"notes.txt","browser_download_url":"https://github.com/a/b/notes.txt"},
          {"name":"localrouter-0.2.0.dmg","browser_download_url":"https://github.com/shchahrykovich/localrouter/releases/download/v0.2.0/localrouter-0.2.0.dmg"}]}
        """
        let r = try Updater.parse(Data(json.utf8))
        XCTAssertEqual(r.version, "0.2.0")
        XCTAssertEqual(r.tag, "v0.2.0")
        XCTAssertEqual(r.dmg.lastPathComponent, "localrouter-0.2.0.dmg")
    }

    func testReleaseWithoutDmgOrWithForeignUrlIsRefused() {
        let noDmg = #"{"tag_name":"v1","assets":[]}"#
        XCTAssertThrowsError(try Updater.parse(Data(noDmg.utf8))) { XCTAssertEqual($0 as? Updater.Failure, .noDmg) }
        let foreign = #"{"tag_name":"v1","assets":[{"name":"a.dmg","browser_download_url":"https://evil.example/a.dmg"}]}"#
        XCTAssertThrowsError(try Updater.parse(Data(foreign.utf8)))
    }

    func testObstacles() {
        let home = URL(fileURLWithPath: "/Users/someone")
        XCTAssertNotNil(Updater.obstacle(appURL: URL(fileURLWithPath: "/tmp/build/LocalRouter"), home: home))
        XCTAssertNotNil(Updater.obstacle(appURL: URL(fileURLWithPath: "/Users/someone/Downloads/LocalRouter.app"), home: home))
    }

    func testInstallScriptQuotesPathsAndChecksTheTeam() {
        let script = Updater.installScript(
            dmg: URL(fileURLWithPath: "/tmp/it's here/LocalRouter-0.2.0.dmg"),
            dest: URL(fileURLWithPath: "/Applications/LocalRouter.app"),
            pid: 4242, team: "7274LDP74B", log: URL(fileURLWithPath: "/tmp/update.log"), uid: 501)
        XCTAssertTrue(script.contains("DMG='/tmp/it'\\''s here/LocalRouter-0.2.0.dmg'"), script)
        XCTAssertTrue(script.contains("TEAM='7274LDP74B'"))
        XCTAssertTrue(script.contains("codesign --verify --deep --strict"))
        XCTAssertTrue(script.contains("launchctl kickstart -k \"gui/501/$LABEL\""))
        XCTAssertTrue(script.contains("LABEL='dev.localrouter.app.daemon'"))
        XCTAssertTrue(script.contains("xattr -dr com.apple.quarantine"))
    }

    func testInstallScriptIsValidBash() throws {
        let script = Updater.installScript(dmg: URL(fileURLWithPath: "/tmp/a.dmg"), dest: URL(fileURLWithPath: "/Applications/LocalRouter.app"),
                                           pid: 1, team: nil, log: URL(fileURLWithPath: "/tmp/u.log"))
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("lr-script-\(UUID().uuidString).sh")
        try script.write(to: file, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: file) }
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/bash")
        p.arguments = ["-n", file.path]
        try p.run()
        p.waitUntilExit()
        XCTAssertEqual(p.terminationStatus, 0)
    }
}

final class CLIInstallerTests: XCTestCase {
    private var dir: URL!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("lr-cli-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dir)
    }

    private func fakeTool(in app: String) throws -> URL {
        let tool = dir.appendingPathComponent("\(app)/\(BundleLayout.cli)")
        try FileManager.default.createDirectory(at: tool.deletingLastPathComponent(), withIntermediateDirectories: true)
        try "#!/bin/sh\n".write(to: tool, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: tool.path)
        return tool
    }

    func testLinksIntoTheBinFolderAndIsIdempotent() throws {
        let tool = try fakeTool(in: "LocalRouter.app")
        let bin = dir.appendingPathComponent("bin")
        let installer = CLIInstaller(tool: tool, binDir: bin, pathVariable: bin.path)
        XCTAssertEqual(try installer.install(), .installed(link: installer.link, onPath: true))
        XCTAssertEqual(try FileManager.default.destinationOfSymbolicLink(atPath: installer.link.path), tool.path)
        XCTAssertEqual(try installer.install(), .installed(link: installer.link, onPath: true))
    }

    func testReplacesALinkToAnotherCopyOfTheApp() throws {
        let old = try fakeTool(in: "Old/LocalRouter.app")
        let new = try fakeTool(in: "LocalRouter.app")
        let bin = dir.appendingPathComponent("bin")
        _ = try CLIInstaller(tool: old, binDir: bin, pathVariable: "").install()
        let installer = CLIInstaller(tool: new, binDir: bin, pathVariable: "")
        _ = try installer.install()
        XCTAssertEqual(try FileManager.default.destinationOfSymbolicLink(atPath: installer.link.path), new.path)
    }

    func testNeverReplacesAForeignFile() throws {
        let tool = try fakeTool(in: "LocalRouter.app")
        let bin = dir.appendingPathComponent("bin")
        try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
        try "mine".write(to: bin.appendingPathComponent("localrouter"), atomically: true, encoding: .utf8)
        let installer = CLIInstaller(tool: tool, binDir: bin, pathVariable: "")
        XCTAssertThrowsError(try installer.install()) { XCTAssertEqual($0 as? CLIInstaller.Failure, .occupied(installer.link)) }
    }

    func testMissingToolIsReported() {
        let installer = CLIInstaller(tool: dir.appendingPathComponent("nope"), binDir: dir, pathVariable: "")
        XCTAssertThrowsError(try installer.install())
    }
}

final class BundleLayoutTests: XCTestCase {
    /// macOS file systems ignore case by default: two programs whose paths differ
    /// only in case are one file, and copying the second overwrites the first.
    func testBundledProgramsNeverCollideWhenCaseIsIgnored() {
        let paths = [BundleLayout.appExecutable, BundleLayout.daemon, BundleLayout.cli]
        XCTAssertEqual(Set(paths.map { $0.lowercased() }).count, paths.count, "\(paths)")
    }
}
