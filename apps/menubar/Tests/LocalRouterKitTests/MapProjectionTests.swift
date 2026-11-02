import CoreGraphics
import Foundation
import XCTest
@testable import LocalRouterKit

// ADR 11, T13.
final class MapProjectionTests: XCTestCase {
    private func assertNear(_ p: CGPoint, _ x: CGFloat, _ y: CGFloat, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(p.x, x, accuracy: 1, file: file, line: line)
        XCTAssertEqual(p.y, y, accuracy: 1, file: file, line: line)
    }

    func testCitiesLandWhereTheCanvasHasThem() {
        assertNear(MapProjection.point(lat: 51.51, lon: -0.13), 340, 137)
        assertNear(MapProjection.point(lat: -33.87, lon: 151.21), 737, 389)
        // The corners of the crop.
        assertNear(MapProjection.point(lat: 74, lon: -130), 0, 0)
        assertNear(MapProjection.point(lat: -50, lon: 160), 760, 446)
    }

    func testAnArcGoesFromThisMacToThePlace() {
        let london = Coordinate(lat: 51.51, lon: -0.13), tokyo = Coordinate(lat: 35.68, lon: 139.69)
        let arc = MapProjection.arc(from: london, to: tokyo, segments: 20)
        XCTAssertEqual(arc.count, 21)
        assertNear(arc[0], 340, 137)
        let end = MapProjection.point(lat: 35.68, lon: 139.69)
        assertNear(arc[20], end.x, end.y)
        // A great circle from London to Tokyo bends north of both cities.
        XCTAssertLessThan(arc[10].y, min(arc[0].y, arc[20].y))
        // The same place twice is one point repeated.
        XCTAssertEqual(Set(MapProjection.arc(from: london, to: london, segments: 4).map { "\($0)" }).count, 1)
    }

    func testTimeZonesGiveACity() {
        XCTAssertEqual(MapProjection.place(forTimeZone: "Europe/London")?.name, "London")
        XCTAssertEqual(MapProjection.place(forTimeZone: "America/Los_Angeles")?.name, "Los Angeles")
        XCTAssertNil(MapProjection.place(forTimeZone: "Mars/Olympus_Mons"))
        XCTAssertGreaterThanOrEqual(MapProjection.timeZoneCities.count, 60)
        XCTAssertEqual(MapProjection.knownPlaces.count, Set(MapProjection.knownPlaces.map(\.name)).count)
    }

    func testLandDotsLoad() throws {
        XCTAssertEqual(LandDots.load(from: Data(#"{"width":760,"height":446,"dots":[[3,3],[9,3],[1]]}"#.utf8)),
                       [CGPoint(x: 3, y: 3), CGPoint(x: 9, y: 3)])
        XCTAssertEqual(LandDots.load(from: Data("nope".utf8)), [])
        // The committed resource: about 4,000 dots, all inside the map.
        let file = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/LocalRouter/TrafficMap/land-dots.json")
        let dots = LandDots.load(from: try Data(contentsOf: file))
        XCTAssertGreaterThan(dots.count, 3_000)
        XCTAssertTrue(dots.allSatisfy { $0.x >= 0 && $0.x <= MapProjection.width && $0.y >= 0 && $0.y <= MapProjection.height })
        // London is on land.
        XCTAssertTrue(dots.contains { abs($0.x - 340) <= 4 && abs($0.y - 137) <= 4 })
    }
}
