import LocalRouterKit
import SwiftUI

/// An AppKit entry point, not a SwiftUI App: the app has no windows of its
/// own, only the menu bar icon and its popover (StatusItemController), and a
/// SwiftUI App with no scene to show opens an empty Settings window.
@main
enum LocalRouterApp {
    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        // No Dock icon, as LSUIElement does for the bundle; also for `swift run`.
        app.setActivationPolicy(.accessory)
        withExtendedLifetime(delegate) { app.run() }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let model = AppModel()
    private var statusItem: StatusItemController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        statusItem = StatusItemController(model: model)
        model.start()
    }
}

enum Tab: String, CaseIterable, Identifiable {
    case domains = "Domains"
    case logs = "Logs"
    case settings = "Settings"
    case help = "Help"
    var id: String { rawValue }
}

struct MainView: View {
    @Environment(AppModel.self) private var model
    @State private var tab: Tab = .domains

    var body: some View {
        VStack(spacing: 0) {
            header
            Picker("", selection: $tab) {
                ForEach(Tab.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(.horizontal, 12)
            .padding(.bottom, 8)
            Divider()
            Group {
                switch tab {
                case .domains: DomainsView()
                case .logs: LogsView()
                case .settings: SettingsView()
                case .help: HelpView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            Divider()
            footer
        }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(Instance.current.appName).font(.headline)
            Spacer()
            if let s = model.status {
                Text("\(s.routes) routes").foregroundStyle(.secondary).font(.caption)
            } else {
                Label("not running", systemImage: "exclamationmark.circle").foregroundStyle(.orange).font(.caption)
            }
        }
        .padding(12)
    }

    private var footer: some View {
        HStack(spacing: 8) {
            UpdateBadge()
            if let m = model.message {
                Text(m).font(.caption).foregroundStyle(.secondary).lineLimit(2).truncationMode(.middle)
            }
            Spacer()
            Menu {
                Button("Install Command Line Tool…") { model.installCLI() }
                Button("Install Claude Code Instructions…") { model.installClaude() }
                Button("Check for Updates…") { Task { await model.checkForUpdates(manual: true) } }
                Divider()
                Button("Quit LocalRouter") { NSApp.terminate(nil) }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }
}

struct UpdateBadge: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        switch model.update {
        case let .available(release):
            Button("Update to \(release.version)") { Task { await model.installUpdate(release) } }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
        case let .downloading(release):
            ProgressView().controlSize(.small)
            Text("Installing \(release.version)…").font(.caption)
        case .checking:
            ProgressView().controlSize(.small)
        case .upToDate:
            Text("Up to date (\(model.version))").font(.caption).foregroundStyle(.secondary)
        case let .failed(why):
            Text(why).font(.caption).foregroundStyle(.red).lineLimit(2)
        case .idle:
            EmptyView()
        }
    }
}
