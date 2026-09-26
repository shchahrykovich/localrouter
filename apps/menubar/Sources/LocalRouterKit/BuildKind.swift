// Release builds are signed with a Developer ID, so they carry a Team ID.
// Local builds (`swift run`, scripts/build-app.sh, scripts/install.sh) are
// unsigned or ad-hoc signed and have none. The menu bar icon is orange for a
// local build, so it cannot be mistaken for the installed release. The updater
// uses the same test to refuse ad-hoc builds.

import Foundation

public enum BuildKind: Equatable, Sendable {
    case release
    case dev

    public static func of(teamID: String?) -> BuildKind {
        teamID?.isEmpty == false ? .release : .dev
    }

    /// The kind of the running app.
    public static var current: BuildKind {
        of(teamID: Updater.teamIdentifier(of: Bundle.main.bundleURL))
    }
}
