import AppKit
import LocalRouterKit
import SwiftUI

/// The Apps view of the Traffic tab (ADR 11, canvas artboard
/// `TrafficApps`): every program that used the network in the last hour,
/// with its bytes in and out and how they went: router, proxy or direct.
struct AppsView: View {
    @Environment(AppModel.self) private var model
    let perform: (AppMenu.Action) -> Void
    @State private var filter = ""
    /// Open rows: app keys and node ids.
    @State private var expanded: Set<String> = []
    /// Lists that show all their rows, not only the first five.
    @State private var showAll: Set<String> = []

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
                AppsHeader()
                Divider()
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

    private func row(_ app: AppTraffic, result: AppTrafficResult) -> some View {
        let open = expanded.contains(app.app)
        return VStack(spacing: 0) {
            HStack(spacing: 8) {
                Disclosure(open: open) { toggle(app.app) }
                AppIconView(app: app, size: 26)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 4) {
                        Text(AppTrafficText.name(app)).font(.system(size: 12, weight: .semibold)).lineLimit(1)
                        ShieldView(verdict: model.traffic.verdict(app))
                    }
                    Text(AppTrafficText.subline(app)).font(.system(size: 10.5)).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 4)
                SparkBars(values: app.spark, width: 46)
                BytesColumns(bytesIn: app.bytesIn, bytesOut: app.bytesOut)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(open ? Theme.rowHover : .clear)
            .contentShape(Rectangle())
            .onTapGesture { if app.kind != .exited { perform(.reviewApp(app.app)) } }
            .help(app.kind == .exited ? "The process ended before its path was read." : "Open the details of \(app.name)")
            if open {
                TrafficNodeList(nodes: TrafficTree.children(of: app, remotes: result.remotes, logs: model.logs), depth: 1,
                                parent: app.app, expanded: $expanded, showAll: $showAll)
                    .background(Theme.field.opacity(0.35))
            }
        }
    }

    private func toggle(_ id: String) {
        if expanded.contains(id) { expanded.remove(id) } else { expanded.insert(id) }
    }
}

/// The list header: the columns of the rows below it.
struct AppsHeader: View {
    var body: some View {
        HStack(spacing: 8) {
            Text("App").frame(maxWidth: .infinity, alignment: .leading).padding(.leading, 46)
            Text("Now").frame(width: 46, alignment: .leading)
            Text("↓ In").frame(width: BytesColumns.width, alignment: .trailing)
            Text("↑ Out").frame(width: BytesColumns.width, alignment: .trailing)
        }
        .font(.system(size: 10, weight: .semibold))
        .textCase(.uppercase)
        .kerning(0.3)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }
}

/// Bytes in and out, one line each, never wrapped.
struct BytesColumns: View {
    static let width: CGFloat = 62
    let bytesIn: UInt64
    let bytesOut: UInt64

    var body: some View {
        Text(TrafficFormat.bytes(bytesIn)).frame(width: Self.width, alignment: .trailing)
            .font(.system(size: 11).monospacedDigit()).lineLimit(1).minimumScaleFactor(0.8)
        Text(TrafficFormat.bytes(bytesOut)).frame(width: Self.width, alignment: .trailing)
            .font(.system(size: 11).monospacedDigit()).foregroundStyle(.secondary).lineLimit(1).minimumScaleFactor(0.8)
    }
}

/// The chevron of a row that opens.
struct Disclosure: View {
    let open: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: open ? "chevron.down" : "chevron.right")
                .font(.system(size: 9, weight: .semibold)).frame(width: 12, height: 20).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
        .help(open ? "Close" : "Open")
    }
}

/// The rows under an app or a node: five at first, then "N more…".
struct TrafficNodeList: View {
    static let shown = 5
    let nodes: [TrafficNode]
    let depth: Int
    let parent: String
    @Binding var expanded: Set<String>
    @Binding var showAll: Set<String>

    var body: some View {
        let all = showAll.contains(parent)
        VStack(spacing: 0) {
            ForEach(all ? nodes : Array(nodes.prefix(Self.shown))) { node in
                Divider().opacity(0.4)
                TrafficNodeRow(node: node, depth: depth, open: expanded.contains(node.id)) {
                    if expanded.contains(node.id) { expanded.remove(node.id) } else { expanded.insert(node.id) }
                }
                if expanded.contains(node.id), !node.children.isEmpty {
                    // AnyView: a view type cannot contain itself.
                    AnyView(TrafficNodeList(nodes: node.children, depth: depth + 1, parent: node.id,
                                            expanded: $expanded, showAll: $showAll))
                }
            }
            if !all && nodes.count > Self.shown {
                Divider().opacity(0.4)
                Button("\(nodes.count - Self.shown) more…") { showAll.insert(parent) }
                    .buttonStyle(.plain)
                    .font(.system(size: 11))
                    .foregroundStyle(Theme.accent)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.leading, 10 + CGFloat(depth) * 16 + 20)
                    .padding(.vertical, 4)
            }
        }
    }
}

/// One process, domain, connection or request under an app.
struct TrafficNodeRow: View {
    let node: TrafficNode
    let depth: Int
    let open: Bool
    let toggle: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            if node.children.isEmpty {
                Color.clear.frame(width: 12, height: 18)
            } else {
                Disclosure(open: open, action: toggle)
            }
            icon.frame(width: 18, height: 18)
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 5) {
                    Text(node.title)
                        .font(node.kind == .address || node.kind == .request ? .system(size: 11, design: .monospaced) : .system(size: 11.5, weight: .medium))
                        .lineLimit(1).truncationMode(.middle)
                    if let via = node.via, node.kind != .address {
                        Chip(text: via.rawValue, kind: via == .router ? .accent : via == .proxy ? .proxy : .outline)
                    }
                }
                if let detail = node.detail {
                    Text(detail).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                }
            }
            Spacer(minLength: 4)
            if node.kind == .request {
                if let status = node.status { Chip(text: "\(status)", kind: kind(status), mono: true) }
            } else {
                BytesColumns(bytesIn: node.bytesIn, bytesOut: node.bytesOut)
            }
        }
        .padding(.leading, 10 + CGFloat(depth) * 16)
        .padding(.trailing, 10)
        .padding(.vertical, 4)
        .contentShape(Rectangle())
        .onTapGesture { if !node.children.isEmpty { toggle() } }
    }

    @ViewBuilder private var icon: some View {
        switch node.kind {
        case .process:
            if let path = node.path {
                Image(nsImage: NSWorkspace.shared.icon(forFile: path)).resizable()
            } else {
                Image(systemName: "gearshape").foregroundStyle(.secondary)
            }
        case .group:
            Image(systemName: node.title == TrafficTree.ipAddresses ? "desktopcomputer"
                  : node.via == .router ? "house" : "globe")
                .font(.system(size: 15)).foregroundStyle(.secondary)
        case .address:
            Image(systemName: "arrow.up.right").font(.system(size: 9)).foregroundStyle(.tertiary)
        case .request:
            Image(systemName: "arrow.left.arrow.right").font(.system(size: 9)).foregroundStyle(.tertiary)
        }
    }

    private func kind(_ status: UInt16) -> Chip.Kind {
        switch StatusKind(status: status) {
        case .success: .success
        case .neutral: .neutral
        case .clientError: .warning
        case .serverError: .danger
        }
    }
}
