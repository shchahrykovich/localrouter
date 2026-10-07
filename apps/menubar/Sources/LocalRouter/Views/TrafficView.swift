import LocalRouterKit
import SwiftUI

/// The Traffic tab: the requests of the routes and of the forward proxy, in
/// columns, newest first.
struct TrafficView: View {
    @Environment(AppModel.self) private var model
    @Binding var scope: TrafficScope
    @State private var filter = ""

    private var shown: [LogEntry] { model.logs.filter { $0.matches(filter: filter, scope: scope) } }

    var body: some View {
        VStack(spacing: 10) {
            HStack(spacing: 8) {
                FilterField(prompt: "Filter by host", text: $filter)
                Segments(items: TrafficScope.allCases, selection: $scope, fill: false) { item, _ in Text(item.rawValue) }
                    .fixedSize()
            }
            .padding(.horizontal, 14)

            Card {
                TrafficHeader(compact: false)
                Divider()
                let entries = shown
                if entries.isEmpty {
                    Text(emptyText)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            ForEach(entries) { entry in
                                TrafficRow(entry: entry, compact: false)
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
    }

    private var emptyText: String {
        if !model.running { return "The daemon is not running." }
        if model.logs.isEmpty { return "No requests yet." }
        return "No requests match."
    }
}

/// The column widths, shared by the header and the rows.
private enum Columns {
    static let time: CGFloat = 56
    static let status: CGFloat = 46
    static let method: CGFloat = 50
    static let took: CGFloat = 54
    static let spacing: CGFloat = 8
}

struct TrafficHeader: View {
    let compact: Bool

    var body: some View {
        HStack(spacing: Columns.spacing) {
            if !compact { Text("Time").frame(width: Columns.time, alignment: .leading) }
            Text("Status").frame(width: Columns.status, alignment: .leading)
            Text("Method").frame(width: Columns.method, alignment: .leading)
            Text("Host and path").frame(maxWidth: .infinity, alignment: .leading)
            Text("Took").frame(width: Columns.took, alignment: .trailing)
        }
        .font(.system(size: 10, weight: .semibold))
        .lineLimit(1)
        .textCase(.uppercase)
        .kerning(0.3)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }
}

/// One request. `compact` leaves out the time, for the Proxy tab.
struct TrafficRow: View {
    let entry: LogEntry
    let compact: Bool

    private static let clock: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss"
        return f
    }()

    private func time(_ ms: UInt64) -> String {
        Self.clock.string(from: Date(timeIntervalSince1970: TimeInterval(ms) / 1000))
    }

    var body: some View {
        switch entry {
        case let .http(t, method, host, path, status, duration, route, proxy, scripts):
            row(time: t, status: Chip(text: "\(status)", kind: kind(StatusKind(status: status)), mono: true),
                method: method, duration: duration, failed: status >= 500) {
                Text(host + path).lineLimit(1).truncationMode(.middle)
                if let proxy {
                    // ADR 06: traffic of the forward proxy, and how it was carried.
                    Chip(text: proxy.mode == "tunnel" ? "tunnel" : "proxy", kind: .proxy)
                        .help("Through the proxy (\(proxy.mode))")
                    if let bytesIn = proxy.bytesIn, let bytesOut = proxy.bytesOut {
                        Text("↑\(TrafficFormat.bytes(bytesIn)) ↓\(TrafficFormat.bytes(bytesOut))").foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                if let scripts {
                    // ADR 07: the script rules that ran, red when one failed.
                    Chip(text: scripts.rules.joined(separator: ","), kind: scripts.error == nil ? .accent : .danger)
                        .help(scripts.error.map { "Script rule \($0) failed" } ?? "Script rules that ran")
                }
            }
            .help(route.map { "Answered by the route \($0)" } ?? "")
        case let .tcp(t, host, port, bytesIn, bytesOut, duration, failed):
            row(time: t, status: Chip(text: failed ? "FAIL" : "TCP", kind: failed ? .danger : .neutral, mono: true),
                method: "–", duration: duration, failed: failed) {
                Text("\(host):\(port)").lineLimit(1).truncationMode(.middle)
                Text("↑\(TrafficFormat.bytes(bytesIn)) ↓\(TrafficFormat.bytes(bytesOut))").foregroundStyle(.secondary).lineLimit(1)
            }
        }
    }

    private func row<Middle: View>(time t: UInt64, status: Chip, method: String, duration: UInt64, failed: Bool,
                                   @ViewBuilder middle: () -> Middle) -> some View {
        HStack(spacing: Columns.spacing) {
            if !compact { Text(time(t)).foregroundStyle(.secondary).frame(width: Columns.time, alignment: .leading) }
            status.frame(width: Columns.status, alignment: .leading)
            Text(method).lineLimit(1).frame(width: Columns.method, alignment: .leading)
            HStack(spacing: 6) { middle() }.frame(maxWidth: .infinity, alignment: .leading)
            Text(TrafficFormat.duration(ms: duration)).foregroundStyle(.secondary).frame(width: Columns.took, alignment: .trailing)
        }
        .font(Theme.mono)
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
        .background(failed ? Theme.dangerRow : .clear)
    }

    private func kind(_ status: StatusKind) -> Chip.Kind {
        switch status {
        case .success: .success
        case .neutral: .neutral
        case .clientError: .warning
        case .serverError: .danger
        }
    }
}
