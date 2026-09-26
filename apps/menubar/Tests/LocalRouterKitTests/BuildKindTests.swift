import XCTest
@testable import LocalRouterKit

final class BuildKindTests: XCTestCase {
    func testDeveloperIDSignedIsRelease() {
        XCTAssertEqual(BuildKind.of(teamID: "ABCDE12345"), .release)
    }

    /// Ad-hoc signed or unsigned: no Team ID.
    func testNoTeamIDIsDev() {
        XCTAssertEqual(BuildKind.of(teamID: nil), .dev)
        XCTAssertEqual(BuildKind.of(teamID: ""), .dev)
    }

    /// The test runner is not Developer ID signed, like `swift run`.
    func testUnsignedTestBundleIsDev() {
        XCTAssertEqual(BuildKind.of(teamID: Updater.teamIdentifier(of: URL(fileURLWithPath: "/nonexistent"))), .dev)
    }
}
