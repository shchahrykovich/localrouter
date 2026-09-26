// Which CA trust buttons Settings shows for the trust state in `status`.

public enum TrustActions: Equatable, Sendable {
    case trust
    case untrust

    /// `trusted` is `CaStatus.trusted`; `nil` means the check could not run.
    public static func `for`(trusted: Bool?) -> [TrustActions] {
        switch trusted {
        case true: [.untrust]
        case false: [.trust]
        case nil: [.trust, .untrust]
        }
    }
}
