import LocalRouterKit
import SwiftUI

struct DomainsView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let problem = model.daemonProblem {
            NotRunningView(problem: problem)
        } else if model.routes.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                Text("No routes yet.").font(.headline)
                Text("Add one from a terminal:")
                CopyLine(text: "localrouter add shop 5173")
                Text("Or let a coding agent do it:")
                CopyLine(text: model.mcpCommand)
                Text("Then paste this into the agent in your project folder:")
                CopyLine(text: model.agentPrompt)
                Spacer()
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            List(model.routes) { view in
                RouteRow(view: view)
            }
            .listStyle(.inset)
        }
    }
}

struct RouteRow: View {
    @Environment(AppModel.self) private var model
    let view: RouteView

    private var dotColor: Color {
        if view.listenFailed { return .orange }
        switch view.upstreamUp {
        case true: return .green
        case false: return .gray
        default: return .gray.opacity(0.4)
        }
    }

    private var primaryURL: String { view.urls.first ?? view.route.fullName }

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Circle().fill(dotColor).frame(width: 8, height: 8).padding(.top, 5)
                .help(view.listenFailed ? "Listen port is taken" : (view.upstreamUp == true ? "Target is up" : "Target is down"))
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    if view.route.protocol == .http {
                        Button(primaryURL) { model.open(primaryURL) }
                            .buttonStyle(.link)
                            .pointingHandCursor()
                    } else {
                        Text(primaryURL).font(.body.monospaced())
                    }
                    if view.route.protocol == .tcp { Badge(text: "tcp") }
                    if view.route.ownerPid != nil { Badge(text: "owned") } else if !view.route.persistent { Badge(text: "session") }
                }
                Text("→ \(view.route.target)").font(.caption.monospaced()).foregroundStyle(.secondary)
                if !view.route.note.isEmpty {
                    Text(view.route.note).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }
            }
            Spacer()
            Button { model.copy(primaryURL) } label: { Image(systemName: "doc.on.doc") }
                .buttonStyle(.borderless).help("Copy")
            Button { Task { await model.remove(view) } } label: { Image(systemName: "trash") }
                .buttonStyle(.borderless).help("Remove")
        }
        .padding(.vertical, 2)
    }
}

struct Badge: View {
    let text: String
    var body: some View {
        Text(text).font(.caption2).padding(.horizontal, 5).padding(.vertical, 1)
            .background(.quaternary, in: Capsule())
    }
}

struct CopyLine: View {
    @Environment(AppModel.self) private var model
    let text: String
    var body: some View {
        HStack {
            Text(text).font(.caption.monospaced()).textSelection(.enabled).lineLimit(2)
            Spacer()
            Button { model.copy(text) } label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless)
        }
        .padding(8)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 6))
    }
}

struct NotRunningView: View {
    @Environment(AppModel.self) private var model
    let problem: String

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("The LocalRouter daemon is not running", systemImage: "exclamationmark.triangle")
                .font(.headline)
            Text(problem).font(.caption).foregroundStyle(.secondary)
            if let note = model.serviceNote {
                Text(note)
                Button("Open Login Items") { model.openLoginItems() }
            } else {
                Button("Start the daemon") { model.registerDaemon() }
            }
            Spacer()
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

extension View {
    /// The hand cursor over a link, as in a browser. `.pointerStyle(.link)`
    /// needs macOS 15; the app supports 14.
    func pointingHandCursor() -> some View {
        onHover { inside in
            if inside { NSCursor.pointingHand.push() } else { NSCursor.pop() }
        }
    }
}
