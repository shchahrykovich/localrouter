import XCTest
@testable import LocalRouterKit

final class TrustActionsTests: XCTestCase {
    func testTrustedCAOffersOnlyUntrust() {
        XCTAssertEqual(TrustActions.for(trusted: true), [.untrust])
    }

    func testUntrustedCAOffersOnlyTrust() {
        XCTAssertEqual(TrustActions.for(trusted: false), [.trust])
    }

    /// The trust check could not run: offer both, the user knows better.
    func testUnknownTrustOffersBoth() {
        XCTAssertEqual(TrustActions.for(trusted: nil), [.trust, .untrust])
    }
}
