// "Install Claude Code Instructions": a symlink ~/.claude/LocalRouter.md
// pointing into the app bundle, and one line `@LocalRouter.md` at the top of
// ~/.claude/CLAUDE.md, so every Claude Code session reads the note.
// The same design as VibeViewer's VV.md:
//   1. A link, never a copy: an update of the app updates the note.
//   2. A name that is taken is left alone: only a link into some
//      LocalRouter bundle is replaced. ~/.claude is edited by hand.
//   3. No ~/.claude folder means no Claude Code here: nothing is created.

import Foundation

public struct ClaudeInstaller {
    public enum Outcome: Equatable {
        /// The note is linked. `importAdded` is false when CLAUDE.md already
        /// had the line.
        case installed(link: URL, importAdded: Bool)
        /// There is no ~/.claude folder, so nothing was done.
        case noClaude(URL)
    }

    public enum Failure: Error, LocalizedError, Equatable {
        case noteMissing(URL)
        case occupied(URL)
        case io(String)

        public var errorDescription: String? {
            switch self {
            case let .noteMissing(url): "The Claude Code note is missing from the app: \(url.path)"
            case let .occupied(url): "\(url.path) already exists and is not a LocalRouter link. Remove it first."
            case let .io(text): text
            }
        }
    }

    public static let noteName = "LocalRouter.md"
    /// The line in CLAUDE.md that imports the note. Claude Code reads the
    /// name relative to the folder CLAUDE.md is in.
    public static let importLine = "@\(noteName)"

    public var note: URL
    public var claudeDir: URL

    /// The note of the running app and ~/.claude.
    public init(bundle: Bundle = .main,
                claudeDir: URL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".claude")) {
        self.note = bundle.bundleURL.appendingPathComponent(BundleLayout.claudeNote)
        self.claudeDir = claudeDir
    }

    public init(note: URL, claudeDir: URL) {
        self.note = note
        self.claudeDir = claudeDir
    }

    public var link: URL { claudeDir.appendingPathComponent(Self.noteName) }
    public var claudeMD: URL { claudeDir.appendingPathComponent("CLAUDE.md") }

    /// Link the note, then make sure CLAUDE.md imports it. The line is added
    /// only after the link exists: an import of a missing file would fail in
    /// every session.
    public func install() throws -> Outcome {
        let fm = FileManager.default
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: claudeDir.path, isDirectory: &isDir), isDir.boolValue else {
            return .noClaude(claudeDir)
        }
        guard fm.fileExists(atPath: note.path) else { throw Failure.noteMissing(note) }
        try linkNote()
        return .installed(link: link, importAdded: try ensureImport())
    }

    /// Remove the link if it is ours. The line in CLAUDE.md stays: it is the
    /// user's file, and an import of a missing file does no harm.
    public func uninstall() {
        if let dest = try? FileManager.default.destinationOfSymbolicLink(atPath: link.path), Self.isOurNote(dest) {
            try? FileManager.default.removeItem(at: link)
        }
    }

    /// A link into any LocalRouter bundle, wherever it is and even if the
    /// bundle is gone, has this shape.
    static func isOurNote(_ path: String) -> Bool {
        path.hasSuffix("/" + BundleLayout.claudeNote)
    }

    private func linkNote() throws {
        let fm = FileManager.default
        // destinationOfSymbolicLink, not fileExists: fileExists follows the
        // link and says "nothing there" for a link into a deleted bundle.
        if let existing = try? fm.destinationOfSymbolicLink(atPath: link.path) {
            if existing == note.path { return }
            guard Self.isOurNote(existing) else { throw Failure.occupied(link) }
            try? fm.removeItem(at: link)
        } else if fm.fileExists(atPath: link.path) {
            throw Failure.occupied(link)
        }
        do {
            try fm.createSymbolicLink(atPath: link.path, withDestinationPath: note.path)
        } catch {
            throw Failure.io("cannot link \(link.path): \(error.localizedDescription)")
        }
    }

    /// Returns true when the line was added.
    private func ensureImport() throws -> Bool {
        // CLAUDE.md may be a link into a dotfiles repository. Write the real
        // file, so the link stays.
        let file = claudeMD.resolvingSymlinksInPath()
        let existing: String
        do {
            existing = try String(contentsOf: file, encoding: .utf8)
        } catch let error as CocoaError where error.code == .fileReadNoSuchFile {
            existing = ""
        } catch {
            // A file that cannot be read is not an empty file: writing would
            // replace what the user has.
            throw Failure.io("cannot read \(claudeMD.path): \(error.localizedDescription)")
        }
        guard let text = Self.withImport(existing) else { return false }
        do {
            try Data(text.utf8).write(to: file, options: .atomic)
        } catch {
            throw Failure.io("cannot add \(Self.importLine) to \(claudeMD.path): \(error.localizedDescription)")
        }
        return true
    }

    /// CLAUDE.md with the import line added as the first line, or nil when a
    /// line already is the import. A sentence that only mentions it does not
    /// count. The rest of the file follows unchanged.
    static func withImport(_ existing: String) -> String? {
        if existing.split(separator: "\n", omittingEmptySubsequences: false)
            .contains(where: { $0.trimmingCharacters(in: .whitespaces) == importLine }) {
            return nil
        }
        return importLine + "\n" + existing
    }
}
