// Who signed a program, checked on this Mac with no network (ADR 11,
// change 3, 3.1). The check is of the file on disk: a running program can
// differ from it after an update.

import CryptoKit
import Foundation
import Security

/// The result of a code signature check, in the order of the table in
/// ADR 11, 3.1.
public enum SignatureStatus: String, Codable, Sendable, CaseIterable {
    /// The requirement `anchor apple` holds: a part of macOS or an Apple app.
    case apple
    /// Valid, with a Developer ID or Mac App Store certificate.
    case developerID
    /// Valid, signed without an identity (`codesign -s -`): Homebrew, cargo
    /// and the user's own builds.
    case adhoc
    /// No signature at all.
    case unsigned
    /// A signature that does not match the file, or any other failure.
    case broken
}

public struct SignatureCheck: Codable, Equatable, Sendable {
    /// The executable that was checked.
    public var path: String
    public var status: SignatureStatus
    /// The common name of the signing certificate: "Developer ID
    /// Application: Google LLC (EQHXZ8M8AV)".
    public var signer: String?
    public var teamID: String?
    /// Whether Apple notarized it. Only for `developerID`; nil otherwise.
    public var notarized: Bool?
    /// SHA-256 of the file, lower-case hex.
    public var sha256: String
    /// Why the signature is `broken`, in macOS's words.
    public var failure: String?

    public init(path: String, status: SignatureStatus, signer: String? = nil, teamID: String? = nil,
                notarized: Bool? = nil, sha256: String, failure: String? = nil) {
        self.path = path
        self.status = status
        self.signer = signer
        self.teamID = teamID
        self.notarized = notarized
        self.sha256 = sha256
        self.failure = failure
    }
}

public enum AppIdentity {
    public enum Failure: Error, LocalizedError, Equatable {
        case cannotRead(String)

        public var errorDescription: String? {
            switch self {
            case let .cannotRead(text): text
            }
        }
    }

    /// Check one executable file. Throws only when the file cannot be read;
    /// a bad signature is a result (`broken`), not an error.
    public static func check(path: String) throws -> SignatureCheck {
        let hash = try sha256(path: path)
        var result = SignatureCheck(path: path, status: .broken, sha256: hash)
        var code: SecStaticCode?
        let url = URL(fileURLWithPath: path) as CFURL
        let created = SecStaticCodeCreateWithPath(url, [], &code)
        guard created == errSecSuccess, let code else {
            result.failure = message(created)
            return result
        }
        var cfError: Unmanaged<CFError>?
        let flags = SecCSFlags(rawValue: kSecCSCheckAllArchitectures | kSecCSStrictValidate)
        let valid = SecStaticCodeCheckValidityWithErrors(code, flags, nil, &cfError)
        if valid == errSecCSUnsigned {
            result.status = .unsigned
            return result
        }
        guard valid == errSecSuccess else {
            let error = cfError?.takeRetainedValue()
            result.failure = error.map { CFErrorCopyDescription($0) as String } ?? message(valid)
            return result
        }
        // The signer and the flags.
        var info: CFDictionary?
        if SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &info) == errSecSuccess,
           let dict = info as? [String: Any] {
            result.teamID = dict[kSecCodeInfoTeamIdentifier as String] as? String
            if let certs = dict[kSecCodeInfoCertificates as String] as? [SecCertificate], let leaf = certs.first {
                result.signer = SecCertificateCopySubjectSummary(leaf) as String?
            }
            let codeFlags = (dict[kSecCodeInfoFlags as String] as? NSNumber)?.uint32Value ?? 0
            if codeFlags & SecCodeSignatureFlags.adhoc.rawValue != 0 {
                result.status = .adhoc
                return result
            }
        }
        if satisfies(code, "anchor apple") {
            result.status = .apple
            return result
        }
        result.status = .developerID
        result.notarized = satisfies(code, "notarized")
        return result
    }

    /// SHA-256 of a file, read in 1 MB pieces.
    public static func sha256(path: String) throws -> String {
        guard let handle = FileHandle(forReadingAtPath: path) else {
            throw Failure.cannotRead("Cannot read \(path).")
        }
        defer { try? handle.close() }
        var hasher = SHA256()
        while true {
            let piece: Data
            do {
                piece = try handle.read(upToCount: 1 << 20) ?? Data()
            } catch {
                throw Failure.cannotRead("Cannot read \(path): \(error.localizedDescription)")
            }
            if piece.isEmpty { break }
            hasher.update(data: piece)
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    static func satisfies(_ code: SecStaticCode, _ text: String) -> Bool {
        var requirement: SecRequirement?
        guard SecRequirementCreateWithString(text as CFString, [], &requirement) == errSecSuccess,
              let requirement else { return false }
        return SecStaticCodeCheckValidity(code, [], requirement) == errSecSuccess
    }

    static func message(_ status: OSStatus) -> String {
        (SecCopyErrorMessageString(status, nil) as String?) ?? "error \(status)"
    }
}

extension SignatureStatus {
    /// The words of the details window: "Apple", "not signed".
    public var label: String {
        switch self {
        case .apple: "Apple"
        case .developerID: "Developer ID"
        case .adhoc: "ad-hoc signed"
        case .unsigned: "not signed"
        case .broken: "signature does not match the file"
        }
    }
}
