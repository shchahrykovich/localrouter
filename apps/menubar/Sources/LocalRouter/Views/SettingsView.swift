import LocalRouterKit
import SwiftUI

/// The Settings window, laid out like the settings of a JetBrains IDE: a
/// tree of pages with a search field on the left, the selected page on the
/// right. The tree and the search are `SettingsTree`.
struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var fallback = true
    @State private var allowLan = false
    @State private var proxyOn = false
    @State private var logMb = 20
    @State private var logRequests = 5000
    @State private var networkName = ""
    @State private var newInspectHost = ""
    @State private var loaded = false
    @State private var confirmUninstall = false
    @State private var openAtLogin = false
    @State private var loginNote: String?
    @State private var confirmReveal: ScriptRuleView?
    @State private var query = ""
    /// Parents the person closed. Every parent is open while a search runs.
    @State private var collapsed: Set<SettingsPage> = []
    static let pageKey = "settingsPage"
    @AppStorage(SettingsView.pageKey) private var page: SettingsPage = .general
    @AppStorage("autoUpdate") private var autoUpdate = true

    var body: some View {
        HStack(spacing: 0) {
            sidebar.frame(width: 220)
            Divider()
            detail.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .task {
            readOpenAtLogin()
            if let c = await model.config {
                fallback = c.fallback
                allowLan = c.allowLan
                proxyOn = c.proxyEnabled ?? false
                logMb = Int(c.proxyLogFileMb ?? 20)
                logRequests = Int(c.proxyLogFileRequests ?? 5000)
            }
            await model.loadLanNetworks()
            loaded = true
        }
        .onChange(of: query) { _, q in
            // Keep the selection inside the tree the search shows.
            let nodes = SettingsTree.filter(q)
            if !SettingsTree.contains(page, in: nodes), let first = SettingsTree.firstPage(in: nodes, query: q) {
                page = first
            }
        }
    }

    // MARK: Tree

    private var sidebar: some View {
        let nodes = SettingsTree.filter(query)
        return VStack(spacing: 0) {
            searchField.padding(8)
            if nodes.isEmpty {
                Text("No settings match").font(.callout).foregroundStyle(.secondary).padding()
                Spacer()
            } else {
                List(selection: Binding(get: { page }, set: { if let p = $0 { page = p } })) {
                    ForEach(nodes) { node in
                        if node.children.isEmpty {
                            row(node.page)
                        } else {
                            DisclosureGroup(isExpanded: expanded(node.page)) {
                                ForEach(node.children) { row($0.page) }
                            } label: {
                                row(node.page)
                            }
                        }
                    }
                }
                .listStyle(.sidebar)
            }
        }
    }

    private var searchField: some View {
        HStack(spacing: 4) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("Search", text: $query).textFieldStyle(.plain)
            if !query.isEmpty {
                Button { query = "" } label: { Image(systemName: "xmark.circle.fill") }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                    .help("Clear the search")
            }
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .background(RoundedRectangle(cornerRadius: 6).fill(.quaternary))
    }

    private func row(_ page: SettingsPage) -> some View {
        Label(page.title, systemImage: page.symbol).tag(page)
    }

    private func expanded(_ page: SettingsPage) -> Binding<Bool> {
        Binding(
            get: { !query.isEmpty || !collapsed.contains(page) },
            set: { open in if open { collapsed.remove(page) } else { collapsed.insert(page) } })
    }

    // MARK: Pages

    private var detail: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                if let parent = SettingsTree.parent(of: page) {
                    Text(parent.title).foregroundStyle(.secondary)
                    Image(systemName: "chevron.right").font(.caption).foregroundStyle(.secondary)
                }
                Text(page.title)
            }
            .font(.title3.weight(.semibold))
            .padding(.horizontal, 20)
            .padding(.top, 14)
            if page == .help {
                HelpView()
            } else {
                Form { content }
                    .formStyle(.grouped)
            }
        }
    }

    @ViewBuilder private var content: some View {
        switch page {
        case .general: generalPage
        case .routing: routingPage
        case .certificates: certificatesPage
        case .proxy: proxyPage
        case .inspection: inspectionPage
        case .scripts: scriptsPage
        case .daemon: daemonPage
        case .updates: updatesPage
        case .help: EmptyView() // HelpView, outside the Form
        }
    }

    @ViewBuilder private var generalPage: some View {
        Section {
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
        Section {
            Button("Uninstall \(Instance.current.appName)…", role: .destructive) { confirmUninstall = true }
                .confirmationDialog("Uninstall \(Instance.current.appName)?", isPresented: $confirmUninstall) {
                    Button("Uninstall", role: .destructive) { Task { await model.uninstall() } }
                } message: {
                    Text("This untrusts the CA, stops the daemon and deletes all routes and settings.")
                }
        }
    }

    @ViewBuilder private var routingPage: some View {
        Section {
            Toggle("Subdomain fallback", isOn: $fallback)
                .onChange(of: fallback) { _, on in if loaded { Task { await model.setFallback(on) } } }
            Text("feat-x.shop.localhost uses the shop route when it has no route of its own.")
                .font(.caption).foregroundStyle(.secondary)
            Toggle("Allow LAN access", isOn: $allowLan)
                .onChange(of: allowLan) { _, on in if loaded { Task { await model.setAllowLan(on) } } }
            Text("On: other machines reach ports \(ports.http) and \(ports.https) only on the networks below. On other networks only this Mac can reach ports \(ports.http) and \(ports.https). TCP routes, the proxy and the proxy log are always this Mac only.")
                .font(.caption).foregroundStyle(.secondary)
        }
        lanNetworksSection
    }

    /// LAN access per network (ADR 08): this network and the allowed list.
    /// Read when the page opens; nothing watches the network.
    @ViewBuilder private var lanNetworksSection: some View {
        Section("Networks") {
            if let here = model.status?.network {
                let known = model.lanNetworks.first { $0.id == here.id }
                let name = (known?.name).flatMap { $0.isEmpty ? nil : $0 }
                Text("This network: \(name.map { "\($0) " } ?? "")(router \(here.router) on \(here.interface)). \(here.lanAllowed ? "Allowed." : known != nil ? "In the list; turn on Allow LAN access." : "Not allowed.")")
                if known != nil {
                    Button("Stop Allowing") { Task { await model.forgetNetwork(here.id) } }
                } else {
                    HStack {
                        TextField("Name", text: $networkName, prompt: Text("Home"))
                        Button("Allow on This Network") {
                            let name = networkName.trimmingCharacters(in: .whitespaces)
                            networkName = ""
                            Task { await model.allowThisNetwork(name: name) }
                        }
                    }
                }
            } else if model.status != nil {
                Text("This network cannot be recognised (no router, or a VPN). LAN access does not work on it.")
                    .foregroundStyle(.secondary)
            }
            ForEach(model.lanNetworks) { network in LanNetworkRow(network: network) }
            if model.lanNetworks.isEmpty {
                Text("No network is allowed yet.").font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Text("A network is known by its router's MAC address.").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("Check Again") { Task { await model.loadLanNetworks() } }
            }
        }
    }

    @ViewBuilder private var certificatesPage: some View {
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
    }

    /// The forward proxy (ADR 06): on and off, its address, Chrome.
    @ViewBuilder private var proxyPage: some View {
        if model.status?.proxy == nil {
            noProxy
        } else {
            Section {
                Toggle("Forward proxy", isOn: $proxyOn)
                    .onChange(of: proxyOn) { _, on in if loaded { Task { await model.setProxyEnabled(on) } } }
                Text("Chrome or a program you start with the proxy settings sends its traffic through LocalRouter, and Logs shows it. Only this Mac can reach the port.")
                    .font(.caption).foregroundStyle(.secondary)
                if let p = model.proxy {
                    LabeledContent("Address", value: p.bound.isEmpty ? (p.enabled ? "not listening" : "off, port \(p.port)") : p.url)
                    ForEach(p.errors, id: \.self) { e in Text(e).foregroundStyle(.red).font(.caption) }
                    if model.chromeInstalled {
                        Button("Open Chrome via Proxy") { Task { await model.openChromeViaProxy() } }
                    }
                }
            }
            proxyLogSection
        }
    }

    /// The proxy log (ADR 08). Hidden when the daemon is older than API 1.5.
    @ViewBuilder private var proxyLogSection: some View {
        if let log = model.proxy?.log {
            Section("Proxy log") {
                Toggle("Write every proxied request to a HAR file", isOn: Binding(
                    get: { log.enabled },
                    set: { on in Task { await model.setProxyLog(on) } }))
                LabeledContent("MB per file") {
                    TextField("MB per file", value: $logMb, format: .number)
                        .labelsHidden().frame(width: 90).multilineTextAlignment(.trailing)
                        .onSubmit { Task { await model.setProxyLogLimits(mb: UInt64(max(logMb, 0)), requests: nil) } }
                }
                LabeledContent("Requests per file") {
                    TextField("Requests per file", value: $logRequests, format: .number)
                        .labelsHidden().frame(width: 90).multilineTextAlignment(.trailing)
                        .onSubmit { Task { await model.setProxyLogLimits(mb: nil, requests: UInt64(max(logRequests, 0))) } }
                }
                Text(ProxyLog.explanation(keepFiles: log.keepFiles) + " A new file starts at whichever limit comes first.")
                    .font(.caption).foregroundStyle(.secondary)
                LabeledContent("Folder", value: log.folder)
                if let line = ProxyLog.stateLine(log) {
                    Text(line).font(.caption).foregroundStyle(.secondary)
                }
                ForEach(ProxyLog.problems(log), id: \.self) { e in Text(e).foregroundStyle(.red).font(.caption) }
                HStack {
                    Button("Open Proxy Log") { Task { await model.openProxyLog() } }
                    Button("Show Folder") { model.showProxyLogFolder() }
                }
            }
        }
    }

    /// The hosts whose HTTPS the proxy reads, and trust for the inspection CA.
    @ViewBuilder private var inspectionPage: some View {
        if let p = model.proxy {
            Section("Inspected hosts") {
                ForEach(p.inspectHosts, id: \.self) { host in
                    HStack {
                        Text(host).font(.callout.monospaced())
                        Spacer()
                        Button("Remove") { Task { await model.stopInspecting(host) } }
                    }
                }
                HStack {
                    TextField("Host", text: $newInspectHost, prompt: Text("api.example.com, *.example.com or *"))
                        .onSubmit(addInspectHost)
                    Button("Inspect", action: addInspectHost).disabled(newInspectHost.isEmpty)
                }
                Text("Other HTTPS hosts pass through unread: Logs shows only their name.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let ca = p.inspectCa {
                Section("Inspection CA") {
                    LabeledContent("CA", value: ca.commonName ?? "?")
                    LabeledContent("Trusted", value: ca.trusted == true ? "yes" : ca.trusted == false ? "no" : "unknown")
                    if let problem = ca.problem { Text(problem).foregroundStyle(.red).font(.caption) }
                    HStack {
                        let actions = TrustActions.for(trusted: ca.trusted)
                        if actions.contains(.trust) {
                            Button("Trust Inspection…") { Task { await model.trustInspectionCA() } }.disabled(model.busy)
                        }
                        if actions.contains(.untrust) {
                            Button("Untrust") { Task { await model.untrustInspectionCA() } }.disabled(model.busy)
                        }
                    }
                    Text("Trust lets \(Instance.current.appName) read HTTPS traffic for the hosts you list.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        } else {
            noProxy
        }
    }

    /// Script rules (ADR 07): what each rule did, its last error, and the
    /// switches. Rules are set with the command line tool or by an agent.
    @ViewBuilder private var scriptsPage: some View {
        if model.proxy == nil {
            noProxy
        } else {
            Section {
                let rules = model.proxy?.scriptRules ?? []
                if rules.isEmpty {
                    Text("No script rules. A Lua script can change or record the traffic of a host: \(Instance.current.cli) rules api")
                        .font(.caption).foregroundStyle(.secondary)
                }
                ForEach(rules) { view in ScriptRuleRow(view: view, confirmReveal: $confirmReveal) }
            }
            .confirmationDialog(
                "Let \(confirmReveal?.id ?? "this rule") see secrets?",
                isPresented: Binding(get: { confirmReveal != nil }, set: { if !$0 { confirmReveal = nil } })
            ) {
                Button("Show API keys and cookies", role: .destructive) {
                    if let view = confirmReveal { Task { await model.revealSecrets(view, true) } }
                    confirmReveal = nil
                }
            } message: {
                Text("The script will see API keys and cookies of \(confirmReveal?.rule.host ?? "its host"), and may write them to its files.")
            }
        }
    }

    /// The daemon has no proxy: it is not running, or it is older than ADR 06.
    private var noProxy: some View {
        Section {
            Text(model.status == nil ? "Start the daemon to use the proxy." : "This daemon has no forward proxy. Restart it after an update.")
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var daemonPage: some View {
        let errors = (model.status?.http.errors ?? []) + (model.status?.https.errors ?? [])
        let daemon = DaemonSection.decide(
            bound: boundPorts,
            file: DaemonSection.filePorts(configURL: configURL, instance: .current),
            hasProblem: model.status == nil || !errors.isEmpty || model.status?.routesFileProblem != nil || model.serviceNote != nil)
        Section {
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
            if let p = model.status?.routesFileProblem { Text(p).foregroundStyle(.orange).font(.caption) }
            ForEach(model.status?.notes ?? [], id: \.self) { note in Text(note).foregroundStyle(.orange).font(.caption) }
            if let note = model.serviceNote {
                Text(note).font(.caption)
                Button("Open Login Items") { model.openLoginItems() }
            }
        }
    }

    @ViewBuilder private var updatesPage: some View {
        Section {
            LabeledContent("This version", value: model.version)
            if let off = Updater.disabledReason(for: .current) {
                Text(off).font(.caption).foregroundStyle(.secondary)
            } else {
                Toggle("Check for updates automatically", isOn: $autoUpdate)
                Button("Check for Updates…") { Task { await model.checkForUpdates(manual: true) } }
            }
        }
    }

    private func addInspectHost() {
        let host = newInspectHost
        newInspectHost = ""
        Task { await model.inspect(host) }
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

/// One script rule in Settings: kind, host and path, counters, the last
/// error in red, an Enabled switch, Remove.
private struct ScriptRuleRow: View {
    @Environment(AppModel.self) private var model
    let view: ScriptRuleView
    @Binding var confirmReveal: ScriptRuleView?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(view.id).font(.callout.monospaced())
                Text(view.kind ?? "not loaded").font(.caption).foregroundStyle(view.kind == nil ? .red : .secondary)
                Spacer()
                Toggle("Enabled", isOn: Binding(
                    get: { view.rule.enabled },
                    set: { on in Task { await model.setRuleEnabled(view, on) } }))
                    .labelsHidden()
                    .help("A disabled rule matches nothing and keeps its counters")
                Button("Remove") { Task { await model.removeRule(view) } }
            }
            Text(view.rule.target + (view.rule.methods.isEmpty ? "" : " " + view.rule.methods.joined(separator: ",")))
                .font(.caption.monospaced())
            Text(counters).font(.caption).foregroundStyle(.secondary)
            if let error = view.lastError {
                Text(error.message).font(.caption).foregroundStyle(.red).lineLimit(3)
            }
            if view.kind != nil {
                if view.rule.revealSecrets {
                    Button("Hide Secrets Again") { Task { await model.revealSecrets(view, false) } }.font(.caption)
                } else {
                    Button("Let It See Secrets…") { confirmReveal = view }.font(.caption)
                }
            }
        }
    }

    private var counters: String {
        var parts = ["matched \(view.matched)", "errors \(view.errors)"]
        if view.answered > 0 { parts.append("answered \(view.answered)") }
        if view.dropped > 0 { parts.append("dropped \(view.dropped)") }
        if view.kind == "log" { parts.append("written \(ByteCountFormatter.string(fromByteCount: Int64(view.bytesWritten), countStyle: .file))") }
        let life = view.rule.ownerPid != nil ? "owned" : view.rule.persistent ? "persistent" : "session"
        return parts.joined(separator: ", ") + " (\(life))"
    }
}

/// One allowed network: its name (edited in place), router and id, Forget.
private struct LanNetworkRow: View {
    @Environment(AppModel.self) private var model
    let network: LanNetwork
    @State private var name = ""

    var body: some View {
        HStack {
            TextField("Name", text: $name, prompt: Text("No name"))
                .frame(maxWidth: 160)
                .onSubmit { Task { await model.renameNetwork(network.id, to: name.trimmingCharacters(in: .whitespaces)) } }
            Text("router \(network.router)").font(.caption).foregroundStyle(.secondary)
            Text(network.id).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
            Spacer()
            Button("Forget") { Task { await model.forgetNetwork(network.id) } }
        }
        .onAppear { name = network.name }
    }
}

