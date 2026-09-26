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
    var routes: [RouteView] = []
    var logs: [LogEntry] = []
    var daemonProblem: String?
    var serviceNote: String?
    var update: UpdateState = .idle
    var message: String?
    var busy = false

    @ObservationIgnored private var statusItemMenu: StatusItemMenu?
    private var logSubscription: LogSubscription?
    private var refreshTask: Task<Void, Never>?
    private var updateTask: Task<Void, Never>?

    static let daemonPlist = "\(Updater.daemonLabel).plist"
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
        statusItemMenu = StatusItemMenu(model: self)
        statusItemMenu?.install()
        registerDaemon()
        refreshTask = Task { [weak self] in
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
            serviceNote = "Running outside an app bundle: start localrouterd yourself."
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

    func openLoginItems() {
        SMAppService.openSystemSettingsLoginItems()
    }

    func refresh() async {
        do {
            let s = try await client.status()
            let r = try await client.listRoutes()
            status = s
            routes = r
            daemonProblem = nil
            if logSubscription == nil { startLogs() }
        } catch {
            status = nil
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
            _ = try await client.unregister(host: route.route.host)
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

    func installCLI() {
        do {
            let installer = CLIInstaller()
            switch try installer.install() {
            case let .installed(link, onPath):
                message = onPath
                    ? "Installed \(link.path)."
                    : "Installed \(link.path). Add it to PATH: \(installer.pathHint)"
            }
        } catch {
            message = error.localizedDescription
        }
    }

    var mcpCommand: String {
        let link = CLIInstaller().link
        let installed = (try? FileManager.default.destinationOfSymbolicLink(atPath: link.path)) != nil
        return "claude mcp add localrouter -- \(installed ? link.path : "localrouter") mcp"
    }

    // MARK: Agent instructions

    /// router.localhost, served by the daemon.
    var agentHelpURL: String {
        AgentHelp.url(httpPort: status?.http.port, httpsPort: status?.https.port)
    }

    var agentPrompt: String { AgentHelp.prompt(url: agentHelpURL) }

    // MARK: Updates

    func checkForUpdates(manual: Bool) async {
        if case .downloading = update { return }
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
        if let obstacle = Updater.obstacle(appURL: Bundle.main.bundleURL) {
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
        let fm = FileManager.default
        try? fm.removeItem(at: Paths.dataDir)
        try? fm.removeItem(at: Paths.logsDir)
        let link = CLIInstaller().link
        if let dest = try? fm.destinationOfSymbolicLink(atPath: link.path), dest.hasSuffix("/" + BundleLayout.cli) {
            try? fm.removeItem(at: link)
        }
        busy = false
        NSWorkspace.shared.activateFileViewerSelecting([Bundle.main.bundleURL])
        message = "LocalRouter is uninstalled. Move LocalRouter.app to the Trash."
        try? await Task.sleep(for: .seconds(4))
        NSApp.terminate(nil)
    }
}
