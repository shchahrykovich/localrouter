import LocalRouterKit
import SwiftUI

struct LogsView: View {
    @Environment(AppModel.self) private var model
    @State private var filter = ""

    private var shown: [LogEntry] {
        let f = filter.trimmingCharacters(in: .whitespaces).lowercased()
        return f.isEmpty ? model.logs : model.logs.filter { $0.host.contains(f) }
    }

    var body: some View {
        VStack(spacing: 0) {
            TextField("Filter by host", text: $filter)
                .textFieldStyle(.roundedBorder)
                .padding(10)
            if shown.isEmpty {
                Text(model.running ? "No requests yet." : "The daemon is not running.")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(shown) { entry in
                    LogRow(entry: entry)
                }
                .listStyle(.plain)
            }
        }
    }
}

struct LogRow: View {
    let entry: LogEntry

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
            HStack(spacing: 6) {
                Text(time(t)).foregroundStyle(.secondary)
                Text("\(status)").foregroundStyle(status >= 500 ? .red : status >= 400 ? .orange : .green)
                Text(method).frame(width: 52, alignment: .leading)
                Text(host + path).lineLimit(1).truncationMode(.middle)
                Spacer()
                if let route { Text(route).foregroundStyle(.secondary).lineLimit(1).help("The route that answered") }
                if let proxy {
                    // ADR 06: traffic of the forward proxy, and how it was carried.
                    Text("proxy").foregroundStyle(.purple)
                        .help("Through the proxy (\(proxy.mode))")
                    if let bytesIn = proxy.bytesIn, let bytesOut = proxy.bytesOut {
                        Text("↑\(bytesIn) ↓\(bytesOut) B").foregroundStyle(.secondary)
                    }
                }
                if let scripts {
                    // ADR 07: the script rules that ran, red when one failed.
                    Text(scripts.rules.joined(separator: ","))
                        .foregroundStyle(scripts.error == nil ? .teal : .red)
                        .lineLimit(1)
                        .help(scripts.error.map { "Script rule \($0) failed" } ?? "Script rules that ran")
                }
                Text("\(duration) ms").foregroundStyle(.secondary)
            }
            .font(.caption.monospaced())
        case let .tcp(t, host, port, bytesIn, bytesOut, duration, failed):
            HStack(spacing: 6) {
                Text(time(t)).foregroundStyle(.secondary)
                Text(failed ? "FAIL" : "tcp").foregroundStyle(failed ? .red : .blue)
                Text("\(host):\(port)").lineLimit(1)
                Spacer()
                Text("↑\(bytesIn) ↓\(bytesOut) B, \(duration) ms").foregroundStyle(.secondary)
            }
            .font(.caption.monospaced())
        }
    }
}
