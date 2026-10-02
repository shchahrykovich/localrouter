import AppKit
import XCTest
@testable import LocalRouterKit

/// A text field handles ⌘A, ⌘C, ⌘V, ⌘X and ⌘Z only through main menu items
/// with these key equivalents. The app has no menu bar of its own (it is an
/// accessory app), so without this menu the keys do nothing.
@MainActor
final class MainMenuTests: XCTestCase {
    private func item(_ menu: NSMenu, _ action: String) -> NSMenuItem? {
        for item in menu.items {
            if item.action == NSSelectorFromString(action) { return item }
            if let sub = item.submenu, let found = self.item(sub, action) { return found }
        }
        return nil
    }

    func testEditKeysHaveMenuItems() throws {
        let menu = MainMenu.make(appName: "LocalRouter-dev")
        let expected: [(action: String, key: String, mods: NSEvent.ModifierFlags)] = [
            ("selectAll:", "a", .command),
            ("copy:", "c", .command),
            ("paste:", "v", .command),
            ("cut:", "x", .command),
            ("undo:", "z", .command),
            ("redo:", "z", [.command, .shift]),
            ("performClose:", "w", .command),
            ("terminate:", "q", .command),
        ]
        for e in expected {
            let found = try XCTUnwrap(item(menu, e.action), "no item for \(e.action)")
            XCTAssertEqual(found.keyEquivalent, e.key, e.action)
            XCTAssertEqual(found.keyEquivalentModifierMask, e.mods, e.action)
            // nil: the action goes to the first responder, the text field.
            XCTAssertNil(found.target, e.action)
        }
    }

    func testQuitNamesTheInstance() throws {
        let quit = try XCTUnwrap(item(MainMenu.make(appName: "LocalRouter-dev"), "terminate:"))
        XCTAssertEqual(quit.title, "Quit LocalRouter-dev")
    }
}
