// What a menu command tells the user when it finishes: the title and text of
// an alert, and the same words in the window footer.

import Foundation

public struct Feedback: Equatable, Sendable {
    public var title: String
    public var detail: String
    /// Shown as a warning, not as information.
    public var failed: Bool

    public init(title: String, detail: String, failed: Bool = false) {
        self.title = title
        self.detail = detail
        self.failed = failed
    }

    /// One line for the window footer.
    public var summary: String { detail.isEmpty ? title : "\(title). \(detail)" }

    public static func cliInstalled(link: URL, onPath: Bool, pathHint: String) -> Feedback {
        onPath
            ? Feedback(title: "Command line tool installed",
                       detail: "\(link.path) links to this app. Open a new terminal and run: \(link.lastPathComponent) status")
            : Feedback(title: "Command line tool installed",
                       detail: "\(link.path) links to this app, but \(link.deletingLastPathComponent().path) is not on PATH. Add it with:\n\(pathHint)")
    }

    public static func claudeInstalled(link: URL, claudeMD: URL, importAdded: Bool) -> Feedback {
        // The link is named after the note, so its name gives the import line.
        Feedback(title: "Claude Code instructions installed",
                 detail: importAdded
                     ? "\(link.path) links to this app, and @\(link.lastPathComponent) is now the first line of \(claudeMD.path). New Claude Code sessions read it."
                     : "\(link.path) links to this app. \(claudeMD.path) already reads it.")
    }

    public static func noClaude(_ dir: URL) -> Feedback {
        Feedback(title: "Claude Code is not installed",
                 detail: "\(dir.path) does not exist. Install Claude Code, then try again.",
                 failed: true)
    }

    public static func upToDate(version: String) -> Feedback {
        Feedback(title: "LocalRouter is up to date", detail: "Version \(version) is the latest version.")
    }

    public static func updateAvailable(_ release: Updater.Release, current: String) -> Feedback {
        Feedback(title: "LocalRouter \(release.version) is available",
                 detail: "You have version \(current). LocalRouter restarts after the update.")
    }

    public static func failed(_ title: String, _ reason: String) -> Feedback {
        Feedback(title: title, detail: reason, failed: true)
    }
}
