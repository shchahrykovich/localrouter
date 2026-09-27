// The file part of "Uninstall LocalRouter…": the instance's data and logs
// folders, and its own CLI and Claude Code links. The app untrusts the CA and
// unregisters the daemon itself. Another instance's folders and links are
// never touched (ADR 04, I10).

import Foundation

public struct Uninstaller {
    public var dataDir: URL
    public var logsDir: URL
    public var cli: CLIInstaller
    public var claude: ClaudeInstaller

    /// The running instance's folders and links.
    public init(instance: Instance = .current) {
        self.init(dataDir: Paths.dataDir, logsDir: Paths.logsDir,
                  cli: CLIInstaller(instance: instance), claude: ClaudeInstaller(instance: instance))
    }

    public init(dataDir: URL, logsDir: URL, cli: CLIInstaller, claude: ClaudeInstaller) {
        self.dataDir = dataDir
        self.logsDir = logsDir
        self.cli = cli
        self.claude = claude
    }

    public func removeFiles() {
        let fm = FileManager.default
        try? fm.removeItem(at: dataDir)
        try? fm.removeItem(at: logsDir)
        cli.uninstall()
        claude.uninstall()
    }
}
