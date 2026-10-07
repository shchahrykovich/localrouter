import CoreImage
import Foundation
import XCTest
@testable import LocalRouterKit

final class PhoneSetupTests: XCTestCase {
    private let home = NetworkStatus(id: "mac:18:35:d1:15:d1:a8", name: "Home", router: "192.168.0.1", interface: "en0",
                                     lanAllowed: true)

    private func setup(proxy: Bool = true, lan: Bool = true, saved: [LanNetwork]? = nil, clients: [ProxyClient] = []) -> PhoneSetup {
        let list = saved ?? [LanNetwork(id: home.id, name: "Home", router: home.router)]
        return PhoneSetup(proxyEnabled: proxy, allowLan: lan, network: home, lanNetworks: list, clients: clients)
    }

    // T12: each missing condition is a row with its fix; all good is ready.
    func testChecklist() {
        let good = setup()
        XCTAssertTrue(good.ready)
        XCTAssertEqual(good.rows.map(\.id), ["proxy", "lan", "network"])
        XCTAssertEqual(setup(proxy: false).rows.first { !$0.ok }?.fix, .turnOnProxy)
        XCTAssertEqual(setup(lan: false).rows.first { !$0.ok }?.fix, .turnOnLan)
        XCTAssertEqual(setup(saved: []).rows.filter { !$0.ok }.map(\.fix), [.allowNetwork])

        let unknown = PhoneSetup(proxyEnabled: true, allowLan: true, network: nil, lanNetworks: [], clients: [])
        let row = unknown.rows.first { $0.id == "network" }!
        XCTAssertFalse(row.ok)
        XCTAssertNil(row.fix, "the app cannot fix an unknown network")
    }

    func testPhoneClientIsTheFirstLanClient() {
        let s = setup(clients: [ProxyClient(name: "chrome", port: 8878), ProxyClient(name: "ipad", port: 8879, lan: true)])
        XCTAssertEqual(s.client, "ipad")
        XCTAssertNil(setup(clients: [ProxyClient(name: "chrome", port: 8878, lan: false)]).client)
    }

    func testNextName() {
        XCTAssertEqual(PhoneSetup.nextName([]), "iphone")
        XCTAssertEqual(PhoneSetup.nextName(["iphone"]), "iphone-2")
        XCTAssertEqual(PhoneSetup.nextName(["iphone", "iphone-2"]), "iphone-3")
    }

    // The banner lists every waiting device, newest first, with its client.
    func testWaitingDevices() throws {
        let json = #"""
        [{"name":"iphone","configured":8880,"port":8880,"bound":[],"errors":[],"lan":true,
          "pending":[{"address":"192.168.0.31","host":"a.example","at_ms":1},{"address":"192.168.0.32","host":"b.example","at_ms":3}]},
         {"name":"chrome","configured":8878,"port":8878,"bound":[],"errors":[]},
         {"name":"ipad","configured":8881,"port":8881,"bound":[],"errors":[],"lan":true,
          "pending":[{"address":"192.168.0.40","host":"c.example","at_ms":2}]}]
        """#
        let clients = try Api.decoder.decode([ProxyClientStatus].self, from: Data(json.utf8))
        let waiting = PhoneSetup.waiting(clients)
        XCTAssertEqual(waiting.map(\.device.address), ["192.168.0.32", "192.168.0.40", "192.168.0.31"])
        XCTAssertEqual(waiting.map(\.client), ["iphone", "ipad", "iphone"])
    }

    // T12: the QR image reads back to the setup URL.
    func testQrRoundTrip() throws {
        let url = "http://192.168.0.10:8880/setup/k7mq-2xph-9tdw-r4nc"
        let image = try XCTUnwrap(PhoneSetup.qrImage(url))
        let detector = try XCTUnwrap(CIDetector(ofType: CIDetectorTypeQRCode, context: nil,
                                                options: [CIDetectorAccuracy: CIDetectorAccuracyHigh]))
        let found = detector.features(in: CIImage(cgImage: image)).compactMap { ($0 as? CIQRCodeFeature)?.messageString }
        XCTAssertEqual(found, [url])
    }
}
