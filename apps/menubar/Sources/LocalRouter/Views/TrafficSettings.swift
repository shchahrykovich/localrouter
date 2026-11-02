import LocalRouterKit
import SwiftUI

/// Settings › Traffic (ADR 11, 5.3, canvas artboard `SettingsTraffic`):
/// monitoring, app checks, Ask AI. The daemon keeps the first group in
/// config.json; the rest are the app's own settings.
struct TrafficSettings: View {
    @Environment(AppModel.self) private var model
    @State private var on = false
    @State private var days: UInt32 = 7
    @State private var places = false
    @State private var loaded = false
    @State private var key = ""
    @State private var keyNote: String?
    @State private var confirmClear = false
    @State private var trustedVersion = 0
    @AppStorage(TrafficModel.Key.checkSignatures) private var checkSignatures = true
    @AppStorage(TrafficModel.Key.lookUpVirusTotal) private var lookUp = false
    @AppStorage(TrafficModel.Key.redDot) private var redDot = true
    @AppStorage(TrafficModel.Key.notify) private var notify = true
    @AppStorage(TrafficModel.Key.warnAdhoc) private var warnAdhoc = false
    @AppStorage(TrafficModel.Key.askService) private var askService = "claude"
    @AppStorage(TrafficModel.Key.askAddresses) private var askAddresses = true
    @AppStorage(TrafficModel.Key.thisMac) private var thisMac = ""

    var body: some View {
        Group {
            monitoring
            appChecks
            askAI
        }
        .task {
            if let c = await model.config {
                on = c.traffic ?? false
                days = c.trafficHistoryDays ?? 7
                places = c.trafficPlaces ?? false
            }
            loaded = true
        }
    }

    @ViewBuilder private var monitoring: some View {
        Section("Monitoring") {
            if let message = model.appsAvailability.message, model.running {
                Text(message).font(.caption).foregroundStyle(Theme.warningText)
            }
            Toggle(isOn: $on) {
                Text("See every app on this Mac")
                Text("Uses nettop, which comes with macOS. Off by default. Nothing leaves this Mac.")
            }
            .onChange(of: on) { _, v in if loaded { Task { await model.setTraffic(v) } } }
            if let e = model.status?.trafficError {
                Text(e).font(.caption).foregroundStyle(Theme.warningText)
            }
            Picker(selection: $days) {
                Text("1 day").tag(UInt32(1))
                Text("7 days").tag(UInt32(7))
                Text("30 days").tag(UInt32(30))
            } label: {
                Text("Keep history")
                Text("\(TrafficFormat.bytes(model.status?.trafficHistoryBytes ?? 0)) on disk, at most 500 MB. Every address every app talked to: it stays on this Mac.")
            }
            .onChange(of: days) { _, v in if loaded { Task { await model.setTrafficHistoryDays(v) } } }
            Button("Clear History…", role: .destructive) { confirmClear = true }
                .confirmationDialog("Delete the traffic history?", isPresented: $confirmClear) {
                    Button("Delete", role: .destructive) { Task { await model.clearTrafficHistory() } }
                } message: {
                    Text("The Apps view and the map start again from zero.")
                }
            Toggle(isOn: $places) {
                Text("Show places on the map")
                Text(placesNote)
            }
            .onChange(of: places) { _, v in if loaded { Task { await model.setTrafficPlaces(v) } } }
            if places {
                HStack {
                    Button("Update") { Task { await model.updateGeoDb() } }.disabled(model.busy)
                    Spacer()
                    Link("IP geolocation by DB-IP", destination: URL(string: "https://db-ip.com")!).font(.caption)
                }
            }
            Picker(selection: $thisMac) {
                Text("Time zone: \(MapProjection.place(forTimeZone: TimeZone.current.identifier)?.name ?? "unknown")").tag("")
                ForEach(MapProjection.knownPlaces.sorted { $0.name < $1.name }, id: \.name) { p in Text(p.name).tag(p.name) }
            } label: {
                Text("This Mac is in")
                Text("The start of the arcs on the map. No network lookup is made for it.")
            }
        }
    }

    private var placesNote: String {
        if let db = model.status?.geoDb {
            return "Database \(db.date), \(TrafficFormat.bytes(db.bytes)), from DB-IP Lite (CC BY 4.0). Lookups stay on this Mac."
        }
        return "Downloads the free DB-IP Lite databases (about 135 MB) once. Lookups stay on this Mac."
    }

    @ViewBuilder private var appChecks: some View {
        Section("App checks") {
            Toggle(isOn: $checkSignatures) {
                Text("Check code signatures")
                Text("Who signed each program and whether Apple notarized it. On this Mac, no network.")
            }
            Toggle(isOn: $lookUp) {
                Text("Look up new programs on VirusTotal")
                Text("Sends only the SHA-256 of a program, never the file. Apple programs are not looked up.")
            }
            HStack {
                SecureField("API key", text: $key, prompt: Text(model.traffic.keySaved ? "Saved in Keychain" : "VirusTotal API key"))
                Button("Save") {
                    keyNote = model.traffic.saveKey(key) ?? (key.isEmpty ? "The key is removed." : "Saved in Keychain.")
                    key = ""
                }
                .disabled(key.isEmpty && !model.traffic.keySaved)
            }
            HStack {
                Text("\(model.traffic.lookupsToday) of \(RateLimiter.perDay) lookups today").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Link("Get a free key", destination: VirusTotal.keyPage).font(.caption)
            }
            if let keyNote { Text(keyNote).font(.caption).foregroundStyle(.secondary) }
            Toggle("Show a red dot on the menu bar icon when a program needs review", isOn: $redDot)
            Toggle("Send a notification when a program needs review", isOn: $notify)
            Toggle(isOn: $warnAdhoc) {
                Text("Also warn about ad-hoc signed programs")
                Text("Off by default: Homebrew, cargo and your own builds are ad-hoc signed.")
            }
            DisclosureGroup("Trusted programs (\(trusted.count))") {
                if trusted.isEmpty {
                    Text("No program is trusted. Trust one in its details window.").font(.caption).foregroundStyle(.secondary)
                }
                ForEach(trusted, id: \.key) { sha, path in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(URL(fileURLWithPath: path).lastPathComponent)
                            Text(sha.prefix(16) + "…").font(.caption.monospaced()).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("Remove") {
                            model.traffic.untrust(sha)
                            trustedVersion += 1
                        }
                    }
                }
            }
        }
    }

    private var trusted: [(key: String, value: String)] {
        _ = trustedVersion
        _ = model.traffic.trustedVersion
        return model.traffic.trusted.all.sorted { $0.value < $1.value }
    }

    @ViewBuilder private var askAI: some View {
        Section("Ask AI") {
            Picker("\"What is this?\" opens", selection: $askService) {
                Text("Claude").tag("claude")
                Text("ChatGPT").tag("chatgpt")
                Text("Both").tag("both")
            }
            Toggle(isOn: $askAddresses) {
                Text("Put network addresses in the question")
                Text("The question never holds your home folder or user name.")
            }
        }
    }
}
