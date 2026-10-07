import LocalRouterKit
import SwiftUI

/// The phone panel of the Proxy tab (ADR 10, change 3): a checklist until a
/// phone can use the proxy, then three numbered steps (the QR code, the
/// values to type, the CA), the switch and the allowed devices.
struct PhoneView: View {
    @Environment(AppModel.self) private var model
    let done: () -> Void

    var body: some View {
        let setup = model.phoneSetup
        VStack(spacing: 0) {
            topBar
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if setup.client != nil, setup.ready, let lan = model.phone?.lan, let url = lan.setupUrl {
                        steps(lan: lan, url: url)
                    } else {
                        checklist(setup)
                    }
                    ForEach(model.phone?.lan?.problems ?? [], id: \.self) { p in
                        Text(p).font(.caption).foregroundStyle(Theme.warningText)
                    }
                }
                .padding(14)
            }
        }
        .task { await model.loadPhone() }
    }

    private var topBar: some View {
        ZStack {
            Text("Use the proxy from an iPhone").fontWeight(.semibold)
            HStack {
                Button(action: done) {
                    HStack(spacing: 2) {
                        Image(systemName: "chevron.left").font(.system(size: 12, weight: .semibold))
                        Text("Proxy")
                    }
                    .foregroundStyle(Theme.accent)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.cancelAction)
                .help("Back to the proxy")
                Spacer()
            }
        }
        .padding(.horizontal, 14)
        .padding(.bottom, 10)
    }

    // MARK: Before a phone can use the proxy

    @ViewBuilder private func checklist(_ setup: PhoneSetup) -> some View {
        Card {
            ForEach(Array(setup.rows.enumerated()), id: \.element.id) { index, row in
                if index > 0 { Divider() }
                HStack(spacing: 8) {
                    Image(systemName: row.ok ? "checkmark.circle.fill" : "circle")
                        .foregroundStyle(row.ok ? Theme.online : .secondary)
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
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
            }
        }
        if setup.client == nil {
            HStack {
                Spacer()
                Button("Set Up a Phone") { Task { await model.setUpPhone() } }
                    .buttonStyle(.borderedProminent)
                    .tint(Theme.accentFill)
                    .disabled(!setup.ready)
                    .keyboardShortcut(.defaultAction)
            }
        }
    }

    // MARK: The three steps

    @ViewBuilder private func steps(lan: LanProxyInfo, url: String) -> some View {
        Card(padding: 14) {
            HStack(alignment: .top, spacing: 14) {
                if let image = PhoneSetup.qrImage(url) {
                    Image(decorative: image, scale: 1)
                        .interpolation(.none)
                        .resizable()
                        .frame(width: 124, height: 124)
                        .padding(8)
                        .background(.white, in: RoundedRectangle(cornerRadius: 8))
                        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.line, lineWidth: 0.5))
                        .accessibilityLabel("QR code that opens the setup page")
                }
                VStack(alignment: .leading, spacing: 6) {
                    StepTitle(number: 1, text: "Scan with the iPhone camera")
                    Text("The page allows this iPhone and shows the next step.").foregroundStyle(.secondary)
                    Text("The iPhone must be on the same Wi-Fi as this Mac.").font(.caption).foregroundStyle(.secondary)
                    Spacer(minLength: 4)
                    LinkButton("New QR code") { Task { await model.newSetupCode() } }
                        .help("The old QR code stops allowing devices. Allowed devices stay allowed.")
                }
            }
        }

        Card(padding: 14) {
            VStack(alignment: .leading, spacing: 10) {
                StepTitle(number: 2, text: "Settings → Wi-Fi → ⓘ → Configure Proxy")
                HStack(alignment: .top, spacing: 8) {
                    if let pac = lan.pacUrl {
                        VStack(alignment: .leading, spacing: 6) {
                            HStack(spacing: 6) {
                                Text("Automatic").fontWeight(.semibold)
                                Chip(text: "Recommended", kind: .solid)
                            }
                            Text("On and off from this Mac. URL:").font(.caption).foregroundStyle(.secondary)
                            CopyLine(text: pac)
                        }
                        .padding(10)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                        .background(Theme.rowHover, in: RoundedRectangle(cornerRadius: 9))
                        .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(Theme.accent, lineWidth: 1.5))
                    }
                    VStack(alignment: .leading, spacing: 6) {
                        Text("Manual").fontWeight(.semibold)
                        BigValue(label: "Server", value: lan.address ?? "unknown")
                        BigValue(label: "Port", value: String(lan.port))
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                    .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(Theme.line, lineWidth: 1))
                }
                .fixedSize(horizontal: false, vertical: true)
            }
        }

        Card(padding: 14) {
            VStack(alignment: .leading, spacing: 6) {
                StepTitle(number: 3, text: "Install the CA, then trust it")
                Text("Install it from the setup page. Then: Settings → General → About → Certificate Trust Settings.")
                    .foregroundStyle(.secondary)
                Text("Until then, HTTPS sites do not open on the iPhone.").font(.caption).foregroundStyle(Theme.warningText)
            }
        }

        Card {
            Toggle(isOn: Binding(
                get: { !model.phonePaused },
                set: { on in Task { await model.setPhonePaused(!on) } }
            )) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Send this iPhone through the proxy").fontWeight(.semibold)
                    Text("Off: the Automatic setting sends it direct. Manual does not follow this switch.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .toggleStyle(.switch)
            .tint(Theme.online)
            .padding(.horizontal, 14)
            .padding(.vertical, 11)
            Divider()
            devices(lan)
        }

        Text("Automatic: turn the switch off here, and the iPhone goes direct (it may take until it rejoins the Wi-Fi). It also goes direct when the Mac sleeps. Manual: Configure Proxy → Off on the iPhone; while it is on, the iPhone has no internet on this Wi-Fi when the Mac sleeps. If the page does not open, the iPhone must be on the same Wi-Fi; some guest networks block devices from each other.")
            .font(.caption).foregroundStyle(.secondary)
            .padding(.horizontal, 4)

        HStack(spacing: 8) {
            Text("Before you remove the phone, set Configure Proxy to Off on it.")
                .font(.caption).foregroundStyle(.secondary)
            Spacer()
            Button("Remove Phone", role: .destructive) { Task { await model.removePhone() } }
                .foregroundStyle(Theme.dangerText)
        }
    }

    @ViewBuilder private func devices(_ lan: LanProxyInfo) -> some View {
        let name = model.phoneSetup.client ?? ""
        VStack(alignment: .leading, spacing: 4) {
            Text("Allowed devices").textCase(.uppercase).kerning(0.3)
                .font(.system(size: 11, weight: .semibold)).foregroundStyle(.secondary)
            if lan.devices.isEmpty {
                Text("No device is allowed yet: scan the QR code.").font(.caption).foregroundStyle(.secondary)
            }
            ForEach(lan.devices, id: \.self) { address in
                HStack(spacing: 8) {
                    Image(systemName: "iphone").foregroundStyle(.secondary)
                    Text(address).font(.system(size: 12, design: .monospaced))
                    Spacer()
                    LinkButton("Remove") { Task { await model.setPhoneDevice(client: name, address: address, allow: false) } }
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }
}

/// A step number in an accent circle, and its title.
private struct StepTitle: View {
    let number: Int
    let text: String
    var body: some View {
        HStack(spacing: 8) {
            Text("\(number)").font(.system(size: 11, weight: .bold)).foregroundStyle(.white)
                .frame(width: 20, height: 20).background(Theme.accentFill, in: Circle())
            Text(text).fontWeight(.semibold)
        }
    }
}

/// A value to type on the phone, large, with a copy button.
private struct BigValue: View {
    @Environment(AppModel.self) private var model
    let label: String
    let value: String
    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(label).font(.caption).foregroundStyle(.secondary)
            HStack(spacing: 4) {
                Text(value).font(.system(size: 15, weight: .semibold, design: .monospaced)).textSelection(.enabled)
                IconButton(symbol: "doc.on.doc", help: "Copy", size: 22) { model.copy(value) }
            }
        }
    }
}

/// Devices that asked to use a phone client: Allow or Deny, as in Charles.
struct WaitingDevicesBanner: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ForEach(model.waitingDevices, id: \.device.address) { item in
            HStack(spacing: 10) {
                Image(systemName: "iphone.radiowaves.left.and.right").font(.system(size: 16)).foregroundStyle(Theme.warningText)
                VStack(alignment: .leading, spacing: 1) {
                    Text("\(item.device.address) wants to use the proxy").fontWeight(.semibold)
                    Text("It asked for \(item.device.host)").font(.caption).foregroundStyle(Theme.warningText)
                }
                Spacer()
                Button("Deny") {
                    Task { await model.setPhoneDevice(client: item.client, address: item.device.address, allow: false) }
                }
                Button("Allow") {
                    Task { await model.setPhoneDevice(client: item.client, address: item.device.address, allow: true) }
                }
                .buttonStyle(.borderedProminent)
                .tint(Theme.accentFill)
                .keyboardShortcut(.defaultAction)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(Theme.warningSoft, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Theme.warningLine, lineWidth: 1))
        }
    }
}
