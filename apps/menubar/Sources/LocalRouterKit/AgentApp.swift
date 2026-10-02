import AppKit
import Foundation

/// A desktop app with a coding agent that a link can open with a prompt
/// already typed: "Help with Claude" and "Help with Codex" in the menu.
public enum AgentApp: String, CaseIterable, Sendable {
    case claude, codex

    public var name: String {
        switch self {
        case .claude: "Claude"
        case .codex: "Codex"
        }
    }

    /// The URL scheme the app registers in its Info.plist.
    public var scheme: String { rawValue }

    /// The query parameter that holds the prompt.
    var promptParameter: String {
        switch self {
        case .claude: "q"
        case .codex: "prompt"
        }
    }

    /// Claude opens a new Claude Code session, Codex a new local thread; both
    /// put the prompt in the composer and wait for the person to send it.
    /// Claude: support.claude.com/en/articles/14729294. Codex: the "Deep
    /// links" part of the Codex app commands reference.
    public func link(prompt: String) -> URL {
        let path = switch self {
        case .claude: "code/new"
        case .codex: "new"
        }
        // Only unreserved characters stay as they are, so `&`, `=`, `+` and
        // `#` in the prompt cannot end or change the value.
        let value = prompt.addingPercentEncoding(withAllowedCharacters: Self.unreserved) ?? ""
        return URL(string: "\(scheme)://\(path)?\(promptParameter)=\(value)")!
    }

    private static let unreserved = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~")

    /// True when some app handles the scheme. Asked each time the menu opens.
    @MainActor public var isInstalled: Bool {
        guard let probe = URL(string: "\(scheme)://") else { return false }
        return NSWorkspace.shared.urlForApplication(toOpen: probe) != nil
    }
}
