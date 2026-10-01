import AppKit
import LocalRouterKit
import Observation
import ServiceManagement

/// Everything the menu bar window shows. Route state lives in the daemon;
/// this model only mirrors it (ADR 01, change 5).
@MainActor
@Observable
final class AppModel {
    enum UpdateState: Equatable {
        case idle
        case checking
        case upToDate
        case available(Updater.Release)
        case downloading(Updater.Release)
        case failed(String)
    }

    let client = DaemonClient()
    var status: StatusResult?
    /// The forward proxy (ADR 06); nil while the daemon is older or not running.
    var proxy: GetProxyResult?
    var routes: [RouteView] = []
    var logs: [LogEntry] = []
    var daemonProblem: String?
    var serviceNote: String?
    var update: UpdateState = .idle
    var message: String?
    var busy = false

    private var logSubscription: LogSubscription?
    private var refreshTask: Task<Void, Never>?
    private var updateTask: Task<Void, Never>?

    static let daemonPlist = "\(Instance.current.daemonLabel).plist"
    private let daemonService = SMAppService.agent(plistName: AppModel.daemonPlist)

    var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
    }

    var inBundle: Bool { Bundle.main.bundleURL.pathExtension == "app" }

    var running: Bool { status != nil }

    // MARK: Lifecycle

    private var started = false

    func start() {
        guard !started else { return }
        started = true
        registerDaemon()
        if inBundle, let problem = OpenAtLogin().turnOnAtFirstLaunch() { message = problem }
        refreshTask = Task { [weak self] in
            await self?.registerAgainIfStale()
            while !Task.isCancelled {
                await self?.refresh()
                try? await Task.sleep(for: .seconds(3))
            }
        }
        updateTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(30))
            while !Task.isCancelled {
                if UserDefaults.standard.object(forKey: "autoUpdate") as? Bool ?? true {
                    await self?.checkForUpdates(manual: false)
                }
                try? await Task.sleep(for: .seconds(6 * 3600))
            }
        }
    }

    /// Start the daemon at login, as a LaunchAgent inside the bundle.
    func registerDaemon() {
        guard inBundle else {
            serviceNote = "Running outside an app bundle: start \(Instance.current.daemonProgram) yourself."
            return
        }
        // A daemon with another suffix would use another folder and ports
        // than this app (ADR 04, 01).
        if let problem = Instance.currentProblem ?? Instance.current.bundleProblem(Bundle.main.bundleURL) {
            serviceNote = problem
            return
        }
        switch daemonService.status {
        case .enabled:
            serviceNote = nil
        case .requiresApproval:
            serviceNote = "Allow LocalRouter in System Settings → General → Login Items."
        default:
            do {
                try daemonService.register()
                serviceNote = daemonService.status == .requiresApproval
                    ? "Allow LocalRouter in System Settings → General → Login Items." : nil
            } catch {
                serviceNote = "Could not start the daemon: \(error.localizedDescription)"
            }
        }
    }

    /// An ad-hoc build gets a new signature each time it is built. After
    /// scripts/install.sh replaces the app, launchd keeps the launch
    /// constraint of the old registration and refuses to start the new
    /// daemon ("spawn failed", exit 78, "needs LWCR update"). Registering
    /// again records the new signature. Release builds are signed by team and
    /// are not affected.
    private func registerAgainIfStale() async {
        guard inBundle, BuildKind.current == .dev, daemonService.status == .enabled else { return }
        try? await Task.sleep(for: .seconds(2))
        guard (try? await client.status()) == nil else { return }
        do {
            try await daemonService.unregister()
            try daemonService.register()
        } catch {
            serviceNote = "Could not start the daemon again: \(error.localizedDescription)"
        }
    }

    func openLoginItems() {
        SMAppService.openSystemSettingsLoginItems()
    }

    func refresh() async {
        do {
            let s = try await client.status()
            let r = try await client.listRoutes()
            status = s
            routes = r
            // The trust check behind get_proxy is cached by the daemon for 10 s.
            proxy = s.proxy == nil ? nil : try? await client.proxy()
            daemonProblem = nil
            if logSubscription == nil { startLogs() }
        } catch {
            status = nil
            proxy = nil
            routes = []
            daemonProblem = error.localizedDescription
            stopLogs()
        }
    }

    // MARK: Logs

    private func startLogs() {
        let sub = LogSubscription()
        logSubscription = sub
        Task {
            if let recent = try? await client.logs(limit: 500) { logs = recent.reversed() }
        }
        sub.start(onEntry: { [weak self] entry in
            Task { @MainActor in
                guard let self else { return }
                self.logs.insert(entry, at: 0)
                if self.logs.count > 1000 { self.logs.removeLast(self.logs.count - 1000) }
            }
        }, onEnd: { [weak self] in
            Task { @MainActor in self?.logSubscription = nil }
        })
    }

    private func stopLogs() {
        logSubscription?.stop()
        logSubscription = nil
    }

    // MARK: Route actions

    func remove(_ route: RouteView) async {
        do {
            _ = try await client.unregister(route.route)
            await refresh()
        } catch {
            message = error.localizedDescription
        }
    }

    func open(_ url: String) {
        if let u = URL(string: url) { NSWorkspace.shared.open(u) }
    }

    func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        message = "Copied \(text)"
    }

    // MARK: Settings

    func setFallback(_ on: Bool) async { await setConfig(SetConfigParams(fallback: on)) }
    func setAllowLan(_ on: Bool) async { await setConfig(SetConfigParams(allowLan: on)) }

    // MARK: Forward proxy (ADR 06)

    func setProxyEnabled(_ on: Bool) async { await setConfig(SetConfigParams(proxyEnabled: on)) }

    /// Adds a host to the inspect list. The first one makes the inspection CA.
    func inspect(_ host: String) async {
        let host = host.trimmingCharacters(in: .whitespaces)
        guard !host.isEmpty else { return }
        await setConfig(SetConfigParams(inspectHosts: (proxy?.inspectHosts ?? []) + [host]))
    }

    func stopInspecting(_ host: String) async {
        await setConfig(SetConfigParams(inspectHosts: (proxy?.inspectHosts ?? []).filter { $0 != host }))
    }

    func trustInspectionCA() async {
        busy = true
        let (ok, out) = await runTool(["proxy", "trust"])
        busy = false
        message = ok ? "The inspection CA is trusted. Restart programs that use the proxy." : out
        try? await Task.sleep(for: .seconds(1))
        await refresh()
    }

    func untrustInspectionCA() async {
        busy = true
        let (ok, out) = await runTool(["proxy", "untrust"])
        busy = false
        message = ok ? "The inspection CA is no longer trusted." : out
        await refresh()
    }

    /// Asked each time the right-click menu opens (I17).
    var chromeInstalled: Bool { ChromeLauncher(daemon: client).chromeInstalled }

    /// Starts a separate Chrome that uses the proxy, turning the proxy on
    /// first if needed. The result goes to the window footer.
    func openChromeViaProxy() async {
        let feedback = await ChromeLauncher(daemon: client).launch()
        message = feedback.summary
        await refresh()
    }

    private func setConfig(_ p: SetConfigParams) async {
        do {
            _ = try await client.setConfig(p)
            await refresh()
        } catch {
            message = error.localizedDescription
        }
    }

    var config: Config? {
        get async { try? await client.config() }
    }

    /// Runs the bundled CLI (trust, untrust). macOS shows its own password dialog.
    func runTool(_ args: [String]) async -> (Bool, String) {
        let tool = Bundle.main.bundleURL.appendingPathComponent(BundleLayout.cli)
        return await Task.detached {
            let p = Process()
            p.executableURL = tool
            p.arguments = args
            let pipe = Pipe()
            p.standardOutput = pipe
            p.standardError = pipe
            do { try p.run() } catch { return (false, error.localizedDescription) }
            p.waitUntilExit()
            let out = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
            return (p.terminationStatus == 0, out.trimmingCharacters(in: .whitespacesAndNewlines))
        }.value
    }

    func trustCA() async {
        busy = true
        let (ok, out) = await runTool(["trust"])
        busy = false
        message = ok ? "The local CA is trusted. Reload open pages." : out
        try? await Task.sleep(for: .seconds(1))
        await refresh()
    }

    func untrustCA() async {
        busy = true
        let (ok, out) = await runTool(["untrust"])
        busy = false
        message = ok ? "The local CA is no longer trusted." : out
        await refresh()
    }

    func revealCA() {
        NSWorkspace.shared.activateFileViewerSelecting([Paths.caPem])
    }

    // MARK: Command line tool

    /// The result also goes to the window footer; the right-click menu shows
    /// it in an alert.
    @discardableResult
    func installCLI() -> Feedback {
        let feedback: Feedback
        do {
            let installer = CLIInstaller()
            switch try installer.install() {
            case let .installed(link, onPath):
                feedback = .cliInstalled(link: link, onPath: onPath, pathHint: installer.pathHint)
            }
        } catch {
            feedback = .failed("Could not install the command line tool", error.localizedDescription)
        }
        message = feedback.summary
        return feedback
    }

    /// Link the note into ~/.claude and import it at the top of CLAUDE.md.
    @discardableResult
    func installClaude() -> Feedback {
        let feedback: Feedback
        do {
            let installer = ClaudeInstaller()
            switch try installer.install() {
            case let .installed(link, importAdded):
                feedback = .claudeInstalled(link: link, claudeMD: installer.claudeMD, importAdded: importAdded)
            case let .noClaude(dir):
                feedback = .noClaude(dir)
            }
        } catch {
            feedback = .failed("Could not install the Claude Code instructions", error.localizedDescription)
        }
        message = feedback.summary
        return feedback
    }

    @discardableResult
    func installCodex() -> Feedback {
        let feedback: Feedback
        do {
            switch try CodexInstaller().install() {
            case let .installed(link, agentsMD, instructionAdded):
                feedback = .codexInstalled(link: link, agentsMD: agentsMD, instructionAdded: instructionAdded)
            case let .noCodex(dir):
                feedback = .noCodex(dir)
            }
        } catch {
            feedback = .failed("Could not install the Codex instructions", error.localizedDescription)
        }
        message = feedback.summary
        return feedback
    }

    var mcpCommand: String {
        let link = CLIInstaller().link
        let installed = (try? FileManager.default.destinationOfSymbolicLink(atPath: link.path)) != nil
        let cli = Instance.current.cli
        return "claude mcp add \(cli) -- \(installed ? link.path : cli) mcp"
    }

    // MARK: Agent instructions

    /// router.localhost, served by the daemon, on its bound ports; the
    /// instance's default ports while the daemon is not running.
    var agentHelpURL: String {
        guard let status else { return Instance.current.defaultHelpURL }
        return AgentHelp.url(httpPort: status.http.port, httpsPort: status.https.port)
    }

    /// `:7443` after a host in an https URL, or nothing on port 443.
    var httpsPortPart: String {
        let port = status?.https.port ?? Instance.current.defaultPorts.https
        return port == 443 ? "" : ":\(port)"
    }

    var agentPrompt: String { AgentHelp.prompt(url: agentHelpURL) }

    // MARK: Updates

    func checkForUpdates(manual: Bool) async {
        if case .downloading = update { return }
        if let reason = Updater.disabledReason(for: .current) {
            update = manual ? .failed(reason) : .idle
            return
        }
        update = .checking
        do {
            let release = try await Updater.latest(currentVersion: version)
            if Updater.isNewer(release.version, than: version) {
                update = .available(release)
            } else {
                update = manual ? .upToDate : .idle
            }
        } catch {
            update = manual ? .failed(error.localizedDescription) : .idle
        }
    }

    func installUpdate(_ release: Updater.Release) async {
        if let obstacle = Updater.disabledReason(for: .current) ?? Updater.obstacle(appURL: Bundle.main.bundleURL) {
            update = .failed(obstacle)
            return
        }
        update = .downloading(release)
        do {
            let dmg = try await Updater.download(release)
            try await Task.detached { try Updater.checkImage(dmg) }.value
            try Updater.startInstall(dmg: dmg, app: Bundle.main.bundleURL)
            try? await Task.sleep(for: .milliseconds(600))
            NSApp.terminate(nil)
        } catch {
            update = .failed(error.localizedDescription)
        }
    }

    // MARK: Uninstall

    /// Untrust the CA, stop the daemon, delete data and the CLI link. The user
    /// then moves the app to the Trash (manifest, Rollback).
    func uninstall() async {
        busy = true
        _ = await runTool(["untrust"])
        try? await daemonService.unregister()
        Uninstaller().removeFiles()
        busy = false
        NSWorkspace.shared.activateFileViewerSelecting([Bundle.main.bundleURL])
        message = "\(Instance.current.appName) is uninstalled. Move \(Instance.current.appName).app to the Trash."
        try? await Task.sleep(for: .seconds(4))
        NSApp.terminate(nil)
    }
}
