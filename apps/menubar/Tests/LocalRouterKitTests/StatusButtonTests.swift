import AppKit
import XCTest
@testable import LocalRouterKit

@MainActor
final class StatusButtonTests: XCTestCase {
    /// macOS 14+ puts the button two levels below the window's content view:
    /// NSStatusBarContentView > NSView > NSStatusBarButton.
    func testFindsButtonTwoLevelsDeep() {
        let content = NSView()
        let middle = NSView()
        let button = NSButton()
        content.addSubview(middle)
        middle.addSubview(button)
        XCTAssertIdentical(StatusButton.find(in: content), button)
    }

    func testContentViewItselfCanBeTheButton() {
        let button = NSButton()
        XCTAssertIdentical(StatusButton.find(in: button), button)
    }

    func testNoButtonGivesNil() {
        let content = NSView()
        content.addSubview(NSView())
        XCTAssertNil(StatusButton.find(in: content))
    }
}
