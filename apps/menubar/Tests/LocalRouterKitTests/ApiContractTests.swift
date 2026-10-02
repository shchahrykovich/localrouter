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
        "get_logs", "subscribe_logs", "get_config", "set_config", "reset_ca", "get_proxy", "reset_inspect_ca",
        "set_script_rule", "remove_script_rule", "list_script_rules",
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
        case "set_script_rule": try roundTrip(SetScriptRuleParams.self, json, file)
        case "remove_script_rule": try roundTrip(IdParams.self, json, file)
        case "get_proxy": try roundTrip(GetProxyParams.self, json, file)
        case "status", "list_routes", "get_config", "reset_ca", "reset_inspect_ca", "list_script_rules":
            try roundTrip(Empty.self, json, file)
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
        case "reset_ca", "reset_inspect_ca": try roundTrip(ResetCaResult.self, json, file)
        case "get_proxy": try roundTrip(GetProxyResult.self, json, file)
        case "set_script_rule": try roundTrip(SetScriptRuleResult.self, json, file)
        case "remove_script_rule": try roundTrip(RemoveScriptRuleResult.self, json, file)
        case "list_script_rules": try roundTrip(ListScriptRulesResult.self, json, file)
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

    /// ADR 09: the proxy clients decode from config, status and get_proxy,
    /// and a reply for one client names it.
    func testProxyClientsDecode() throws {
        let result = { (file: String) throws -> Data in
            let reply = try JSONSerialization.jsonObject(with: Data(contentsOf: self.examples.appendingPathComponent(file))) as? [String: Any]
            return try JSONSerialization.data(withJSONObject: reply?["result"] ?? [:])
        }
        let config = try Api.decoder.decode(Config.self, from: result("get_config.reply.json"))
        XCTAssertEqual(config.proxyClients, [ProxyClient(name: "chrome", port: 8878), ProxyClient(name: "agent-1", port: 8879)])
        let status = try Api.decoder.decode(StatusResult.self, from: result("status.reply.json"))
        XCTAssertEqual(status.proxy?.clients?.map(\.port), [8878, nil])
        let one = try Api.decoder.decode(GetProxyResult.self, from: result("get_proxy_client.reply.json"))
        XCTAssertEqual(one.client, "chrome")
        XCTAssertEqual(one.url, "http://127.0.0.1:8878")
        XCTAssertEqual(one.log?.url, "http://proxy.localhost/chrome")
        let main = try Api.decoder.decode(GetProxyResult.self, from: result("get_proxy.reply.json"))
        XCTAssertNil(main.client)
        XCTAssertEqual(main.clients?.first?.errors, [])
        let params = try JSONSerialization.jsonObject(with: Api.encoder.encode(GetProxyParams(client: "chrome"))) as? [String: String]
        XCTAssertEqual(params, ["client": "chrome"])
    }

    /// ADR 06: proxy log entries decode with their mode and bytes, and the
    /// env keys of get_proxy stay as the daemon wrote them.
    func testProxyFieldsDecode() throws {
        let read = { (file: String) throws -> Data in try Data(contentsOf: self.examples.appendingPathComponent(file)) }
        let event = try Api.decoder.decode(LogEvent.self, from: read("log_proxy.event.json"))
        guard case let .http(_, _, _, _, _, _, _, proxy, _) = event.entry else { return XCTFail("not http") }
        XCTAssertEqual(proxy?.mode, "inspect")
        let logs = try JSONSerialization.jsonObject(with: read("get_logs.reply.json")) as? [String: Any]
        let entries = try JSONSerialization.data(withJSONObject: logs?["result"] ?? [:])
        let tunnel = try Api.decoder.decode(GetLogsResult.self, from: entries).entries.compactMap { entry -> ProxyTraffic? in
            if case let .http(_, _, _, _, _, _, _, proxy, _) = entry { return proxy }
            return nil
        }.first
        XCTAssertEqual(tunnel, ProxyTraffic(mode: "tunnel", bytesIn: 1830, bytesOut: 48211))
        let reply = try JSONSerialization.jsonObject(with: read("get_proxy.reply.json")) as? [String: Any]
        let result = try JSONSerialization.data(withJSONObject: reply?["result"] ?? [:])
        let settings = try Api.decoder.decode(GetProxyResult.self, from: result)
        XCTAssertEqual(settings.env["HTTPS_PROXY"], "http://127.0.0.1:8877")
        XCTAssertEqual(settings.env["NODE_EXTRA_CA_CERTS"], settings.inspectCa?.pemPath)
    }

    /// ADR 07: script rules and the rules of a log entry decode, and a
    /// get_proxy reply from a 1.3 daemon has no rules.
    func testScriptRuleFieldsDecode() throws {
        let read = { (file: String) throws -> Data in try Data(contentsOf: self.examples.appendingPathComponent(file)) }
        let event = try Api.decoder.decode(LogEvent.self, from: read("log.event.json"))
        guard case let .http(_, _, _, _, _, _, _, _, scripts) = event.entry else { return XCTFail("not http") }
        XCTAssertEqual(scripts, ScriptRun(rules: ["add-trace", "claude-capture"], error: "add-trace"))
        let reply = try JSONSerialization.jsonObject(with: read("get_proxy.reply.json")) as? [String: Any]
        var result = reply?["result"] as? [String: Any] ?? [:]
        let rules = try Api.decoder.decode(GetProxyResult.self, from: JSONSerialization.data(withJSONObject: result)).scriptRules
        XCTAssertEqual(rules.map(\.rule.id), ["claude-capture", "add-trace"])
        XCTAssertEqual(rules[0].kind, "log")
        XCTAssertFalse(rules[1].rule.enabled)
        result.removeValue(forKey: "script_rules")
        let old = try Api.decoder.decode(GetProxyResult.self, from: JSONSerialization.data(withJSONObject: result))
        XCTAssertEqual(old.scriptRules, [])
    }

    /// ADR 08: the log block and the network decode; a 1.4 reply without
    /// them decodes too, and the app then hides the log controls.
    func testProxyLogAndNetworkDecode() throws {
        let read = { (file: String) throws -> Data in try Data(contentsOf: self.examples.appendingPathComponent(file)) }
        let reply = try JSONSerialization.jsonObject(with: read("get_proxy.reply.json")) as? [String: Any]
        var result = reply?["result"] as? [String: Any] ?? [:]
        let p = try Api.decoder.decode(GetProxyResult.self, from: JSONSerialization.data(withJSONObject: result))
        XCTAssertEqual(p.log?.url, "http://proxy.localhost")
        XCTAssertEqual(p.log?.fileRequests, 5000)
        XCTAssertEqual(p.env["SSL_CERT_FILE"]?.hasSuffix("inspect-ca/bundle.pem"), true)
        XCTAssertEqual(p.env["http_proxy"], p.env["HTTP_PROXY"], "lower and upper case keys stay apart")
        result.removeValue(forKey: "log")
        XCTAssertNil(try Api.decoder.decode(GetProxyResult.self, from: JSONSerialization.data(withJSONObject: result)).log)
        let status = try JSONSerialization.jsonObject(with: read("status.reply.json")) as? [String: Any]
        let s = try Api.decoder.decode(StatusResult.self, from: JSONSerialization.data(withJSONObject: status?["result"] ?? [:]))
        XCTAssertEqual(s.network?.id, "mac:18:35:d1:15:d1:a8")
        XCTAssertEqual(s.network?.lanAllowed, false)
        let config = try JSONSerialization.jsonObject(with: read("get_config.reply.json")) as? [String: Any]
        let c = try Api.decoder.decode(Config.self, from: JSONSerialization.data(withJSONObject: config?["result"] ?? [:]))
        XCTAssertEqual(c.lanNetworks?.first?.name, "Home")
        XCTAssertEqual(c.proxyLogFileMb, 20)
    }
}
