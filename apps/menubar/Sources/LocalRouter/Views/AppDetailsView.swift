import AppKit
import LocalRouterKit
import SwiftUI

/// One app: who signed it, what VirusTotal knows about its hash, a question
/// for a chat, and where it sent data (ADR 11, canvas artboard `AppInfo`).
struct AppDetailsView: View {
    @Environment(AppModel.self) private var model
    let appKey: String
    @State private var selectedPath: String?
    @State private var note: String?
    @State private var confirmUpload = false
    @State private var confirmQuit = false
    @State private var uploading = false

    private var traffic: TrafficModel { model.traffic }

    private var app: AppTraffic? { traffic.hour?.apps.first { $0.app == appKey } }

    /// The executable shown: the one picked, else the app's main one.
    private func path(_ app: AppTraffic) -> String? {
        if let selectedPath, app.paths.contains(selectedPath) { return selectedPath }
        let main = app.paths.first { $0.hasPrefix(app.app + "/Contents/MacOS/") }
        return main ?? app.paths.first
    }

    var body: some View {
        Group {
            if let app {
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        header(app)
                        if let path = path(app) {
                            if let check = traffic.checks[path] {
                                verdictLine(check)
                                signatureCard(check)
                                virusTotalCard(check)
                                askCard(app, check: check)
                            } else {
                                Card(padding: 12) { Text("Checking the code signature…").foregroundStyle(.secondary) }
                            }
                        }
                        networkCard(app)
                        if let path = path(app), let check = traffic.checks[path] {
                            actions(app, path: path, check: check)
                        }
                        if let note { Text(note).font(.caption).foregroundStyle(.secondary) }
                    }
                    .padding(16)
                }
            } else {
                VStack(spacing: 8) {
                    Text("This app sent nothing in the last hour, or traffic is off.").foregroundStyle(.secondary)
                    Text(appKey).font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(Theme.ground)
        .tint(Theme.accentFill)
    }

    // MARK: Parts

    private func header(_ app: AppTraffic) -> some View {
        HStack(alignment: .top, spacing: 12) {
            AppIconView(app: app, size: 44)
            VStack(alignment: .leading, spacing: 3) {
                Text(AppTrafficText.name(app)).font(.title2.weight(.semibold))
                Text(app.app.hasPrefix("/") ? app.app : app.name).font(.caption.monospaced()).foregroundStyle(.secondary)
                    .textSelection(.enabled).lineLimit(2).truncationMode(.middle)
                Text(app.pids.isEmpty ? "Not running now" : "pid \(app.pids.map(String.init).joined(separator: ", "))")
                    .font(.caption).foregroundStyle(.secondary)
                if app.paths.count > 1 {
                    Picker("Program", selection: Binding(get: { path(app) ?? "" }, set: { selectedPath = $0 })) {
                        ForEach(app.paths, id: \.self) { p in
                            Text(URL(fileURLWithPath: p).lastPathComponent).tag(p)
                        }
                    }
                    .pickerStyle(.menu)
                    .frame(maxWidth: 320)
                }
            }
            Spacer()
        }
    }

    @ViewBuilder private func verdictLine(_ check: SignatureCheck) -> some View {
        let verdict = traffic.verdict(check)
        HStack(spacing: 8) {
            ShieldView(verdict: verdict, size: 16)
            Text(verdictText(verdict, check: check)).font(.system(size: 13, weight: .medium))
                .foregroundStyle(verdict == .needsReview ? Theme.dangerText : .primary)
        }
    }

    private func verdictText(_ v: AppVerdict, check: SignatureCheck) -> String {
        switch v {
        case .apple: "Part of macOS, signed by Apple."
        case .ok: "Signed by \(check.signer ?? "a known developer"). Nothing wrong found."
        case .caution: "One VirusTotal engine flags this file. One engine alone is often a false positive."
        case .trusted: "You marked this exact file as trusted."
        case .needsReview: "Needs review: \(reviewReason(check))."
        }
    }

    private func reviewReason(_ check: SignatureCheck) -> String {
        if check.status == .unsigned { return "it has no code signature" }
        if check.status == .broken { return "its signature does not match the file" }
        if check.status == .adhoc { return "it is only ad-hoc signed" }
        if let vt = traffic.report(for: check.sha256)?.0 { return "\(vt.malicious) of \(vt.engines) VirusTotal engines flag it" }
        return "see below"
    }

    private func signatureCard(_ check: SignatureCheck) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            SectionLabel("Code signature")
            Card(padding: 12) {
                VStack(alignment: .leading, spacing: 5) {
                    LabeledContent("Status", value: check.status.label)
                    if let signer = check.signer { LabeledContent("Signer", value: signer) }
                    if let team = check.teamID { LabeledContent("Team ID", value: team) }
                    if let notarized = check.notarized { LabeledContent("Notarized", value: notarized ? "yes" : "no") }
                    if let failure = check.failure { Text(failure).font(.caption).foregroundStyle(Theme.dangerText) }
                    LabeledContent("SHA-256") {
                        Text(check.sha256).font(.caption.monospaced()).textSelection(.enabled).lineLimit(1).truncationMode(.middle)
                    }
                    Text("Checked on the file on disk: a program that was updated while it runs can differ.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                .font(.system(size: 12))
            }
        }
    }

    @ViewBuilder private func virusTotalCard(_ check: SignatureCheck) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            SectionLabel(text: "VirusTotal") {
                if check.status != .apple && traffic.keySaved {
                    LinkButton(traffic.isLookingUp(check.sha256) ? "Looking up…" : "Check again") {
                        Task { await traffic.checkAgain(check.path) }
                    }
                }
            }
            Card(padding: 12) {
                VStack(alignment: .leading, spacing: 6) {
                    if check.status == .apple {
                        Text("Apple programs are not looked up.").foregroundStyle(.secondary)
                    } else if !traffic.keySaved {
                        Text("Save your free VirusTotal API key in Settings › Traffic to look this file up by its hash.")
                            .foregroundStyle(.secondary)
                    } else if let (vt, at) = traffic.report(for: check.sha256) {
                        if vt.state == .unknown {
                            Text("VirusTotal does not know this file.")
                        } else {
                            Text("\(vt.malicious) of \(vt.engines) engines flag it")
                                .font(.system(size: 13, weight: .semibold))
                                .foregroundStyle(vt.malicious >= 2 ? Theme.dangerText : vt.malicious == 1 ? Theme.warningText : Theme.successText)
                            if let label = vt.label { LabeledContent("Label", value: label) }
                            if let first = vt.firstSubmission {
                                LabeledContent("First seen", value: first.formatted(date: .abbreviated, time: .omitted))
                            }
                        }
                        Text("Looked up by hash\(at.map { ", \($0.formatted(.relative(presentation: .named)))" } ?? ""). The file was not sent.")
                            .font(.caption).foregroundStyle(.secondary)
                    } else {
                        Text(UserDefaults.standard.bool(forKey: TrafficModel.Key.lookUpVirusTotal)
                             ? "Not looked up yet." : "Automatic lookups are off. Press Check again to look this hash up.")
                            .foregroundStyle(.secondary)
                    }
                    if let e = traffic.lastError { Text(e).font(.caption).foregroundStyle(Theme.warningText) }
                    if check.status != .apple && traffic.keySaved { uploadBox(check) }
                }
                .font(.system(size: 12))
            }
        }
    }

    /// The upload: only after the warning dialog, whose confirm button makes
    /// the one value `upload` accepts (I9).
    private func uploadBox(_ check: SignatureCheck) -> some View {
        HStack {
            Button(uploading ? "Uploading…" : "Upload File to VirusTotal…") { confirmUpload = true }
                .disabled(uploading)
            Spacer()
        }
        .padding(.top, 4)
        .confirmationDialog("Upload this file to VirusTotal?", isPresented: $confirmUpload) {
            Button("Upload", role: .destructive) {
                // Review checkpoint I9: the only place that makes an UploadConfirmation.
                guard let confirmation = UploadConfirmation.userConfirmed(warningShown: VirusTotal.uploadWarning,
                                                                          file: URL(fileURLWithPath: check.path)) else { return }
                uploading = true
                Task {
                    note = await traffic.upload(confirmation, check: check)
                    uploading = false
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(VirusTotal.uploadWarning)
        }
    }

    private func askCard(_ app: AppTraffic, check: SignatureCheck) -> some View {
        let question = traffic.question(app: app, check: check)
        return VStack(alignment: .leading, spacing: 6) {
            SectionLabel("What is this?")
            Card(padding: 12) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(question).font(.system(size: 11.5)).textSelection(.enabled).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack {
                        ForEach(traffic.askServices, id: \.self) { service in
                            Button("Ask \(service.title)") { note = traffic.ask(service, question: question) }
                        }
                        Button("Copy Question") { model.copy(question) }
                        Spacer()
                    }
                    Text("The question opens in a new chat in your browser. It is shown here first, so you see what leaves this Mac.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
    }

    private func networkCard(_ app: AppTraffic) -> some View {
        let remotes = traffic.hour.map { AppTrafficText.remotes(of: app.app, in: $0) } ?? []
        return VStack(alignment: .leading, spacing: 6) {
            SectionLabel(text: "Network, last hour") {
                ViaChips(byVia: app.byVia)
            }
            Card {
                if remotes.isEmpty {
                    Text("No addresses.").foregroundStyle(.secondary).padding(12)
                }
                ForEach(Array(remotes.prefix(100).enumerated()), id: \.offset) { i, r in
                    if i > 0 { Divider().opacity(0.5) }
                    HStack(spacing: 6) {
                        VStack(alignment: .leading, spacing: 0) {
                            Text(AppTrafficText.label(r)).lineLimit(1).truncationMode(.middle)
                            Text(AppTrafficText.detail(r)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                        }
                        Spacer()
                        Chip(text: r.via.rawValue, kind: r.via == .router ? .accent : r.via == .proxy ? .proxy : .outline)
                        Text("↓ \(TrafficFormat.bytes(r.bytesIn))").frame(width: 70, alignment: .trailing)
                        Text("↑ \(TrafficFormat.bytes(r.bytesOut))").foregroundStyle(.secondary).frame(width: 66, alignment: .trailing)
                    }
                    .font(.system(size: 11, design: .monospaced))
                    .padding(.horizontal, 12)
                    .padding(.vertical, 4)
                }
            }
        }
    }

    private func actions(_ app: AppTraffic, path: String, check: SignatureCheck) -> some View {
        HStack {
            Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)]) }
            Button("Copy Path") { model.copy(path) }
            if traffic.isTrusted(check.sha256) {
                Button("Untrust") { traffic.untrust(check.sha256) }
            } else if traffic.verdict(check) == .needsReview {
                Button("Trust This File") { traffic.trust(check) }
                    .help("Trust is kept for this exact file: a changed file needs review again.")
            }
            Spacer()
            if AppChecks.canQuit(check.status) && !app.pids.isEmpty {
                Button("Quit Process…", role: .destructive) { confirmQuit = true }
                    .confirmationDialog("Quit \(app.name)?", isPresented: $confirmQuit) {
                        Button("Quit", role: .destructive) { quit(app) }
                        Button("Cancel", role: .cancel) {}
                    } message: {
                        Text("This sends SIGTERM to process \(app.pids.map(String.init).joined(separator: ", ")). Unsaved work in it can be lost.")
                    }
            }
        }
    }

    private func quit(_ app: AppTraffic) {
        var failed: [UInt32] = []
        for pid in app.pids where kill(pid_t(pid), SIGTERM) != 0 { failed.append(pid) }
        note = failed.isEmpty ? "Sent SIGTERM to \(app.name)." : "Could not stop process \(failed.map(String.init).joined(separator: ", "))."
    }
}
