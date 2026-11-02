import AppKit
import LocalRouterKit
import SwiftUI

/// The real icon of an app (`NSWorkspace.icon(forFile:)`); a letter tile when
/// there is no path (an exited process, macOS services).
struct AppIconView: View {
    let app: AppTraffic
    var size: CGFloat = 20

    var body: some View {
        if app.kind == .services {
            Image(systemName: "apple.logo").font(.system(size: size * 0.6)).frame(width: size, height: size)
                .foregroundStyle(.secondary)
        } else if app.kind != .exited, app.app.hasPrefix("/") {
            Image(nsImage: NSWorkspace.shared.icon(forFile: app.app)).resizable().frame(width: size, height: size)
        } else {
            Text(String(app.name.prefix(1)).uppercased())
                .font(.system(size: size * 0.5, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: size, height: size)
                .background(Color.gray, in: RoundedRectangle(cornerRadius: size * 0.22))
        }
    }
}

/// The shield of a verdict: grey Apple, green signed, amber one engine, red
/// needs review.
struct ShieldView: View {
    let verdict: AppVerdict?
    var size: CGFloat = 11

    var body: some View {
        if let verdict {
            Image(systemName: symbol(verdict)).font(.system(size: size)).foregroundStyle(color(verdict))
                .help(help(verdict))
        }
    }

    private func symbol(_ v: AppVerdict) -> String {
        switch v {
        case .needsReview: "exclamationmark.shield.fill"
        case .caution: "exclamationmark.shield"
        default: "checkmark.shield"
        }
    }

    private func color(_ v: AppVerdict) -> Color {
        switch v {
        case .apple: .secondary
        case .ok, .trusted: Theme.online
        case .caution: Theme.warning
        case .needsReview: Theme.dangerText
        }
    }

    private func help(_ v: AppVerdict) -> String {
        switch v {
        case .apple: "Apple"
        case .ok: "Signed"
        case .trusted: "Trusted by you"
        case .caution: "One VirusTotal engine flags it"
        case .needsReview: "Needs review"
        }
    }
}

/// Twelve small bars: the app's bytes over the range.
struct SparkBars: View {
    let values: [UInt64]
    var width: CGFloat = 48
    var height: CGFloat = 14

    var body: some View {
        let bars = AppTrafficText.spark(values)
        HStack(alignment: .bottom, spacing: 1) {
            ForEach(Array(bars.enumerated()), id: \.offset) { _, v in
                RoundedRectangle(cornerRadius: 1)
                    .fill(v > 0 ? Theme.accent : Theme.line)
                    .frame(height: max(1.5, height * v))
            }
        }
        .frame(width: width, height: height, alignment: .bottom)
        .accessibilityHidden(true)
    }
}

/// The histogram of a range: received down from the middle line, sent up.
struct TrafficHistogram: View {
    let buckets: [TrafficBucket]
    var height: CGFloat = 40

    var body: some View {
        Canvas { ctx, size in
            guard !buckets.isEmpty else { return }
            let top = Double(buckets.map { max($0.bytesIn, $0.bytesOut) }.max() ?? 0)
            let mid = size.height / 2
            ctx.fill(Path(CGRect(x: 0, y: mid - 0.25, width: size.width, height: 0.5)), with: .color(Theme.line))
            guard top > 0 else { return }
            let w = size.width / CGFloat(buckets.count)
            for (i, b) in buckets.enumerated() {
                let x = CGFloat(i) * w + 0.5
                let up = CGFloat(Double(b.bytesOut) / top) * (mid - 1)
                let down = CGFloat(Double(b.bytesIn) / top) * (mid - 1)
                if up > 0 { ctx.fill(Path(CGRect(x: x, y: mid - up, width: max(1, w - 1), height: up)), with: .color(Theme.proxyText.opacity(0.7))) }
                if down > 0 { ctx.fill(Path(CGRect(x: x, y: mid, width: max(1, w - 1), height: down)), with: .color(Theme.accent)) }
            }
        }
        .frame(height: height)
        .accessibilityLabel("Sent up, received down")
    }
}

/// "router 3.4 KB · proxy 96.0 KB · direct 1.2 MB", the bytes in and out
/// together of each `via` that has any.
struct ViaChips: View {
    let byVia: ByVia

    var body: some View {
        HStack(spacing: 4) {
            ForEach(TrafficVia.allCases, id: \.self) { via in
                let bytes = byVia[via].total
                if bytes > 0 {
                    Chip(text: "\(via.rawValue) \(TrafficFormat.bytes(bytes))", kind: kind(via))
                }
            }
        }
    }

    private func kind(_ via: TrafficVia) -> Chip.Kind {
        switch via {
        case .router: .accent
        case .proxy: .proxy
        case .direct: .outline
        }
    }
}
