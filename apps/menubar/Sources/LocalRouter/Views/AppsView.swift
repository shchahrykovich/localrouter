import LocalRouterKit
import SwiftUI

/// The Apps view of the Traffic tab (ADR 11, canvas artboard
/// `TrafficApps`): every program that used the network in the last hour,
/// with its bytes in and out and how they went: router, proxy or direct.
struct AppsView: View {
    @Environment(AppModel.self) private var model
    let perform: (AppMenu.Action) -> Void
    @State private var filter = ""
    @State private var open: String?

    var body: some View {
        VStack(spacing: 10) {
            if let message = blocked {
                stateCard(message)
            } else if model.samplerState == .off {
                offCard
            } else {
                content
            }
        }
    }

    /// The daemon is not running, or too old for this view (I16).
    private var blocked: String? {
        if !model.running { return "The daemon is not running." }
        return model.appsAvailability.message
    }

    private func stateCard(_ text: String) -> some View {
        Card(padding: 16) {
            Text(text).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .center)
        }
        .frame(maxHeight: .infinity, alignment: .top)
        .padding(.horizontal, 14)
    }

    private var offCard: some View {
        Card(padding: 16) {
            VStack(alignment: .leading, spacing: 10) {
                Label("See which apps use the network", systemImage: "chart.bar").font(.system(size: 13, weight: .semibold))
                Text("\(Instance.current.appName) reads it with nettop, which comes with macOS. Nothing leaves this Mac.")
                    .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                Button("Turn On") { Task { await model.setTraffic(true) } }
                    .buttonStyle(.borderedProminent)
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
        .padding(.horizontal, 14)
    }

    @ViewBuilder private var content: some View {
        let result = model.traffic.hour
        HStack(spacing: 8) {
            FilterField(prompt: "Filter apps or addresses", text: $filter)
            Chip(text: "1h", kind: .neutral).help("The last hour. The map shows longer ranges.")
            Button { perform(.openTrafficMap) } label: { Label("Map", systemImage: "globe.europe.africa") }
                .controlSize(.small)
        }
        .padding(.horizontal, 14)

        if model.samplerState == .restarting {
            Text("Reading traffic again… \(model.status?.trafficError ?? "")")
                .font(.caption).foregroundStyle(Theme.warningText).lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 18)
        }
        if let first = model.traffic.appsToReview.first {
            reviewBanner(first, count: model.traffic.appsToReview.count)
        }
        if let result {
            totals(result)
        }
        Card {
            let apps = shown(result)
            if result == nil {
                placeholder("Reading traffic…")
            } else if apps.isEmpty {
                placeholder(filter.isEmpty ? "No traffic in the last hour." : "No apps match.")
            } else {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(apps) { app in
                            row(app, result: result!)
                            Divider().opacity(0.5)
                        }
                    }
                }
            }
        }
        .frame(maxHeight: .infinity)
        .padding(.horizontal, 14)
        .padding(.bottom, 12)
    }

    private func placeholder(_ text: String) -> some View {
        Text(text).foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func shown(_ result: AppTrafficResult?) -> [AppTraffic] {
        guard let result else { return [] }
        let q = filter.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return result.apps }
        return result.apps.filter { app in
            app.name.lowercased().contains(q) || app.app.lowercased().contains(q)
                || AppTrafficText.remotes(of: app.app, in: result).contains { AppTrafficText.matches($0, appName: app.name, query: q) }
        }
    }

    private func reviewBanner(_ app: AppTraffic, count: Int) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.shield.fill").foregroundStyle(Theme.dangerText)
            Text(count == 1 ? "\(app.name) needs review." : "\(count) programs need review.")
                .font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.dangerText)
            Spacer()
            Button("Review") { perform(.reviewApp(app.app)) }.controlSize(.small)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Theme.dangerSoft, in: RoundedRectangle(cornerRadius: 8))
        .padding(.horizontal, 14)
    }

    private func totals(_ r: AppTrafficResult) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Text("↓ \(TrafficFormat.bytes(r.totals.bytesIn))").font(.system(size: 12, weight: .semibold))
                Text("↑ \(TrafficFormat.bytes(r.totals.bytesOut))").font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.secondary)
                Spacer()
                ViaChips(byVia: r.totals.byVia)
            }
            TrafficHistogram(buckets: r.buckets, height: 28)
        }
        .padding(.horizontal, 18)
    }

    @ViewBuilder private func row(_ app: AppTraffic, result: AppTrafficResult) -> some View {
        let expanded = open == app.app
        HStack(spacing: 8) {
            Button { open = expanded ? nil : app.app } label: {
                Image(systemName: expanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 9, weight: .semibold)).frame(width: 12, height: 20).contentShape(Rectangle())
            }
            .buttonStyle(.plain).foregroundStyle(.secondary).help(expanded ? "Hide addresses" : "Show addresses")
            AppIconView(app: app, size: 20)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    Text(AppTrafficText.name(app)).font(.system(size: 12, weight: .medium)).lineLimit(1)
                    ShieldView(verdict: model.traffic.verdict(app))
                }
                Text(AppTrafficText.subline(app)).font(.system(size: 10.5)).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 4)
            SparkBars(values: app.spark)
            Text("↓ \(TrafficFormat.bytes(app.bytesIn))").frame(width: 66, alignment: .trailing)
            Text("↑ \(TrafficFormat.bytes(app.bytesOut))").foregroundStyle(.secondary).frame(width: 62, alignment: .trailing)
        }
        .font(.system(size: 11, design: .monospaced))
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .contentShape(Rectangle())
        .onTapGesture { if app.kind != .exited { perform(.reviewApp(app.app)) } }
        .help(app.kind == .exited ? "The process ended before its path was read." : "Open the details of \(app.name)")
        if expanded {
            let remotes = AppTrafficText.remotes(of: app.app, in: result)
            VStack(spacing: 0) {
                ForEach(Array(remotes.prefix(30).enumerated()), id: \.offset) { _, r in
                    HStack(spacing: 6) {
                        VStack(alignment: .leading, spacing: 0) {
                            Text(AppTrafficText.label(r)).lineLimit(1).truncationMode(.middle)
                            Text(AppTrafficText.detail(r)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                        }
                        Spacer()
                        Chip(text: r.via.rawValue, kind: r.via == .router ? .accent : r.via == .proxy ? .proxy : .outline)
                        Text("↓ \(TrafficFormat.bytes(r.bytesIn))").frame(width: 66, alignment: .trailing)
                        Text("↑ \(TrafficFormat.bytes(r.bytesOut))").foregroundStyle(.secondary).frame(width: 62, alignment: .trailing)
                    }
                    .font(.system(size: 10.5, design: .monospaced))
                    .padding(.leading, 40).padding(.trailing, 10).padding(.vertical, 3)
                }
                if remotes.count > 30 {
                    Text("and \(remotes.count - 30) more: open the map for all of them").font(.caption).foregroundStyle(.secondary)
                        .padding(.vertical, 3)
                }
            }
            .background(Theme.field.opacity(0.5))
        }
    }
}
