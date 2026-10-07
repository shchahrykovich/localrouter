import LocalRouterKit
import SwiftUI

/// The Routes tab: the routes grouped by project, one card per project.
struct RoutesView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("collapsedProjects") private var collapsed = CollapsedProjects()

    var body: some View {
        if let problem = model.daemonProblem {
            NotRunningView(problem: problem)
        } else if model.routes.isEmpty {
            EmptyRoutesView()
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(RouteGroup.group(model.routes)) { group in
                        let isCollapsed = collapsed.contains(group.project)
                        VStack(alignment: .leading, spacing: 6) {
                            projectHeader(group, collapsed: isCollapsed)
                            if !isCollapsed {
                                Card {
                                    ForEach(Array(group.routes.enumerated()), id: \.element.id) { index, view in
                                        if index > 0 { Divider() }
                                        RouteRow(view: view)
                                    }
                                }
                            }
                        }
                    }
                }
                .padding(.horizontal, 14)
                .padding(.bottom, 12)
            }
        }
    }

    private func projectHeader(_ group: RouteGroup, collapsed isCollapsed: Bool) -> some View {
        Button { collapsed.toggle(group.project) } label: {
            HStack(spacing: 6) {
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .bold))
                    .rotationEffect(.degrees(isCollapsed ? -90 : 0))
                    .frame(width: 10)
                Text(group.project).textCase(.uppercase).kerning(0.3)
                Spacer()
                Text(group.summary).fontWeight(.medium)
            }
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(isCollapsed ? "Show the routes of \(group.project)" : "Hide the routes of \(group.project)")
    }
}

/// One route: the dot, the name, its labels and target; Open, Copy and
/// Remove appear under the pointer.
struct RouteRow: View {
    @Environment(AppModel.self) private var model
    let view: RouteView
    @State private var hovering = false

    private var route: Route { view.route }
    private var isHTTP: Bool { route.protocol == .http }

    var body: some View {
        HStack(spacing: 10) {
            StatusDot(health: view.health)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 6) {
                    name
                    ForEach(route.badges, id: \.self) { badge in
                        Chip(text: badge.rawValue, kind: badge == .tcp ? .accent : .outline).help(help(badge))
                    }
                }
                target
                if !route.note.isEmpty {
                    Text(route.note).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }
            }
            Spacer(minLength: 4)
            HStack(spacing: 2) {
                if isHTTP {
                    IconButton(symbol: "arrow.up.right.square", help: "Open in the browser") { model.open(view.primaryURL) }
                }
                IconButton(symbol: "doc.on.doc", help: "Copy") { model.copy(view.primaryURL) }
                IconButton(symbol: "trash", help: "Remove") { Task { await model.remove(view) } }
            }
            // Hidden, not removed, so the row keeps its height and width.
            .opacity(hovering ? 1 : 0)
            .allowsHitTesting(hovering)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(hovering ? Theme.rowHover : .clear)
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
    }

    @ViewBuilder private var name: some View {
        let label = Text(route.host).fontWeight(.medium).foregroundStyle(view.health == .down ? .secondary : .primary)
            + Text(route.nameSuffix).foregroundStyle(.secondary)
            + Text(route.path ?? "").foregroundStyle(Theme.accent)
        if isHTTP {
            Button { model.open(view.primaryURL) } label: { label }
                .buttonStyle(.plain)
                .pointingHandCursor()
                .help("Open \(view.primaryURL)")
        } else {
            label.textSelection(.enabled)
        }
    }

    private var target: some View {
        let note = view.healthNote.map { Text(" · \($0)").font(.caption) } ?? Text("")
        return (Text("→ \(route.displayTarget())").font(Theme.mono) + note)
            .foregroundStyle(view.health == .listenFailed ? Theme.warningText : .secondary)
            .lineLimit(1)
            .truncationMode(.middle)
    }

    private func help(_ badge: RouteBadge) -> String {
        switch badge {
        case .tcp: "A TCP route: chosen by its listen port"
        case .folder: "Serves a folder, with no dev server"
        case .stripPath: "The path is removed before the request reaches the target"
        case .owned: "Removed when its dev server stops"
        case .session: "Not saved: removed when the daemon stops"
        }
    }
}

/// Green: the target is up. Grey ring: down. Orange: the listen port is taken.
struct StatusDot: View {
    let health: RouteHealth

    var body: some View {
        Group {
            switch health {
            case .online: Circle().fill(Theme.online)
            case .listenFailed: Circle().fill(Theme.warning)
            case .down: Circle().strokeBorder(Color.secondary, lineWidth: 1.5)
            case .unknown: Circle().fill(Color.secondary.opacity(0.35))
            }
        }
        .frame(width: 8, height: 8)
        .help(text)
        .accessibilityLabel(text)
    }

    private var text: String {
        switch health {
        case .online: "Target is up"
        case .down: "Target is down"
        case .listenFailed: "Listen port is taken"
        case .unknown: "Not checked yet"
        }
    }
}

/// No routes: how to add the first one.
struct EmptyRoutesView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("No routes yet").font(.system(size: 17, weight: .semibold))
                    (Text("Give a dev server a name instead of a port: ")
                        + Text("https://shop.localhost\(model.httpsPortPart)").font(.system(size: 12, design: .monospaced))
                        .foregroundStyle(Theme.accentSoftText))
                        .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 4)

                Card(padding: 12) {
                    VStack(alignment: .leading, spacing: 8) {
                        Label("From a terminal", systemImage: "terminal").font(.body.weight(.semibold))
                            .labelStyle(AccentIconLabelStyle())
                        CopyLine(text: "\(Instance.current.cli) add shop 5173")
                    }
                }

                Card(padding: 12) {
                    VStack(alignment: .leading, spacing: 8) {
                        Label("Or let a coding agent do it", systemImage: "sparkles").font(.body.weight(.semibold))
                            .labelStyle(AccentIconLabelStyle())
                        step(1, "Add the MCP server:", model.mcpCommand)
                        step(2, "In your project folder, paste this into the agent:", model.agentPrompt)
                    }
                }

                LinkButton("Open the full guide at \(hostOnly(model.agentHelpURL))") { model.open(model.agentHelpURL) }
                    .padding(.horizontal, 4)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
        }
    }

    private func step(_ n: Int, _ text: String, _ value: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Text("\(n)").font(.system(size: 10, weight: .bold)).foregroundStyle(Theme.accentSoftText)
                .frame(width: 18, height: 18).background(Theme.accentSoft, in: Circle())
            VStack(alignment: .leading, spacing: 4) {
                Text(text).font(.callout).foregroundStyle(.secondary)
                CopyLine(text: value)
            }
        }
    }

    private func hostOnly(_ url: String) -> String {
        url.replacingOccurrences(of: "http://", with: "").replacingOccurrences(of: "https://", with: "")
    }
}

/// The daemon does not answer: why, and the button that starts it.
struct NotRunningView: View {
    @Environment(AppModel.self) private var model
    let problem: String

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "exclamationmark.triangle")
                .font(.system(size: 22))
                .foregroundStyle(Theme.warningText)
                .frame(width: 52, height: 52)
                .background(Theme.warningSoft, in: Circle())
            VStack(spacing: 4) {
                Text("The \(Instance.current.appName) daemon is not running").font(.system(size: 16, weight: .semibold))
                Text("Saved routes come back when it starts.").foregroundStyle(.secondary)
            }
            .multilineTextAlignment(.center)
            Text(problem)
                .font(Theme.mono).foregroundStyle(.secondary).textSelection(.enabled)
                .padding(.horizontal, 10).padding(.vertical, 7)
                .background(Theme.card, in: RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.line, lineWidth: 0.5))
            if let note = model.serviceNote {
                Text(note).font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
                Button("Open Login Items") { model.openLoginItems() }
                    .buttonStyle(.borderedProminent).tint(Theme.accentFill)
            } else {
                Button("Start the daemon") { model.registerDaemon() }
                    .buttonStyle(.borderedProminent).tint(Theme.accentFill).controlSize(.large)
            }
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// A label whose icon is in the accent color.
struct AccentIconLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 8) {
            configuration.icon.foregroundStyle(Theme.accent)
            configuration.title
        }
    }
}
