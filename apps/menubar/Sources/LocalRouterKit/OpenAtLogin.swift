import Foundation
import ServiceManagement

/// The part of `SMAppService` that the Open at login switch uses, so tests
/// can use a fake in place of the real login item.
public protocol LoginItemService {
    var status: SMAppService.Status { get }
    func register() throws
    func unregister() throws
}

extension SMAppService: LoginItemService {}

/// The Open at login switch in Settings: starts the menu bar app when the
/// user logs in. The daemon has its own LaunchAgent and starts at login
/// either way.
public struct OpenAtLogin {
    private let service: LoginItemService

    public init(service: LoginItemService = SMAppService.mainApp) {
        self.service = service
    }

    /// On when macOS will open the app at login, or will once the user
    /// allows it in Login Items.
    public var isOn: Bool {
        service.status == .enabled || service.status == .requiresApproval
    }

    /// What the user must still do, if anything.
    public var note: String? {
        service.status == .requiresApproval ? "Allow LocalRouter in System Settings → General → Login Items." : nil
    }

    /// Set once the user or the first launch has chosen, so a switch the user
    /// turned off stays off.
    static let chosenKey = "openAtLoginChosen"

    /// On by default: the first launch turns the login item on, so the app
    /// is back after a restart without the user starting it. Later launches
    /// keep what the user chose. Returns an error text on failure.
    @discardableResult
    public func turnOnAtFirstLaunch(_ defaults: UserDefaults = .standard) -> String? {
        guard !defaults.bool(forKey: Self.chosenKey) else { return nil }
        let problem = set(true)
        // Try again at the next launch if macOS refused.
        if problem == nil { defaults.set(true, forKey: Self.chosenKey) }
        return problem
    }

    /// The user moved the switch: turns the login item on or off and
    /// remembers the choice. Returns an error text on failure.
    public func choose(_ on: Bool, _ defaults: UserDefaults = .standard) -> String? {
        defaults.set(true, forKey: Self.chosenKey)
        return set(on)
    }

    /// Turns the login item on or off. Returns an error text on failure.
    public func set(_ on: Bool) -> String? {
        guard on != isOn else { return nil }
        do {
            if on { try service.register() } else { try service.unregister() }
            return nil
        } catch {
            return "Could not \(on ? "turn on" : "turn off") Open at login: \(error.localizedDescription)"
        }
    }
}
