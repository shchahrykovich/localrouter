import LocalRouterKit
import SwiftUI

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var fallback = true
    @State private var allowLan = false
    @State private var loaded = false
    @State private var confirmUninstall = false
    @State private var openAtLogin = false
    @State private var loginNote: String?
    @AppStorage("autoUpdate") private var autoUpdate = true

    var body: some View {
        Form {
            Section("General") {
                Toggle("Open at login", isOn: $openAtLogin)
                    .disabled(!model.inBundle)
                    .onChange(of: openAtLogin) { _, on in setOpenAtLogin(on) }
                Text(model.inBundle
                    ? "Starts LocalRouter when you log in, so it works after a restart. The daemon starts at login either way."
                    : "Only an app bundle can open at login.")
                    .font(.caption).foregroundStyle(.secondary)
                if let note = loginNote {
                    Text(note).font(.caption)
                    Button("Open Login Items") { model.openLoginItems() }
                }
            }
            Section("Routing") {
                Toggle("Subdomain fallback", isOn: $fallback)
                    .onChange(of: fallback) { _, on in if loaded { Task { await model.setFallback(on) } } }
                Text("feat-x.shop.localhost uses the shop route when it has no route of its own.")
                    .font(.caption).foregroundStyle(.secondary)
                Toggle("Allow LAN access", isOn: $allowLan)
                    .onChange(of: allowLan) { _, on in if loaded { Task { await model.setAllowLan(on) } } }
                Text("Off: only this Mac can reach ports \(ports.http) and \(ports.https). TCP routes are always this Mac only.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("HTTPS certificate authority") {
                if let ca = model.status?.ca {
                    LabeledContent("CA", value: ca.commonName ?? "?")
                    LabeledContent("Trusted", value: ca.trusted == true ? "yes" : ca.trusted == false ? "no" : "unknown")
                    if let problem = ca.problem { Text(problem).foregroundStyle(.red).font(.caption) }
                    HStack {
                        let actions = TrustActions.for(trusted: ca.trusted)
                        if actions.contains(.trust) {
                            Button("Trust…") { Task { await model.trustCA() } }.disabled(model.busy)
                        }
                        if actions.contains(.untrust) {
                            Button("Untrust") { Task { await model.untrustCA() } }.disabled(model.busy)
                        }
                        Button("Show ca.pem") { model.revealCA() }
                    }
                } else {
                    Text("Start the daemon to create the CA.").foregroundStyle(.secondary)
                }
            }
            // Shown when the ports are not 80 and 443, or something is wrong (ADR 04, I12).
            let errors = (model.status?.http.errors ?? []) + (model.status?.https.errors ?? [])
            let routesFileProblem = model.status?.routesFileProblem
            let daemon = DaemonSection.decide(
                bound: boundPorts,
                file: DaemonSection.filePorts(configURL: configURL, instance: .current),
                hasProblem: model.status == nil || !errors.isEmpty || routesFileProblem != nil || model.serviceNote != nil)
            if daemon.visible {
                Section("Daemon") {
                    if let s = model.status {
                        LabeledContent("Version", value: s.daemonVersion)
                        LabeledContent("HTTP", value: s.http.port.map(String.init) ?? "not listening")
                        LabeledContent("HTTPS", value: s.https.port.map(String.init) ?? "not listening")
                    } else {
                        Text(model.daemonProblem ?? "Not running").foregroundStyle(.secondary)
                    }
                    if let pending = daemon.pendingPorts {
                        Text("Restart the daemon to use ports \(pending.http) and \(pending.https):").font(.caption)
                        CopyLine(text: DaemonSection.restartCommand(for: .current))
                    }
                    HStack {
                        Text("Ports are set in config.json.").font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button("Show config.json") { NSWorkspace.shared.activateFileViewerSelecting([configURL]) }
                    }
                    ForEach(errors, id: \.self) { e in
                        Text(e).foregroundStyle(.red).font(.caption)
                    }
                    if let p = routesFileProblem { Text(p).foregroundStyle(.orange).font(.caption) }
                    if let note = model.serviceNote {
                        Text(note).font(.caption)
                        Button("Open Login Items") { model.openLoginItems() }
                    }
                }
            }
            Section("Updates") {
                LabeledContent("This version", value: model.version)
                if let off = Updater.disabledReason(for: .current) {
                    Text(off).font(.caption).foregroundStyle(.secondary)
                } else {
                    Toggle("Check for updates automatically", isOn: $autoUpdate)
                    Button("Check for Updates…") { Task { await model.checkForUpdates(manual: true) } }
                }
            }
            Section {
                Button("Uninstall \(Instance.current.appName)…", role: .destructive) { confirmUninstall = true }
                    .confirmationDialog("Uninstall \(Instance.current.appName)?", isPresented: $confirmUninstall) {
                        Button("Uninstall", role: .destructive) { Task { await model.uninstall() } }
                    } message: {
                        Text("This untrusts the CA, stops the daemon and deletes all routes and settings.")
                    }
            }
        }
        .formStyle(.grouped)
        .task {
            readOpenAtLogin()
            if let c = await model.config {
                fallback = c.fallback
                allowLan = c.allowLan
            }
            loaded = true
        }
    }

    private var configURL: URL { Paths.dataDir.appendingPathComponent("config.json") }

    private var boundPorts: DaemonSection.Ports? {
        guard let http = model.status?.http.port, let https = model.status?.https.port else { return nil }
        return DaemonSection.Ports(http: http, https: https)
    }

    /// The ports the LAN switch is about: bound, or the instance's defaults.
    private var ports: DaemonSection.Ports {
        boundPorts ?? DaemonSection.Ports(http: Instance.current.defaultPorts.http, https: Instance.current.defaultPorts.https)
    }

    private func readOpenAtLogin() {
        guard model.inBundle else { return }
        let login = OpenAtLogin()
        openAtLogin = login.isOn
        loginNote = login.note
    }

    /// Acts only when the switch differs from what macOS has, so reading the
    /// state back does not start a second change.
    private func setOpenAtLogin(_ on: Bool) {
        guard model.inBundle, on != OpenAtLogin().isOn else { return }
        if let problem = OpenAtLogin().choose(on) { model.message = problem }
        // Show what macOS did, which is not always what was asked.
        readOpenAtLogin()
    }
}
