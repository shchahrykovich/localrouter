import LocalRouterKit
import SwiftUI

enum Tab: String, CaseIterable, Identifiable {
    case routes = "Routes"
    case traffic = "Traffic"
    case proxy = "Proxy"
    var id: String { rawValue }
}

/// The window of the menu bar icon: the header, the tabs, the footer.
struct MainView: View {
    @Environment(AppModel.self) private var model
    @State private var tab: Tab = .routes
    @State private var trafficScope: TrafficScope = .all
    /// Opens the Settings window (SettingsWindowController), at a page or
    /// at the last one shown.
    let openSettings: (SettingsPage?) -> Void
    /// Runs a command of the "…" menu (StatusItemController).
    let perform: (AppMenu.Action) -> Void

    var body: some View {
        VStack(spacing: 0) {
            header
            Segments(items: Tab.allCases, selection: $tab) { tab, _ in tabLabel(tab) }
                .padding(.horizontal, 14)
                .padding(.bottom, 12)
            Group {
                switch tab {
                case .routes: RoutesView()
                case .traffic: TrafficView(scope: $trafficScope)
                case .proxy: ProxyView(openSettings: openSettings) {
                        trafficScope = .proxy
                        tab = .traffic
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            Divider()
            footer
        }
        .background(Theme.ground)
        .tint(Theme.accentFill)
    }

    private var header: some View {
        HStack(spacing: 10) {
            AppMark(active: model.running)
            VStack(alignment: .leading, spacing: 1) {
                Text(Instance.current.appName).font(.system(size: 13, weight: .semibold))
                HStack(spacing: 5) {
                    Circle().fill(model.running ? Theme.online : Theme.warning).frame(width: 6, height: 6)
                    Text(StatusLine.text(running: model.running, online: model.routes.filter(\.online).count, total: model.routes.count))
                        .foregroundStyle(model.running ? .secondary : Theme.warningText)
                }
                .font(.system(size: 11))
            }
            Spacer()
            IconButton(symbol: "slider.horizontal.3", help: "Settings", size: 28) { openSettings(nil) }
        }
        .padding(.horizontal, 14)
        .padding(.top, 12)
        .padding(.bottom, 10)
    }

    @ViewBuilder private func tabLabel(_ tab: Tab) -> some View {
        HStack(spacing: 6) {
            Text(tab.rawValue)
            switch tab {
            case .routes where !model.routes.isEmpty:
                Text("\(model.routes.count)").font(.system(size: 11, weight: .medium)).foregroundStyle(.secondary)
            case .proxy where model.proxy?.enabled == true:
                Circle().fill(Theme.online).frame(width: 6, height: 6).accessibilityLabel("on")
            default:
                EmptyView()
            }
        }
    }

    private var footer: some View {
        HStack(spacing: 8) {
            UpdateBadge()
            if let m = model.message {
                Text(m).font(.caption).foregroundStyle(.secondary).lineLimit(2).truncationMode(.middle)
            }
            Spacer()
            Text("v\(model.version)").font(.caption).foregroundStyle(.secondary)
            Menu {
                AppMenuItems(entries: AppMenu.entries(model.menuContext, window: false), perform: perform)
            } label: {
                Image(systemName: "ellipsis")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("More commands")
        }
        .padding(.leading, 14)
        .padding(.trailing, 10)
        .frame(height: 38)
    }
}

/// AppMenu entries as a SwiftUI menu: a section per group between separators.
struct AppMenuItems: View {
    let entries: [AppMenu.Entry]
    let perform: (AppMenu.Action) -> Void

    /// The entries split at the separators; a group's first header is its title.
    private var groups: [(title: String?, entries: [AppMenu.Entry])] {
        var out: [(title: String?, entries: [AppMenu.Entry])] = [(nil, [])]
        for entry in entries {
            switch entry {
            case .separator: out.append((nil, []))
            case let .header(title) where out[out.count - 1].entries.isEmpty: out[out.count - 1].title = title
            default: out[out.count - 1].entries.append(entry)
            }
        }
        return out.filter { !$0.entries.isEmpty }
    }

    var body: some View {
        ForEach(Array(groups.enumerated()), id: \.offset) { _, group in
            if let title = group.title {
                Section(title) { rows(group.entries) }
            } else {
                Section { rows(group.entries) }
            }
        }
    }

    private func rows(_ entries: [AppMenu.Entry]) -> some View {
        ForEach(Array(entries.enumerated()), id: \.offset) { _, entry in
            switch entry {
            case let .item(item):
                let button = Button { perform(item.action) } label: {
                    if item.checked {
                        Label(item.title, systemImage: "checkmark")
                    } else if let symbol = item.symbol {
                        Label(item.title, systemImage: symbol)
                    } else {
                        Text(item.title)
                    }
                }
                if let key = item.shortcut?.first {
                    button.keyboardShortcut(KeyEquivalent(key))
                } else {
                    button
                }
            case let .submenu(title, symbol, children):
                Menu {
                    AppMenuItems(entries: children, perform: perform)
                } label: {
                    Label(title, systemImage: symbol)
                }
            case .header, .separator:
                EmptyView()
            }
        }
    }
}

struct UpdateBadge: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        switch model.update {
        case let .available(release):
            Button("Update to \(release.version)") { Task { await model.installUpdate(release) } }
                .buttonStyle(.borderedProminent)
                .tint(Theme.accentFill)
                .controlSize(.small)
        case let .downloading(release):
            ProgressView().controlSize(.small)
            Text("Installing \(release.version)…").font(.caption)
        case .checking:
            ProgressView().controlSize(.small)
        case .upToDate:
            Label("Up to date", systemImage: "checkmark").font(.caption).foregroundStyle(.secondary)
        case let .failed(why):
            Text(why).font(.caption).foregroundStyle(Theme.dangerText).lineLimit(2)
        case .idle:
            EmptyView()
        }
    }
}
