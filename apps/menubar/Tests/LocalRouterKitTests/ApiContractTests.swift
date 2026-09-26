// T9 (Swift half), I11: every file in api/examples decodes into its Swift type
// and encodes back to the same JSON. The Rust side runs the same check.

import Foundation
import XCTest
@testable import LocalRouterKit

final class ApiContractTests: XCTestCase {
    private var examples: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // LocalRouterKitTests
            .deletingLastPathComponent() // Tests
            .deletingLastPathComponent() // menubar
            .deletingLastPathComponent() // apps
            .deletingLastPathComponent() // repo
            .appendingPathComponent("api/examples")
    }

    private static let methods = [
        "hello", "status", "register_route", "unregister_route", "list_routes", "find_free_port",
        "get_logs", "subscribe_logs", "get_config", "set_config", "reset_ca",
    ]

    /// Decode, encode again, and compare with the file (nulls and default
    /// values that both sides omit are ignored).
    private func roundTrip<T: Codable>(_ type: T.Type, _ json: Any, _ file: String) throws {
        let data = try JSONSerialization.data(withJSONObject: json)
        let value: T
        do {
            value = try Api.decoder.decode(T.self, from: data)
        } catch {
            XCTFail("\(file): \(error)")
            return
        }
        let back = try JSONSerialization.jsonObject(with: Api.encoder.encode(value))
        XCTAssertEqual(normalize(json) as? NSObject, normalize(back) as? NSObject, "\(file): round trip differs")
    }

    private func normalize(_ v: Any) -> Any {
        if let dict = v as? [String: Any] {
            var out: [String: Any] = [:]
            for (k, value) in dict {
                if value is NSNull { continue }
                if k == "listen_failed", let b = value as? Bool, !b { continue }
                out[k] = normalize(value)
            }
            return out
        }
        if let array = v as? [Any] { return array.map(normalize) }
        return v
    }

    private func params(_ method: String, _ json: Any, _ file: String) throws {
        switch method {
        case "hello": try roundTrip(HelloParams.self, json, file)
        case "register_route": try roundTrip(Route.self, json, file)
        case "unregister_route": try roundTrip(HostParams.self, json, file)
        case "find_free_port": try roundTrip(FindFreePortParams.self, json, file)
        case "get_logs": try roundTrip(GetLogsParams.self, json, file)
        case "subscribe_logs": try roundTrip(SubscribeLogsParams.self, json, file)
        case "set_config": try roundTrip(SetConfigParams.self, json, file)
        case "status", "list_routes", "get_config", "reset_ca": try roundTrip(Empty.self, json, file)
        default: XCTFail("\(file): unknown method \(method)")
        }
    }

    private func result(_ method: String, _ json: Any, _ file: String) throws {
        switch method {
        case "hello": try roundTrip(HelloResult.self, json, file)
        case "status": try roundTrip(StatusResult.self, json, file)
        case "register_route": try roundTrip(RegisterRouteResult.self, json, file)
        case "unregister_route": try roundTrip(UnregisterRouteResult.self, json, file)
        case "list_routes": try roundTrip(ListRoutesResult.self, json, file)
        case "find_free_port": try roundTrip(FindFreePortResult.self, json, file)
        case "get_logs": try roundTrip(GetLogsResult.self, json, file)
        case "subscribe_logs": try roundTrip(SubscribeLogsResult.self, json, file)
        case "get_config": try roundTrip(Config.self, json, file)
        case "set_config": try roundTrip(SetConfigResult.self, json, file)
        case "reset_ca": try roundTrip(ResetCaResult.self, json, file)
        default: XCTFail("\(file): unknown method \(method)")
        }
    }

    func testEveryExampleDecodes() throws {
        let files = try FileManager.default.contentsOfDirectory(atPath: examples.path).filter { $0.hasSuffix(".json") }
        XCTAssertGreaterThanOrEqual(files.count, 20)
        var seen = Set<String>()
        for file in files {
            let json = try JSONSerialization.jsonObject(with: Data(contentsOf: examples.appendingPathComponent(file)))
            let stem = String(file.dropLast(".json".count))
            guard let dot = stem.lastIndex(of: ".") else { XCTFail("\(file): name it <method>.<kind>.json"); continue }
            let name = String(stem[..<dot]), kind = String(stem[stem.index(after: dot)...])
            let object = json as? [String: Any] ?? [:]
            switch kind {
            case "request":
                let method = object["method"] as? String ?? ""
                seen.insert(method)
                try params(method, object["params"] ?? [:], file)
            case "reply" where name == "error":
                try roundTrip(ApiError.self, object["error"] ?? [:], file)
            case "reply":
                guard let method = Self.methods.first(where: { name == $0 || name.hasPrefix($0 + "_") }) else {
                    XCTFail("\(file): unknown method"); continue
                }
                try result(method, object["result"] ?? [:], file)
            case "event":
                try roundTrip(LogEvent.self, json, file)
            default:
                XCTFail("\(file): unknown kind \(kind)")
            }
        }
        for m in Self.methods { XCTAssertTrue(seen.contains(m), "no request example for \(m)") }
    }
}
