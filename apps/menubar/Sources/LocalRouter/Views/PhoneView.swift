import LocalRouterKit
import SwiftUI

/// The phone panel of the Proxy tab (ADR 10, change 3): a checklist until a
/// phone can use the proxy, then the QR code and the two values to type,
/// and the allowed devices.
struct PhoneView: View {
    @Environment(AppModel.self) private var model
    let done: () -> Void
    @State private var confirmRemove = false

    var body: some View {
        let setup = model.phoneSetup
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("Use the proxy from an iPhone").font(.headline)
                    Spacer()
                    Button("Done", action: done)
                }
                if setup.client != nil, setup.ready, let lan = model.phone?.lan, let url = lan.setupUrl {
                    qrSection(lan: lan, url: url)
                } else {
                    checklist(setup)
                    if setup.client == nil {
                        HStack {
                            Spacer()
                            Button("Set Up a Phone") { Task { await model.setUpPhone() } }
                                .disabled(!setup.ready)
                                .keyboardShortcut(.defaultAction)
                        }
                    }
                }
                ForEach(model.phone?.lan?.problems ?? [], id: \.self) { p in
                    Text(p).font(.caption).foregroundStyle(.orange)
                }
            }
            .padding(12)
        }
        .task { await model.loadPhone() }
    }

    @ViewBuilder private func checklist(_ setup: PhoneSetup) -> some View {
        ForEach(setup.rows) { row in
            HStack(spacing: 6) {
                Image(systemName: row.ok ? "checkmark.circle.fill" : "xmark.circle")
                    .foregroundStyle(row.ok ? .green : .secondary)
                Text(row.text)
                Spacer()
                if !row.ok, let fix = row.fix {
                    switch fix {
                    case .turnOnProxy: Button("Turn On") { Task { await model.fixPhone(fix) } }
                    case .turnOnLan: Button("Turn On") { Task { await model.fixPhone(fix) } }
                        .help("Other machines may then reach ports 80 and 443 on the allowed networks")
                    case .allowNetwork: Button("Allow") { Task { await model.fixPhone(fix) } }
                    }
                }
            }
        }
    }

    @ViewBuilder private func qrSection(lan: LanProxyInfo, url: String) -> some View {
        HStack(alignment: .top, spacing: 12) {
            if let image = PhoneSetup.qrImage(url) {
                Image(decorative: image, scale: 1)
                    .interpolation(.none)
                    .resizable()
                    .frame(width: 150, height: 150)
                    .padding(8)
                    .background(.white, in: RoundedRectangle(cornerRadius: 6))
            }
            VStack(alignment: .leading, spacing: 6) {
                Text("1. Scan with the iPhone camera. The page allows this iPhone and shows the next step.")
                Text("2. Settings → Wi-Fi → (i) → Configure Proxy, one of:")
                if let pac = lan.pacUrl {
                    Text("Automatic (recommended): on and off from this Mac").foregroundStyle(.secondary)
                    CopyLine(text: pac)
                }
                Text("Manual").foregroundStyle(.secondary)
                BigValue(label: "Server", value: lan.address ?? "unknown")
                BigValue(label: "Port", value: String(lan.port))
                Text("3. Install CA on the page, then trust it: Settings → General → About → Certificate Trust Settings. Until then, HTTPS sites do not open on the iPhone.")
            }
            .font(.callout)
        }
        Toggle("Send this iPhone through the proxy", isOn: Binding(
            get: { !model.phonePaused },
            set: { on in Task { await model.setPhonePaused(!on) } }
        ))
        .help("Off: the Automatic setting sends the iPhone direct. A Manual setting does not follow this switch.")
        devices(lan)
        Text("Automatic: turn the switch off here, and the iPhone goes direct (it may take until it rejoins the Wi-Fi). It also goes direct when the Mac sleeps. Manual: Configure Proxy → Off on the iPhone; while it is on, the iPhone has no internet on this Wi-Fi when the Mac sleeps. If the page does not open, the iPhone must be on the same Wi-Fi; some guest networks block devices from each other.")
            .font(.caption).foregroundStyle(.secondary)
        if confirmRemove {
            HStack {
                Text("Set Configure Proxy to Off on the iPhone first, or it has no internet on this Wi-Fi.")
                    .font(.caption).foregroundStyle(.orange)
                Spacer()
                Button("Cancel") { confirmRemove = false }
                Button("Remove") {
                    confirmRemove = false
                    Task { await model.removePhone() }
                }
            }
        } else {
            HStack {
                Button("New QR Code") { Task { await model.newSetupCode() } }
                    .help("The old QR code stops allowing devices. Allowed devices stay allowed.")
                Spacer()
                Button("Remove Phone") { confirmRemove = true }
            }
        }
    }

    @ViewBuilder private func devices(_ lan: LanProxyInfo) -> some View {
        let name = model.phoneSetup.client ?? ""
        if lan.devices.isEmpty {
            Text("No device is allowed yet: scan the QR code.").font(.caption).foregroundStyle(.secondary)
        } else {
            VStack(alignment: .leading, spacing: 2) {
                Text("Allowed devices").font(.caption).foregroundStyle(.secondary)
                ForEach(lan.devices, id: \.self) { address in
                    HStack {
                        Image(systemName: "iphone")
                        Text(address).font(.callout.monospaced())
                        Spacer()
                        Button("Remove") { Task { await model.setPhoneDevice(client: name, address: address, allow: false) } }
                            .buttonStyle(.borderless)
                    }
                }
            }
        }
    }
}

/// A value to type on the phone, large, with a copy button.
private struct BigValue: View {
    @Environment(AppModel.self) private var model
    let label: String
    let value: String
    var body: some View {
        HStack {
            Text(label).foregroundStyle(.secondary).frame(width: 48, alignment: .leading)
            Text(value).font(.title3.monospaced().weight(.semibold)).textSelection(.enabled)
            Button { model.copy(value) } label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless).help("Copy")
        }
    }
}

/// Devices that asked to use a phone client: Allow or Deny, as in Charles.
struct WaitingDevicesBanner: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ForEach(model.waitingDevices, id: \.device.address) { item in
            HStack(spacing: 6) {
                Image(systemName: "iphone.radiowaves.left.and.right").foregroundStyle(.orange)
                VStack(alignment: .leading, spacing: 0) {
                    Text("\(item.device.address) wants to use the proxy").font(.callout.weight(.semibold))
                    Text("asked for \(item.device.host)").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Deny") {
                    Task { await model.setPhoneDevice(client: item.client, address: item.device.address, allow: false) }
                }
                Button("Allow") {
                    Task { await model.setPhoneDevice(client: item.client, address: item.device.address, allow: true) }
                }
                .keyboardShortcut(.defaultAction)
            }
            .padding(8)
            .background(.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 6))
        }
    }
}
