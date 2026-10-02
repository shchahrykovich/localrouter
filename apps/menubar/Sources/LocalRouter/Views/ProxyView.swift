import LocalRouterKit
import SwiftUI

/// The forward proxy (ADR 06) at a glance: on and off, its address, how to
/// start a program through it, and the requests it carried. The inspect list,
/// the inspection CA and script rules are in the Settings window.
struct ProxyView: View {
    @Environment(AppModel.self) private var model
    let openSettings: (SettingsPage?) -> Void

    var body: some View {
        if model.status?.proxy == nil {
            Text(model.status == nil ? "The daemon is not running." : "This daemon has no forward proxy. Restart it after an update.")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            VStack(alignment: .leading, spacing: 0) {
                controls.padding(12)
                Divider()
                traffic
            }
        }
    }

    @ViewBuilder private var controls: some View {
        let p = model.proxy
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Toggle("Forward proxy", isOn: Binding(
                    get: { p?.enabled ?? false },
                    set: { on in Task { await model.setProxyEnabled(on) } }))
                    .toggleStyle(.switch)
                Spacer()
                if let p {
                    Text(p.bound.isEmpty ? (p.enabled ? "not listening" : "off, port \(p.port)") : p.url)
                        .font(.callout.monospaced()).foregroundStyle(.secondary)
                }
            }
            ForEach(p?.errors ?? [], id: \.self) { e in Text(e).foregroundStyle(.red).font(.caption) }
            Text("Start a program through the proxy:").font(.caption).foregroundStyle(.secondary)
            CopyLine(text: "eval \"$(\(Instance.current.cli) proxy env)\" && npm test")
            HStack {
                if model.chromeInstalled {
                    Button("Open Chrome via Proxy") { Task { await model.openChromeViaProxy() } }
                }
                Spacer()
                let hosts = p?.inspectHosts ?? []
                Button(hosts.isEmpty ? "Inspect HTTPS…" : "Inspecting \(hosts.count) host\(hosts.count == 1 ? "" : "s")…") {
                    openSettings(.inspection)
                }
                .help("The hosts whose HTTPS the proxy reads")
            }
        }
    }

    @ViewBuilder private var traffic: some View {
        let entries = model.logs.filter(\.isProxied)
        if entries.isEmpty {
            Text("No requests through the proxy yet.")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            List(entries) { LogRow(entry: $0) }
                .listStyle(.plain)
        }
    }
}
