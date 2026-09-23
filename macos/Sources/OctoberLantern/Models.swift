import Foundation

// Mirrors protocol/README.md.

enum AgentKind: String, Codable {
    case claude, codex, opencode, pi

    var displayName: String {
        switch self {
        case .claude: "Claude Code"
        case .codex: "Codex"
        case .opencode: "OpenCode"
        case .pi: "Pi"
        }
    }
}

enum AgentState: String, Codable {
    case working, waiting, idle, unknown
    case needsInput = "needs_input"

    var label: String {
        switch self {
        case .working: "Working"
        case .waiting: "Your turn"
        case .needsInput: "Needs you"
        case .idle: "Idle"
        case .unknown: "Running"
        }
    }

    /// Waiting on the person, so it belongs in the inbox.
    var wantsYou: Bool { self == .waiting || self == .needsInput }
}

struct HostApp: Codable, Hashable {
    let app: String
    let pid: Int32
    let bundlePath: String
}

struct TmuxPane: Codable, Hashable {
    let socket: String?
    let target: String
    let paneId: String
}

struct Agent: Codable, Identifiable, Hashable {
    let id: String
    let kind: AgentKind
    let handle: String
    let pid: Int32
    let tty: String?
    let cwd: String?
    let project: String?
    let title: String?
    let sessionId: String?
    let state: AgentState
    let stateSince: Double?
    let lastMessage: String?
    let question: String?
    let host: HostApp?
    let tmux: TmuxPane?
    let canReply: Bool
    let stateSource: String

    /// Identifies one particular turn, so dismissing it doesn't hide the next one.
    var turnKey: String { "\(id)@\(Int(stateSince ?? 0))" }

    var since: Date? { stateSince.map { Date(timeIntervalSince1970: $0 / 1000) } }

    var shortCwd: String? {
        guard let cwd else { return nil }
        let home = NSHomeDirectory()
        return cwd.hasPrefix(home) ? "~" + cwd.dropFirst(home.count) : cwd
    }

    /// Where the agent lives, for display: "cmux", "tmux t1:0.0", ...
    var location: String? {
        if let tmux { return "tmux \(tmux.target)" }
        return host?.app
    }
}

enum EngineMessage: Decodable {
    case hello(version: String)
    case snapshot(agents: [Agent])
    case replyResult(requestId: String, ok: Bool, error: String?, message: String?)
    case other

    private enum Keys: String, CodingKey { case type, version, agents, requestId, ok, error, message }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        switch try c.decode(String.self, forKey: .type) {
        case "hello":
            self = .hello(version: try c.decodeIfPresent(String.self, forKey: .version) ?? "?")
        case "snapshot":
            self = .snapshot(agents: try c.decode([Agent].self, forKey: .agents))
        case "replyResult":
            self = .replyResult(
                requestId: try c.decode(String.self, forKey: .requestId),
                ok: try c.decode(Bool.self, forKey: .ok),
                error: try c.decodeIfPresent(String.self, forKey: .error),
                message: try c.decodeIfPresent(String.self, forKey: .message)
            )
        default:
            self = .other
        }
    }
}
