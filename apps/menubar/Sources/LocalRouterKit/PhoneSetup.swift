import CoreImage
import CoreImage.CIFilterBuiltins
import Foundation

/// The phone panel of the Proxy tab (ADR 10): what is missing before an
/// iPhone can use the proxy, the phone client, and the QR code. No UI here,
/// so it can be tested.
public struct PhoneSetup: Equatable, Sendable {
    /// What a button next to a missing condition does.
    public enum Fix: Equatable, Sendable {
        case turnOnProxy, turnOnLan, allowNetwork
    }

    public struct Row: Equatable, Sendable, Identifiable {
        public var id: String
        public var ok: Bool
        public var text: String
        /// Nil when nothing in the app can fix it (an unknown network).
        public var fix: Fix?
    }

    public var rows: [Row]
    /// The first phone client of the config, if any.
    public var client: String?

    /// Every condition holds: a phone on this Wi-Fi can use the proxy.
    public var ready: Bool { rows.allSatisfy(\.ok) }

    public init(proxyEnabled: Bool, allowLan: Bool, network: NetworkStatus?, lanNetworks: [LanNetwork], clients: [ProxyClient]) {
        let saved = network.flatMap { n in lanNetworks.first { $0.id == n.id } }
        client = clients.first { $0.lan == true }?.name
        var rows = [
            Row(id: "proxy", ok: proxyEnabled, text: proxyEnabled ? "Forward proxy on" : "Forward proxy is off", fix: .turnOnProxy),
            Row(id: "lan", ok: allowLan, text: allowLan ? "LAN access on" : "LAN access is off", fix: .turnOnLan),
        ]
        if let network {
            let name = saved.map { $0.name.isEmpty ? "" : "\($0.name), " } ?? ""
            rows.append(Row(
                id: "network", ok: saved != nil,
                text: saved != nil ? "This network (\(name)router \(network.router)) is allowed"
                    : "This network (router \(network.router)) is not allowed",
                fix: .allowNetwork))
        } else {
            rows.append(Row(id: "network", ok: false, text: "This network cannot be recognised (no router, or a VPN)", fix: nil))
        }
        self.rows = rows
    }

    /// `iphone`, or `iphone-2`, `iphone-3` when the name is taken.
    public static func nextName(_ taken: [String]) -> String {
        if !taken.contains("iphone") { return "iphone" }
        var n = 2
        while taken.contains("iphone-\(n)") { n += 1 }
        return "iphone-\(n)"
    }

    /// Every device waiting for Allow or Deny, with its client, newest first.
    public static func waiting(_ clients: [ProxyClientStatus]) -> [(client: String, device: PendingDevice)] {
        clients.flatMap { c in (c.pending ?? []).map { (client: c.name, device: $0) } }.sorted { $0.device.atMs > $1.device.atMs }
    }

    /// A QR code for `text`: black modules on white, `scale` pixels each,
    /// without smoothing, so a camera reads it in dark mode too.
    public static func qrImage(_ text: String, scale: CGFloat = 8) -> CGImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: scale, y: scale)) else { return nil }
        return CIContext(options: [.useSoftwareRenderer: true]).createCGImage(output, from: output.extent)
    }
}
