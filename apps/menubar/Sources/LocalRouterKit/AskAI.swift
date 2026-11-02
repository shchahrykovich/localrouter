// "What is this?": a question about a program, opened in Claude or ChatGPT
// by a link (ADR 11, change 4). LocalRouter has no chat of its own. The
// question is shown in the details window before any button is pressed, so
// the user sees what leaves the Mac.
//
// Rules (I10): the home folder is written as `~` and the user name never
// appears; the whole URL stays under 2,000 characters.

import Foundation

public enum AskAI {
    /// The longest URL a button opens.
    public static let maxURLLength = 2_000

    public enum Service: String, CaseIterable, Sendable {
        case claude
        case chatGPT

        public var title: String {
            switch self {
            case .claude: "Ask Claude"
            case .chatGPT: "Ask ChatGPT"
            }
        }

        /// A new chat with the question as `q`.
        public var base: String {
            switch self {
            case .claude: "https://claude.ai/new"
            case .chatGPT: "https://chatgpt.com/"
            }
        }

        /// The button copies the question and opens `base` instead, when
        /// the service ignores `q` (U3, checked by hand in M0). Not
        /// verified yet: both assume `q` works.
        public var copyFallback: Bool {
            switch self {
            case .claude: false
            case .chatGPT: false
            }
        }
    }

    /// One address the program sent data to.
    public struct Address: Equatable, Sendable {
        public var address: String
        /// "Netherlands", "London, United Kingdom"; nil when unknown.
        public var place: String?
        public var bytesOut: UInt64

        public init(address: String, place: String? = nil, bytesOut: UInt64) {
            self.address = address
            self.place = place
            self.bytesOut = bytesOut
        }
    }

    /// What the question is about.
    public struct Subject: Equatable, Sendable {
        public var name: String
        /// The executable path.
        public var path: String
        /// How it starts: "LaunchAgent com.helperd.agent". Nil when unknown.
        public var startsAs: String?
        public var signature: SignatureStatus
        public var signer: String?
        public var notarized: Bool?
        public var sha256: String
        public var virusTotal: VirusTotalReport?
        /// Largest first.
        public var addresses: [Address]

        public init(name: String, path: String, startsAs: String? = nil, signature: SignatureStatus, signer: String? = nil,
                    notarized: Bool? = nil, sha256: String, virusTotal: VirusTotalReport? = nil, addresses: [Address] = []) {
            self.name = name
            self.path = path
            self.startsAs = startsAs
            self.signature = signature
            self.signer = signer
            self.notarized = notarized
            self.sha256 = sha256
            self.virusTotal = virusTotal
            self.addresses = addresses
        }
    }

    /// The question, in English. Addresses only with `includeAddresses`
    /// (Settings > Traffic, on by default). When the URL would be too long,
    /// the address list goes first, then the hash is shortened.
    public static func question(for subject: Subject, includeAddresses: Bool, home: String = NSHomeDirectory()) -> String {
        let attempts: [(Bool, Bool)] = [(includeAddresses, false), (false, false), (false, true)]
        var text = ""
        for (addresses, shortHash) in attempts {
            text = compose(subject, addresses: addresses, shortHash: shortHash, home: home)
            if fits(text) { return text }
        }
        // Still too long (a very long path): cut the middle of the text.
        while !fits(text), text.count > 40 {
            let keep = text.count * 9 / 10
            text = String(text.prefix(keep / 2)) + "…" + String(text.suffix(keep / 2))
        }
        return text
    }

    /// The link of a button. With `copyFallback` it is the service's base
    /// URL: the button then copies the question first.
    public static func url(question: String, service: Service) -> URL {
        if service.copyFallback { return URL(string: service.base)! }
        let encoded = question.addingPercentEncoding(withAllowedCharacters: queryAllowed) ?? ""
        return URL(string: "\(service.base)?q=\(encoded)")!
    }

    /// Letters, digits and `-._~`: everything else is escaped, so `&`, `+`,
    /// `#` and `=` in a path cannot break the query.
    static let queryAllowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~")

    static func fits(_ text: String) -> Bool {
        Service.allCases.allSatisfy { url(question: text, service: $0).absoluteString.count < maxURLLength }
    }

    static func compose(_ s: Subject, addresses: Bool, shortHash: Bool, home: String) -> String {
        var parts: [String] = []
        var first = "What is the macOS program \"\(s.name)\"? It runs from \(s.path)"
        if let startsAs = s.startsAs, !startsAs.isEmpty {
            first += " and starts as \(startsAs)"
        }
        parts.append(first + ".")
        parts.append(signatureSentence(s))
        let hash = shortHash && s.sha256.count > 10 ? "\(s.sha256.prefix(4))…\(s.sha256.suffix(6))" : s.sha256
        parts.append("SHA-256 \(hash).")
        if let vt = s.virusTotal {
            switch vt.state {
            case .unknown:
                parts.append("VirusTotal does not know this file.")
            case .found:
                var line = "VirusTotal: \(vt.malicious) of \(vt.engines) engines"
                if let label = vt.label, !label.isEmpty { line += ", \"\(label)\"" }
                parts.append(line + ".")
            }
        }
        if addresses, !s.addresses.isEmpty {
            let list = s.addresses.map { a in
                "\(TrafficFormat.bytes(a.bytesOut)) to \(a.address)" + (a.place.map { " (\($0))" } ?? "")
            }
            parts.append("It sent \(list.joined(separator: ", ")).")
        }
        parts.append("Is it safe, and how do I remove it?")
        return withoutHome(parts.joined(separator: " "), home: home)
    }

    static func signatureSentence(_ s: Subject) -> String {
        switch s.signature {
        case .apple: return "It is signed by Apple."
        case .unsigned: return "It has no code signature."
        case .broken: return "Its code signature does not match the file."
        case .adhoc: return "It is ad-hoc signed, with no developer identity."
        case .developerID:
            let who = s.signer.map { "signed by \($0)" } ?? "signed with a Developer ID"
            switch s.notarized {
            case true?: return "It is \(who) and notarized by Apple."
            case false?: return "It is \(who) and not notarized."
            case nil: return "It is \(who)."
            }
        }
    }

    /// The home folder becomes `~`; another user's folder `/Users/…`. The
    /// user name never appears (I10).
    static func withoutHome(_ text: String, home: String) -> String {
        var out = text
        let home = home.hasSuffix("/") ? String(home.dropLast()) : home
        // Only the whole folder: home `/Users/al` must not touch `/Users/alice`.
        if !home.isEmpty, home != "/",
           let own = try? NSRegularExpression(pattern: NSRegularExpression.escapedPattern(for: home) + "(?=/|\\s|\"|\\.\\s|\\.$|$)") {
            out = own.stringByReplacingMatches(in: out, range: NSRange(out.startIndex..., in: out), withTemplate: "~")
        }
        if let regex = try? NSRegularExpression(pattern: "/Users/(?!Shared/)[^/\\s\"]+") {
            let range = NSRange(out.startIndex..., in: out)
            out = regex.stringByReplacingMatches(in: out, range: range, withTemplate: "/Users/…")
        }
        return out
    }
}
