import AppKit
import LocalRouterKit
import SwiftUI

/// The traffic map (ADR 11, canvas artboard `Map`): apps on the left, a dot
/// map with an arc from this Mac to each place in the middle, the summary
/// on the right. Drawn with a Canvas from a dots file in the bundle: no map
/// tiles are fetched.
struct TrafficMapView: View {
    @Environment(AppModel.self) private var model
    let openApp: (String) -> Void
    @State private var range: TrafficRange = .hour
    @State private var query = ""
    @State private var selected: String?
    @State private var result: AppTrafficResult?
    @State private var problem: String?
    @State private var zoom: CGFloat = 1
    @State private var pan: CGSize = .zero
    @State private var dragStart: CGSize = .zero

    private static let dots: [CGPoint] = {
        var url = Bundle.main.url(forResource: "land-dots", withExtension: "json")
        if url == nil {
            // `swift run` outside a bundle: the file next to the sources.
            let local = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
                .appendingPathComponent("TrafficMap/land-dots.json")
            if FileManager.default.fileExists(atPath: local.path) { url = local }
        }
        guard let url, let data = try? Data(contentsOf: url) else { return [] }
        return LandDots.load(from: data)
    }()

    private var appNames: [String: String] {
        Dictionary((result?.apps ?? []).map { ($0.app, AppTrafficText.name($0)) }, uniquingKeysWith: { a, _ in a })
    }

    /// The remotes of the selected app, filtered by the search.
    private var remotes: [RemoteTraffic] {
        let names = appNames
        return (result?.remotes ?? []).filter { r in
            (selected == nil || r.app == selected) && AppTrafficText.matches(r, appName: names[r.app] ?? r.app, query: query)
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            toolbar
            Divider()
            if let problem {
                Text(problem).foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                HStack(spacing: 0) {
                    appList.frame(width: 220)
                    Divider()
                    center.frame(maxWidth: .infinity)
                    Divider()
                    summary.frame(width: 230)
                }
            }
        }
        .background(Theme.ground)
        .tint(Theme.accentFill)
        .task(id: range) {
            while !Task.isCancelled {
                await load()
                try? await Task.sleep(for: .seconds(5))
            }
        }
    }

    private func load() async {
        guard model.running else {
            problem = "The daemon is not running."
            return
        }
        if let message = model.appsAvailability.message {
            problem = message
            return
        }
        do {
            result = try await model.client.appTraffic(range: range)
            problem = nil
        } catch {
            problem = error.localizedDescription
        }
    }

    // MARK: Toolbar

    private var toolbar: some View {
        HStack(spacing: 10) {
            Segments(items: TrafficRange.allCases, selection: $range, fill: false) { r, _ in Text(r.label) }.fixedSize()
            Spacer()
            if model.samplerState == .off {
                Text("Traffic is off").font(.caption).foregroundStyle(Theme.warningText)
                Button("Turn On") { Task { await model.setTraffic(true) } }.controlSize(.small)
            }
            FilterField(prompt: "App, address, owner or country", text: $query).frame(width: 260)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    // MARK: Left: apps

    private var appList: some View {
        let apps = result?.apps ?? []
        let off = AppTrafficText.notOnMap(selected == nil ? (result?.remotes ?? []) : remotes)
        return VStack(alignment: .leading, spacing: 0) {
            List(selection: Binding(get: { selected ?? "" }, set: { selected = ($0?.isEmpty ?? true) ? nil : $0 })) {
                HStack {
                    Text("All apps").font(.system(size: 12, weight: .medium))
                    Spacer()
                    Text(TrafficFormat.bytes(result?.totals.bytesIn ?? 0)).font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                .tag("")
                ForEach(apps) { app in
                    HStack(spacing: 6) {
                        AppIconView(app: app, size: 16)
                        Text(AppTrafficText.name(app)).lineLimit(1)
                        if model.traffic.verdict(app) == .needsReview { ShieldView(verdict: .needsReview) }
                        Spacer()
                        Text(TrafficFormat.bytes(app.bytesIn)).font(.caption.monospaced()).foregroundStyle(.secondary)
                    }
                    .font(.system(size: 12))
                    .tag(app.app)
                    .contextMenu { if app.kind != .exited { Button("Details…") { openApp(app.app) } } }
                }
            }
            .listStyle(.sidebar)
            Divider()
            VStack(alignment: .leading, spacing: 2) {
                Text("Not on the map").font(.system(size: 11, weight: .semibold)).foregroundStyle(.secondary)
                Text("router \(TrafficFormat.bytes(off.router)): stays on this Mac").font(.caption)
                Text("no place \(TrafficFormat.bytes(off.noPlace))").font(.caption)
            }
            .padding(10)
        }
    }

    // MARK: Center: map, histogram, table

    private var center: some View {
        let places = AppTrafficText.places(remotes, appNames: appNames)
        return VStack(spacing: 10) {
            ZStack(alignment: .bottomTrailing) {
                MapCanvas(dots: Self.dots, places: places, home: model.traffic.thisMac?.coordinate, zoom: zoom, pan: pan)
                    .background(Theme.card, in: RoundedRectangle(cornerRadius: 10))
                    .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Theme.line, lineWidth: 0.5))
                    .gesture(DragGesture()
                        .onChanged { g in pan = CGSize(width: dragStart.width + g.translation.width, height: dragStart.height + g.translation.height) }
                        .onEnded { _ in dragStart = pan })
                VStack(spacing: 4) {
                    IconButton(symbol: "plus", help: "Zoom in") { zoom = min(zoom * 1.5, 6) }
                    IconButton(symbol: "minus", help: "Zoom out") {
                        zoom = max(zoom / 1.5, 1)
                        if zoom == 1 { pan = .zero; dragStart = .zero }
                    }
                }
                .padding(6)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                .padding(8)
            }
            .frame(minHeight: 280)
            if result?.places == false {
                Text("Places are off. Turn on Show places on the map in Settings › Traffic to put addresses on the map.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let buckets = result?.buckets {
                TrafficHistogram(buckets: buckets, height: 44).padding(.horizontal, 4)
            }
            placesTable(places)
            HStack {
                Text("Places are city-level guesses. A CDN address often shows your own city.").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Link("IP geolocation by DB-IP", destination: URL(string: "https://db-ip.com")!).font(.caption)
            }
        }
        .padding(12)
    }

    private func placesTable(_ places: [AppTrafficText.MapPlace]) -> some View {
        Card {
            HStack {
                Text("Place").frame(maxWidth: .infinity, alignment: .leading)
                Text("Addresses").frame(width: 70, alignment: .trailing)
                Text("Top app").frame(width: 140, alignment: .leading)
                Text("In").frame(width: 76, alignment: .trailing)
                Text("Out").frame(width: 76, alignment: .trailing)
            }
            .font(.system(size: 10, weight: .semibold)).textCase(.uppercase).foregroundStyle(.secondary)
            .padding(.horizontal, 10).padding(.vertical, 5)
            Divider()
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(places) { p in
                        HStack {
                            Text(p.name.isEmpty ? "Unknown" : p.name).frame(maxWidth: .infinity, alignment: .leading)
                            Text("\(p.addresses)").frame(width: 70, alignment: .trailing)
                            Text(p.topApp).lineLimit(1).frame(width: 140, alignment: .leading)
                            Text(TrafficFormat.bytes(p.bytesIn)).frame(width: 76, alignment: .trailing)
                            Text(TrafficFormat.bytes(p.bytesOut)).foregroundStyle(.secondary).frame(width: 76, alignment: .trailing)
                        }
                        .font(.system(size: 11, design: .monospaced))
                        .padding(.horizontal, 10).padding(.vertical, 3)
                        Divider().opacity(0.4)
                    }
                }
            }
        }
        .frame(height: 170)
    }

    // MARK: Right: summary

    private var summary: some View {
        let apps = (result?.apps ?? []).filter { selected == nil || $0.app == selected }
        let rem = remotes
        let sent = rem.reduce(UInt64(0)) { $0 + $1.bytesOut }
        let received = rem.reduce(UInt64(0)) { $0 + $1.bytesIn }
        var through = ByVia()
        for r in rem {
            switch r.via {
            case .router: through.router.bytesIn += r.bytesIn; through.router.bytesOut += r.bytesOut
            case .proxy: through.proxy.bytesIn += r.bytesIn; through.proxy.bytesOut += r.bytesOut
            case .direct: through.direct.bytesIn += r.bytesIn; through.direct.bytesOut += r.bytesOut
            }
        }
        return ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("Summary").font(.headline)
                HStack(spacing: 16) {
                    stat("Sent", sent)
                    stat("Received", received)
                }
                Text("Totals count a VPN's tunnel again.").font(.caption).foregroundStyle(.secondary)
                group("Through") {
                    ForEach(TrafficVia.allCases, id: \.self) { via in
                        line(via.rawValue, through[via].total)
                    }
                }
                group("Top apps") {
                    ForEach(AppTrafficText.topApps(apps)) { line($0.label, $0.bytes) }
                }
                group("Top owners") {
                    ForEach(AppTrafficText.topOwners(rem)) { line($0.label, $0.bytes) }
                }
                group("Top countries") {
                    ForEach(AppTrafficText.topCountries(rem)) { line($0.label, $0.bytes) }
                }
                if let here = model.traffic.thisMac {
                    Text("This Mac is in \(here.name). Change it in Settings › Traffic.").font(.caption).foregroundStyle(.secondary)
                }
            }
            .padding(14)
        }
    }

    private func stat(_ title: String, _ bytes: UInt64) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            Text(TrafficFormat.bytes(bytes)).font(.system(size: 16, weight: .semibold))
        }
    }

    private func group<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.system(size: 11, weight: .semibold)).textCase(.uppercase).foregroundStyle(.secondary)
            content()
        }
    }

    private func line(_ label: String, _ bytes: UInt64) -> some View {
        HStack {
            Text(label).lineLimit(1).truncationMode(.middle)
            Spacer()
            Text(TrafficFormat.bytes(bytes)).font(.caption.monospaced()).foregroundStyle(.secondary)
        }
        .font(.system(size: 12))
    }
}

/// The dot map: land dots, an arc from this Mac to each place (wider for
/// more bytes in), and the names of the largest places.
private struct MapCanvas: View {
    let dots: [CGPoint]
    let places: [AppTrafficText.MapPlace]
    let home: Coordinate?
    let zoom: CGFloat
    let pan: CGSize

    var body: some View {
        Canvas { ctx, size in
            let scale = min(size.width / MapProjection.width, size.height / MapProjection.height) * zoom
            let origin = CGPoint(x: (size.width - MapProjection.width * scale) / 2 + pan.width,
                                 y: (size.height - MapProjection.height * scale) / 2 + pan.height)
            func at(_ p: CGPoint) -> CGPoint { CGPoint(x: origin.x + p.x * scale, y: origin.y + p.y * scale) }

            let dot = max(1.2, 1.6 * scale)
            var land = Path()
            for p in dots {
                let q = at(p)
                land.addEllipse(in: CGRect(x: q.x - dot / 2, y: q.y - dot / 2, width: dot, height: dot))
            }
            ctx.fill(land, with: .color(.secondary.opacity(0.35)))

            let top = Double(places.map(\.bytesIn).max() ?? 1)
            let homePoint = home.map { at(MapProjection.point(lat: $0.lat, lon: $0.lon)) }
            for place in places {
                let target = Coordinate(lat: place.lat, lon: place.lon)
                let weight = top > 0 ? Double(place.bytesIn) / top : 0
                if let home {
                    let points = MapProjection.arc(from: home, to: target).map(at)
                    var arc = Path()
                    arc.addLines(points)
                    ctx.stroke(arc, with: .color(Theme.accent.opacity(0.35 + 0.5 * weight)), lineWidth: 0.6 + 3 * weight)
                }
                let p = at(MapProjection.point(lat: place.lat, lon: place.lon))
                let r = 2.5 + 4 * weight
                ctx.fill(Path(ellipseIn: CGRect(x: p.x - r, y: p.y - r, width: 2 * r, height: 2 * r)), with: .color(Theme.accent))
            }
            for place in places.prefix(6) where !place.name.isEmpty {
                let p = at(MapProjection.point(lat: place.lat, lon: place.lon))
                ctx.draw(Text(place.name).font(.system(size: 10, weight: .medium)), at: CGPoint(x: p.x + 6, y: p.y - 8), anchor: .leading)
            }
            if let h = homePoint {
                ctx.fill(Path(ellipseIn: CGRect(x: h.x - 5, y: h.y - 5, width: 10, height: 10)), with: .color(Theme.proxyText))
                ctx.draw(Text("This Mac").font(.system(size: 10, weight: .semibold)), at: CGPoint(x: h.x + 7, y: h.y + 9), anchor: .leading)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .accessibilityLabel("Map of the places this Mac sent data to")
    }
}
