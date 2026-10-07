import LocalRouterKit
import SwiftUI

/// The forward proxy (ADR 06) at a glance: on and off, its address, how to
/// send traffic through it, and the latest requests it carried. The inspect
/// list, the inspection CA and script rules are in the Settings window.
struct ProxyView: View {
    @Environment(AppModel.self) private var model
    let openSettings: (SettingsPage?) -> Void
    /// Opens the Traffic tab with only the proxy's requests.
    let showTraffic: () -> Void
    /// The phone panel (ADR 10) replaces the tab while it is open.
    @State private var showingPhone = false

    var body: some View {
        if model.status?.proxy == nil {
            Text(model.status == nil ? "The daemon is not running." : "This daemon has no forward proxy. Restart it after an update.")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if showingPhone {
            PhoneView { showingPhone = false }
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    WaitingDevicesBanner()
                    proxyCard
                    SectionLabel("Send traffic through it")
                    tiles
                    CopyLine(text: "eval \"$(\(Instance.current.cli) proxy env)\" && npm test", prompt: "$")
                    latest
                }
                .padding(.horizontal, 14)
                .padding(.bottom, 12)
            }
        }
    }

    // MARK: Switch, address, log

    private var proxyCard: some View {
        let p = model.proxy
        return Card {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Forward proxy").fontWeight(.semibold)
                    HStack(spacing: 4) {
                        if let p, !p.bound.isEmpty {
                            Text(p.url).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.accentSoftText)
                                .textSelection(.enabled)
                            IconButton(symbol: "doc.on.doc", help: "Copy the proxy address", size: 22) { model.copy(p.url) }
                        } else if let p {
                            Text(p.enabled ? "not listening" : "off, port \(p.port)")
                                .font(.system(size: 12, design: .monospaced)).foregroundStyle(.secondary)
                        }
                    }
                    .frame(minHeight: 22)
                }
                Spacer()
                Toggle("Forward proxy", isOn: Binding(
                    get: { p?.enabled ?? false },
                    set: { on in Task { await model.setProxyEnabled(on) } }))
                    .toggleStyle(.switch)
                    .labelsHidden()
                    .tint(Theme.online)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            ForEach(p?.errors ?? [], id: \.self) { e in
                Text(e).foregroundStyle(Theme.dangerText).font(.caption).padding(.horizontal, 12).padding(.bottom, 8)
            }
            // ADR 08: where every proxied request goes.
            if let line = ProxyLog.stateLine(p?.log) {
                Divider()
                HStack(spacing: 8) {
                    Image(systemName: "doc.text").foregroundStyle(.secondary)
                    Text(line).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    Spacer(minLength: 4)
                    LinkButton("Open log") { Task { await model.openProxyLog() } }
                    LinkButton("Show folder") { model.showProxyLogFolder() }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                ForEach(ProxyLog.problems(p?.log), id: \.self) { e in
                    Text(e).foregroundStyle(Theme.dangerText).font(.caption).padding(.horizontal, 12).padding(.bottom, 8)
                }
            }
        }
    }

    // MARK: Chrome, iPhone, HTTPS

    private var tiles: some View {
        HStack(spacing: 6) {
            if model.chromeInstalled {
                Tile(symbol: "globe", title: "Open Chrome", help: "Start a separate Chrome that uses the proxy") {
                    Task { await model.openChromeViaProxy() }
                }
            }
            Tile(symbol: "iphone", title: "Set up iPhone", help: "Send an iPhone's traffic through the proxy: a QR code to scan") {
                showingPhone = true
            }
            Tile(symbol: "lock", title: TrafficFormat.inspection(hosts: model.proxy?.inspectHosts ?? []),
                 help: "The hosts whose HTTPS the proxy reads") {
                openSettings(.inspection)
            }
        }
    }

    // MARK: Latest requests

    @ViewBuilder private var latest: some View {
        let entries = model.logs.filter(\.isProxied).prefix(3)
        SectionLabel(text: "Latest") { LinkButton("All in Traffic", action: showTraffic) }
            .padding(.top, 2)
        if entries.isEmpty {
            Text("No requests through the proxy yet.").font(.callout).foregroundStyle(.secondary).padding(.horizontal, 4)
        } else {
            Card {
                ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
                    if index > 0 { Divider().opacity(0.5) }
                    TrafficRow(entry: entry, compact: true)
                }
            }
        }
    }
}

/// A button with an icon above its title, one of three in a row.
private struct Tile: View {
    let symbol: String
    let title: String
    let help: String
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: 6) {
                Image(systemName: symbol).font(.system(size: 14)).foregroundStyle(Theme.accent)
                Text(title).font(.system(size: 12, weight: .medium)).lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 10)
            .padding(.vertical, 9)
            .background(hovering ? Theme.rowHover : Theme.card, in: RoundedRectangle(cornerRadius: 9))
            .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(Theme.line, lineWidth: 0.5))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(help)
    }
}
