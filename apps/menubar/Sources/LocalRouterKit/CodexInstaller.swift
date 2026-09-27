// Links the bundled guide into Codex home and adds an explicit read instruction.
// Codex does not use Claude Code’s @file import syntax.

import Foundation

public struct CodexInstaller {
    public enum Outcome: Equatable {
        /// The note is linked. `instructionAdded` is false when AGENTS.md already
        /// had the line.
        case installed(link: URL, agentsMD: URL, instructionAdded: Bool)
        /// There is no ~/.codex folder, so nothing was done.
        case noCodex(URL)
    }

    public enum Failure: Error, LocalizedError, Equatable {
        case noteMissing(URL)
        case occupied(URL)
        case io(String)

        public var errorDescription: String? {
            switch self {
            case let .noteMissing(url): "The Codex note is missing from the app: \(url.path)"
            case let .occupied(url): "\(url.path) already exists and is not a LocalRouter link. Remove it first."
            case let .io(text): text
            }
        }
    }

    /// Use an absolute path: Codex may be working in any project.
    public var instructionLine: String {
        "Before working with local dev servers, read and follow the instructions in `\(link.path)` if that file exists."
    }

    public var note: URL
    public var codexDir: URL
    public var instance: Instance

    /// The note of the running app and ~/.codex.
    public init(bundle: Bundle = .main, instance: Instance = .current,
                codexDir: URL = CodexInstaller.defaultDirectory()) {
        self.note = bundle.bundleURL.appendingPathComponent(BundleLayout.claudeNote)
        self.codexDir = codexDir
        self.instance = instance
    }

    public init(note: URL, codexDir: URL, instance: Instance = .current) {
        self.note = note
        self.codexDir = codexDir
        self.instance = instance
    }

    public var link: URL { codexDir.appendingPathComponent(instance.note) }
    public static func defaultDirectory(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        home: URL = FileManager.default.homeDirectoryForCurrentUser
    ) -> URL {
        if let path = environment["CODEX_HOME"], !path.isEmpty {
            return URL(fileURLWithPath: (path as NSString).expandingTildeInPath)
        }
        return home.appendingPathComponent(".codex")
    }

    /// Codex loads only the first non-empty global instruction file.
    public func instructionsFile() throws -> URL {
        let override = codexDir.appendingPathComponent("AGENTS.override.md")
        if !((try readInstructions(override)).trimmingCharacters(in: .whitespacesAndNewlines)).isEmpty {
            return override
        }
        return codexDir.appendingPathComponent("AGENTS.md")
    }

    /// Check the existing instructions before linking, then prepend the read
    /// instruction only after the note is available.
    public func install() throws -> Outcome {
        let fm = FileManager.default
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: codexDir.path, isDirectory: &isDir), isDir.boolValue else {
            return .noCodex(codexDir)
        }
        guard fm.fileExists(atPath: note.path) else { throw Failure.noteMissing(note) }
        let file = try instructionsFile()
        let existing = try readInstructions(file)
        try linkNote()
        return .installed(link: link, agentsMD: file, instructionAdded: try ensureInstruction(file, existing: existing))
    }

    /// Leave the conditional instruction in the user’s file when removing our link.
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

    private func readInstructions(_ url: URL) throws -> String {
        do {
            return try String(contentsOf: url.resolvingSymlinksInPath(), encoding: .utf8)
        } catch let error as CocoaError where error.code == .fileReadNoSuchFile {
            return ""
        } catch {
            throw Failure.io("cannot read \(url.path): \(error.localizedDescription)")
        }
    }

    private func ensureInstruction(_ url: URL, existing: String) throws -> Bool {
        guard let text = Self.withInstruction(existing, line: instructionLine) else { return false }
        do {
            // Preserve a user's symlink into a dotfiles repository.
            try Data(text.utf8).write(to: url.resolvingSymlinksInPath(), options: .atomic)
        } catch {
            throw Failure.io("cannot update \(url.path): \(error.localizedDescription)")
        }
        return true
    }

    static func withInstruction(_ existing: String, line: String) -> String? {
        if existing.split(separator: "\n", omittingEmptySubsequences: false)
            .contains(where: { $0.trimmingCharacters(in: .whitespaces) == line }) {
            return nil
        }
        return line + "\n\n" + existing
    }
}
