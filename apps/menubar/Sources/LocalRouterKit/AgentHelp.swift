// The daemon serves instructions for coding agents at router.localhost.

public enum AgentHelp {
    public static let host = "router.localhost"

    /// The help page URL for the ports the daemon has bound. Plain HTTP first:
    /// curl reads it without trusting the local CA.
    public static func url(httpPort: UInt16?, httpsPort: UInt16?) -> String {
        switch (httpPort, httpsPort) {
        case let (http?, _): http == 80 ? "http://\(host)" : "http://\(host):\(http)"
        case let (nil, https?): https == 443 ? "https://\(host)" : "https://\(host):\(https)"
        case (nil, nil): "http://\(host)"
        }
    }

    /// What the user pastes into a coding agent.
    public static func prompt(url: String) -> String {
        "Run curl -s \(url) and follow it to add LocalRouter to this project."
    }

    /// The prompt for "Help with Claude" and "Help with Codex": the agent
    /// reads the help page first, then the person asks their question.
    public static func helpPrompt(url: String, app: String) -> String {
        "Run curl -s \(url) and read it. It describes \(app) on this Mac: what it does, its routes, its status and its commands. Then help me with \(app). My question: "
    }
}
