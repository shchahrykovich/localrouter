// The projection of the traffic map window (ADR 11, change 2): Mercator on
// the crop of the design canvas, the great-circle arc from this Mac to a
// place, and the city of a time zone (where "This Mac" is, without asking the
// network for the public address).
//
// scripts/map/gen-land-dots.mjs uses the same numbers, so the land dots and
// the arcs line up.

import CoreGraphics
import Foundation

public enum MapProjection {
    /// The map in points: longitude −130 to 160 across 760 points.
    public static let width: CGFloat = 760
    /// Latitude 74 at the top to −50 at the bottom.
    public static let height: CGFloat = 446
    public static let west = -130.0, east = 160.0, north = 74.0, south = -50.0

    /// Points per radian: 760 points for 290 degrees of longitude.
    static let scale = Double(width) / ((east - west) * .pi / 180)

    static func mercatorY(_ lat: Double) -> Double {
        // Clamped, so a pole does not give infinity.
        let phi = max(-85, min(85, lat)) * .pi / 180
        return log(tan(.pi / 4 + phi / 2))
    }

    /// The point of a latitude and longitude. London is (340, 137), Sydney
    /// (737, 389). A point outside the crop is outside 0…760 × 0…446.
    public static func point(lat: Double, lon: Double) -> CGPoint {
        let x = scale * (lon - west) * .pi / 180
        let y = scale * (mercatorY(north) - mercatorY(lat))
        return CGPoint(x: x, y: y)
    }

    /// The great-circle path from one place to another, `segments + 1`
    /// points, the first at `from` and the last at `to`.
    public static func arc(from: Coordinate, to: Coordinate, segments: Int = 24) -> [CGPoint] {
        let n = max(1, segments)
        return greatCircle(from: from, to: to, segments: n).map { point(lat: $0.lat, lon: $0.lon) }
    }

    /// Points on the great circle, as d3's `geoInterpolate` makes them.
    public static func greatCircle(from: Coordinate, to: Coordinate, segments: Int) -> [Coordinate] {
        let rad = Double.pi / 180
        let (la1, lo1, la2, lo2) = (from.lat * rad, from.lon * rad, to.lat * rad, to.lon * rad)
        let a = [cos(la1) * cos(lo1), cos(la1) * sin(lo1), sin(la1)]
        let b = [cos(la2) * cos(lo2), cos(la2) * sin(lo2), sin(la2)]
        let dot = max(-1, min(1, a[0] * b[0] + a[1] * b[1] + a[2] * b[2]))
        let d = acos(dot)
        return (0...segments).map { i in
            let t = Double(i) / Double(segments)
            if d < 1e-9 { return from }
            let k1 = sin((1 - t) * d) / sin(d), k2 = sin(t * d) / sin(d)
            let x = k1 * a[0] + k2 * b[0], y = k1 * a[1] + k2 * b[1], z = k1 * a[2] + k2 * b[2]
            return Coordinate(lat: atan2(z, (x * x + y * y).squareRoot()) / rad, lon: atan2(y, x) / rad)
        }
    }

    /// The city of a time zone id, for the "This Mac" point. An unknown id
    /// gives nil: the map then has no point for this Mac.
    public static func place(forTimeZone id: String) -> TimeZonePlace? {
        guard let city = timeZoneCities[id] else { return nil }
        return TimeZonePlace(name: city.0, coordinate: Coordinate(lat: city.1, lon: city.2))
    }

    /// Every city the table knows, by name, for the "This Mac is in" picker.
    public static var knownPlaces: [TimeZonePlace] {
        var seen = Set<String>()
        return timeZoneCities.values
            .filter { seen.insert($0.0).inserted }
            .map { TimeZonePlace(name: $0.0, coordinate: Coordinate(lat: $0.1, lon: $0.2)) }
            .sorted { $0.name < $1.name }
    }

    static let timeZoneCities: [String: (String, Double, Double)] = [
        "Europe/London": ("London", 51.51, -0.13),
        "Europe/Dublin": ("Dublin", 53.35, -6.26),
        "Europe/Lisbon": ("Lisbon", 38.72, -9.14),
        "Europe/Madrid": ("Madrid", 40.42, -3.70),
        "Europe/Paris": ("Paris", 48.86, 2.35),
        "Europe/Brussels": ("Brussels", 50.85, 4.35),
        "Europe/Amsterdam": ("Amsterdam", 52.37, 4.90),
        "Europe/Berlin": ("Berlin", 52.52, 13.40),
        "Europe/Zurich": ("Zurich", 47.38, 8.54),
        "Europe/Rome": ("Rome", 41.90, 12.50),
        "Europe/Vienna": ("Vienna", 48.21, 16.37),
        "Europe/Prague": ("Prague", 50.08, 14.44),
        "Europe/Warsaw": ("Warsaw", 52.23, 21.01),
        "Europe/Stockholm": ("Stockholm", 59.33, 18.07),
        "Europe/Oslo": ("Oslo", 59.91, 10.75),
        "Europe/Copenhagen": ("Copenhagen", 55.68, 12.57),
        "Europe/Helsinki": ("Helsinki", 60.17, 24.94),
        "Europe/Vilnius": ("Vilnius", 54.69, 25.28),
        "Europe/Riga": ("Riga", 56.95, 24.11),
        "Europe/Tallinn": ("Tallinn", 59.44, 24.75),
        "Europe/Minsk": ("Minsk", 53.90, 27.56),
        "Europe/Kyiv": ("Kyiv", 50.45, 30.52),
        "Europe/Kiev": ("Kyiv", 50.45, 30.52),
        "Europe/Moscow": ("Moscow", 55.76, 37.62),
        "Europe/Istanbul": ("Istanbul", 41.01, 28.98),
        "Europe/Athens": ("Athens", 37.98, 23.73),
        "Europe/Bucharest": ("Bucharest", 44.43, 26.10),
        "Europe/Budapest": ("Budapest", 47.50, 19.04),
        "Asia/Nicosia": ("Nicosia", 35.17, 33.36),
        "Asia/Famagusta": ("Famagusta", 35.12, 33.94),
        "Asia/Jerusalem": ("Jerusalem", 31.77, 35.21),
        "Asia/Dubai": ("Dubai", 25.20, 55.27),
        "Asia/Tbilisi": ("Tbilisi", 41.72, 44.79),
        "Asia/Yerevan": ("Yerevan", 40.18, 44.51),
        "Asia/Almaty": ("Almaty", 43.24, 76.89),
        "Asia/Tashkent": ("Tashkent", 41.30, 69.24),
        "Asia/Karachi": ("Karachi", 24.86, 67.01),
        "Asia/Kolkata": ("Mumbai", 19.08, 72.88),
        "Asia/Calcutta": ("Mumbai", 19.08, 72.88),
        "Asia/Dhaka": ("Dhaka", 23.81, 90.41),
        "Asia/Bangkok": ("Bangkok", 13.76, 100.50),
        "Asia/Ho_Chi_Minh": ("Ho Chi Minh City", 10.82, 106.63),
        "Asia/Jakarta": ("Jakarta", -6.21, 106.85),
        "Asia/Singapore": ("Singapore", 1.35, 103.82),
        "Asia/Kuala_Lumpur": ("Kuala Lumpur", 3.14, 101.69),
        "Asia/Manila": ("Manila", 14.60, 120.98),
        "Asia/Hong_Kong": ("Hong Kong", 22.32, 114.17),
        "Asia/Shanghai": ("Shanghai", 31.23, 121.47),
        "Asia/Taipei": ("Taipei", 25.03, 121.57),
        "Asia/Seoul": ("Seoul", 37.57, 126.98),
        "Asia/Tokyo": ("Tokyo", 35.68, 139.69),
        "Australia/Perth": ("Perth", -31.95, 115.86),
        "Australia/Adelaide": ("Adelaide", -34.93, 138.60),
        "Australia/Brisbane": ("Brisbane", -27.47, 153.03),
        "Australia/Sydney": ("Sydney", -33.87, 151.21),
        "Australia/Melbourne": ("Melbourne", -37.81, 144.96),
        "Pacific/Auckland": ("Auckland", -36.85, 174.76),
        "Africa/Cairo": ("Cairo", 30.04, 31.24),
        "Africa/Lagos": ("Lagos", 6.52, 3.38),
        "Africa/Nairobi": ("Nairobi", -1.29, 36.82),
        "Africa/Johannesburg": ("Johannesburg", -26.20, 28.05),
        "Africa/Casablanca": ("Casablanca", 33.57, -7.59),
        "Atlantic/Reykjavik": ("Reykjavik", 64.15, -21.94),
        "America/St_Johns": ("St. John's", 47.56, -52.71),
        "America/Halifax": ("Halifax", 44.65, -63.57),
        "America/New_York": ("New York", 40.71, -74.01),
        "America/Toronto": ("Toronto", 43.65, -79.38),
        "America/Detroit": ("Detroit", 42.33, -83.05),
        "America/Chicago": ("Chicago", 41.88, -87.63),
        "America/Mexico_City": ("Mexico City", 19.43, -99.13),
        "America/Denver": ("Denver", 39.74, -104.99),
        "America/Phoenix": ("Phoenix", 33.45, -112.07),
        "America/Edmonton": ("Edmonton", 53.55, -113.49),
        "America/Los_Angeles": ("Los Angeles", 34.05, -118.24),
        "America/Vancouver": ("Vancouver", 49.28, -123.12),
        "America/Anchorage": ("Anchorage", 61.22, -149.90),
        "Pacific/Honolulu": ("Honolulu", 21.31, -157.86),
        "America/Bogota": ("Bogotá", 4.71, -74.07),
        "America/Lima": ("Lima", -12.05, -77.04),
        "America/Santiago": ("Santiago", -33.45, -70.67),
        "America/Sao_Paulo": ("São Paulo", -23.55, -46.63),
        "America/Argentina/Buenos_Aires": ("Buenos Aires", -34.60, -58.38),
        "America/Buenos_Aires": ("Buenos Aires", -34.60, -58.38),
    ]
}

/// A latitude and longitude in degrees.
public struct Coordinate: Equatable, Hashable, Sendable, Codable {
    public var lat: Double
    public var lon: Double

    public init(lat: Double, lon: Double) {
        self.lat = lat
        self.lon = lon
    }
}

/// The city of a time zone, as "This Mac is in: London" shows it.
public struct TimeZonePlace: Equatable, Hashable, Sendable {
    public var name: String
    public var coordinate: Coordinate
}

/// The land dots of the map, from `land-dots.json`
/// (scripts/map/gen-land-dots.mjs).
public enum LandDots {
    /// The dots in map points. A file that does not parse gives no dots: the
    /// map then shows only the arcs.
    public static func load(from data: Data) -> [CGPoint] {
        struct File: Decodable {
            var dots: [[Double]]
        }
        guard let file = try? JSONDecoder().decode(File.self, from: data) else { return [] }
        return file.dots.compactMap { $0.count == 2 ? CGPoint(x: $0[0], y: $0[1]) : nil }
    }
}
