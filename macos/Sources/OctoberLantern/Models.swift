import Foundation

// Mirrors protocol/README.md.

/// The harness an agent runs on. Kept open-ended so a newer engine can report harnesses this app
/// doesn't know yet.
struct AgentKind: RawRepresentable, Codable, Hashable {
    let rawValue: String

    init(rawValue: String) { self.rawValue = rawValue }
    init(from decoder: Decoder) throws { rawValue = try decoder.singleValueContainer().decode(String.self) }
    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(rawValue)
    }

    var displayName: String {
        [
            "claude": "Claude Code", "codex": "Codex", "opencode": "OpenCode", "pi": "Pi", "gemini": "Gemini CLI",
            "grok": "Grok", "cursor": "Cursor", "qwen": "Qwen Code", "goose": "Goose", "aider": "Aider", "amp": "Amp",
            "copilot": "Copilot", "kimi": "Kimi", "droid": "Droid", "crush": "Crush", "auggie": "Auggie",
        ][rawValue] ?? rawValue.capitalized
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
    case installed(Installed)
    case launchResult(requestId: String, ok: Bool, message: String?)
    case attachResult(requestId: String, ok: Bool, message: String?)
    case other

    struct Installed: Decodable {
        let kinds: [AgentKind]
        let tmux: Bool
    }

    private enum Keys: String, CodingKey { case type, version, agents, requestId, ok, error, message, installed }

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
        case "installed":
            self = .installed(try c.decode(Installed.self, forKey: .installed))
        case "launchResult", "attachResult":
            let id = try c.decode(String.self, forKey: .requestId)
            let ok = try c.decode(Bool.self, forKey: .ok)
            let message = try c.decodeIfPresent(String.self, forKey: .message)
            self = try c.decode(String.self, forKey: .type) == "launchResult"
                ? .launchResult(requestId: id, ok: ok, message: message)
                : .attachResult(requestId: id, ok: ok, message: message)
        default:
            self = .other
        }
    }
}
