import LocalRouterKit
import SwiftUI

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var fallback = true
    @State private var allowLan = false
    @State private var loaded = false
    @State private var confirmUninstall = false
    @AppStorage("autoUpdate") private var autoUpdate = true

    var body: some View {
        Form {
            Section("Routing") {
                Toggle("Subdomain fallback", isOn: $fallback)
                    .onChange(of: fallback) { _, on in if loaded { Task { await model.setFallback(on) } } }
                Text("feat-x.shop.localhost uses the shop route when it has no route of its own.")
                    .font(.caption).foregroundStyle(.secondary)
                Toggle("Allow LAN access", isOn: $allowLan)
                    .onChange(of: allowLan) { _, on in if loaded { Task { await model.setAllowLan(on) } } }
                Text("Off: only this Mac can reach ports 80 and 443. TCP routes are always this Mac only.")
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
            // Shown only when something is wrong with the daemon.
            let errors = (model.status?.http.errors ?? []) + (model.status?.https.errors ?? [])
            let routesFileProblem = model.status?.routesFileProblem
            if model.status == nil || !errors.isEmpty || routesFileProblem != nil || model.serviceNote != nil {
                Section("Daemon") {
                    if model.status == nil {
                        Text(model.daemonProblem ?? "Not running").foregroundStyle(.secondary)
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
                Toggle("Check for updates automatically", isOn: $autoUpdate)
                Button("Check for Updates…") { Task { await model.checkForUpdates(manual: true) } }
            }
            Section {
                Button("Uninstall LocalRouter…", role: .destructive) { confirmUninstall = true }
                    .confirmationDialog("Uninstall LocalRouter?", isPresented: $confirmUninstall) {
                        Button("Uninstall", role: .destructive) { Task { await model.uninstall() } }
                    } message: {
                        Text("This untrusts the CA, stops the daemon and deletes all routes and settings.")
                    }
            }
        }
        .formStyle(.grouped)
        .task {
            if let c = await model.config {
                fallback = c.fallback
                allowLan = c.allowLan
            }
            loaded = true
        }
    }
}
