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

    static let october = AgentKind(rawValue: "october")
    static let claude = AgentKind(rawValue: "claude")
    static let codex = AgentKind(rawValue: "codex")

    /// Every agent Lantern supports, in the order New Session lists them.
    static let supported: [AgentKind] = [
        "claude", "codex", "gemini", "opencode", "pi", "october", "grok", "cursor", "copilot", "qwen", "goose", "aider", "amp",
        "kimi", "droid", "crush", "auggie", "muse",
    ].map { AgentKind(rawValue: $0) }

    /// Where to get an agent that isn't installed.
    var website: URL? {
        [
            "claude": "https://code.claude.com/docs", "codex": "https://github.com/openai/codex",
            "gemini": "https://github.com/google-gemini/gemini-cli", "opencode": "https://opencode.ai",
            "pi": "https://github.com/earendil-works/pi", "october": "https://github.com/october-dev/october-harness",
            "grok": "https://x.ai", "cursor": "https://cursor.com/cli", "copilot": "https://github.com/github/copilot-cli",
            "qwen": "https://github.com/QwenLM/qwen-code", "goose": "https://github.com/block/goose", "aider": "https://aider.chat",
            "amp": "https://ampcode.com", "kimi": "https://github.com/MoonshotAI/kimi-cli", "droid": "https://factory.ai",
            "crush": "https://github.com/charmbracelet/crush", "auggie": "https://www.augmentcode.com",
        ][rawValue].flatMap { URL(string: $0) }
    }

    var displayName: String {
        [
            "claude": "Claude Code", "codex": "Codex", "opencode": "OpenCode", "pi": "Pi", "gemini": "Gemini CLI",
            "grok": "Grok", "cursor": "Cursor", "qwen": "Qwen Code", "goose": "Goose", "aider": "Aider", "amp": "Amp",
            "copilot": "Copilot", "kimi": "Kimi", "droid": "Droid", "crush": "Crush", "auggie": "Auggie",
            "muse": "Muse", "october": "October",
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

/// How Lantern types into an agent: "tmux", "cmux", "terminal", "iterm", "october" or "none".
struct Route: Codable, Hashable {
    let via: String

    /// Whether single keys (e.g. "1" to allow a permission prompt) can be sent without Enter.
    var sendsKeys: Bool { via == "tmux" || via == "cmux" || via == "iterm" }
}

/// What an agent's question offers (engine `Choices`).
struct Choices: Codable, Hashable {
    struct Question: Codable, Hashable {
        let question: String
        let header: String?
        let multiSelect: Bool
        let options: [Option]
    }
    struct Option: Codable, Hashable {
        let label: String
        let description: String?
    }
    var questions: [Question] = []
    /// A permission prompt's "don't ask again" option, as the terminal words it.
    var always: String?

    enum CodingKeys: String, CodingKey { case questions, always }
    init(questions: [Question] = [], always: String? = nil) {
        self.questions = questions
        self.always = always
    }
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        questions = try c.decodeIfPresent([Question].self, forKey: .questions) ?? []
        always = try c.decodeIfPresent(String.self, forKey: .always)
    }
}

struct Agent: Codable, Identifiable, Hashable {
    /// `<kind>:<pid>:<start time>`; a reused pid is a different agent.
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
    /// "permission" when a hook says the agent is at a tool permission prompt (`1` allows once,
    /// Escape declines); "choice" for Claude's own multiple-choice question; "plan" for its plan
    /// waiting for approval; "other" for any other question.
    let questionKind: String?
    /// The whole request behind `question` (a permission prompt's full command).
    let questionDetail: String?
    /// Identifies one permission prompt; Allow/Deny send it so they can't answer a different one.
    let promptId: String?
    /// The options Lantern offers for the question (Claude's own questions; a permission's "always").
    let choices: Choices?
    /// Its October Bus id, for an agent Lantern started on the Bus (Ask Lantern's messages go there).
    let busId: String?
    /// How the conversation was matched to this process: "exact", "guessed", "ambiguous" (another
    /// agent of this kind in this folder; state and history hidden) or "none".
    let sessionMatch: String?
    /// The app a session belongs to when it isn't a terminal: "Claude Desktop", "Cowork", "Codex app".
    let source: String?
    /// false for an app session that isn't running any more (listed for three days).
    let live: Bool?
    let host: HostApp?
    let tmux: TmuxPane?
    let canReply: Bool
    let route: Route?
    let stateSource: String

    /// A question Lantern can answer from its card (by pressing the terminal's keys).
    var isAnswerable: Bool {
        guard state == .needsInput, promptId != nil, route?.sendsKeys ?? false else { return false }
        switch questionKind {
        case "permission", "plan": return true
        case "choice": return !(choices?.questions ?? []).isEmpty
        default: return false
        }
    }

    var isLive: Bool { live ?? true }

    /// Lantern can't tell which conversation belongs to this process.
    var sessionAmbiguous: Bool { sessionMatch == "ambiguous" }

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
        if let source { return source }
        if let tmux { return "tmux \(tmux.target)" }
        return host?.app
    }
}

struct ChatMessage: Codable, Hashable {
    enum Role: String, Codable { case user, agent, tool }
    let role: Role
    let text: String
    let at: String?
}

/// The connection to October Desktop (see engine/src/october_link.rs).
struct OctoberLink: Decodable, Equatable {
    /// notInstalled | notRunning | readOnly | connected | pairing | error
    let status: String
    let coreVersion: String?
    let paired: Bool
    let pairingCode: String?
    let message: String?
    let agentCount: Int
}

enum EngineMessage: Decodable {
    case hello(protocolVersion: Int, version: String)
    case snapshot(agents: [Agent])
    case replyResult(requestId: String, ok: Bool, error: String?, message: String?)
    /// `harness`: October Harness's status, from engines that have it.
    case installed(Installed, harness: HarnessStatus?)
    case harness(HarnessStatus)
    case computer(ComputerStatus)
    case models(ModelList)
    /// `session`: the tmux session a started agent runs in (absent without tmux).
    case launchResult(requestId: String, ok: Bool, message: String?, session: String?)
    case attachResult(requestId: String, ok: Bool, message: String?)
    case history(requestId: String, agentId: String, supported: Bool, messages: [ChatMessage], busy: Bool)
    case october(OctoberLink)
    case phone(PhoneModel.State)
    /// Every session's cards (engine `cards`), newest first.
    case computerHold(ControlSnapshot)
    case controlResult(RoutineReceipt)
    case routines(RoutineSnapshot)
    case routineResult(RoutineReceipt)
    case cards([CardLaunch])
    case cardResult(requestId: String, cardId: String, ok: Bool, message: String?)
    /// Show me (engine/src/show): take a picture, ask October, or where the guide is.
    case show(ShowMessage)
    case other

    /// The models an agent can start with (see engine/src/models.rs). `choosable` is false for
    /// agents whose model option Lantern doesn't know.
    struct ModelList: Decodable, Equatable {
        let kind: AgentKind
        let choosable: Bool
        let models: [ModelOption]
    }

    struct ModelOption: Decodable, Hashable {
        let id: String
        let label: String
        let group: String?
    }

    struct Installed: Decodable {
        let kinds: [AgentKind]
        let tmux: Bool
        /// Installed agents signed in to the user's own plan, best first (engine `subscriptions`).
        let signedIn: [AgentKind]?
        /// Installed Claude Code or Codex that has never been signed in.
        let neverSetUp: [AgentKind]?
    }

    /// Cua Driver, which lets agents use this Mac's apps (engine/src/computer.rs): `ready`,
    /// `installing`, `failed` (with `message`) or `off`.
    struct ComputerStatus: Decodable, Equatable {
        let status: String
        let message: String?
        let version: String?
    }

    /// October Harness, built in (see engine/src/harness.rs): `ready` (on PATH or installed by
    /// Lantern), `installing`, `failed` (with `message`) or `missing` (installed on first use).
    struct HarnessStatus: Decodable, Equatable {
        let status: String
        /// "path" (the user's own `october`) or "bundled" (Lantern's), when ready.
        let source: String?
        let version: String?
        let message: String?

        var isReady: Bool { status == "ready" }
    }

    private enum Keys: String, CodingKey {
        case type, `protocol`, version, agents, requestId, ok, error, message, installed, agentId, supported, messages, october, session, warning, busy,
            harness, launches, cardId
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        switch try c.decode(String.self, forKey: .type) {
        case "hello":
            self = .hello(
                protocolVersion: try c.decodeIfPresent(Int.self, forKey: .protocol) ?? 0,
                version: try c.decodeIfPresent(String.self, forKey: .version) ?? "?"
            )
        case "snapshot":
            self = .snapshot(agents: try c.decode([Agent].self, forKey: .agents))
        case "replyResult", "sendResult":
            self = .replyResult(
                requestId: try c.decode(String.self, forKey: .requestId),
                ok: try c.decode(Bool.self, forKey: .ok),
                error: try c.decodeIfPresent(String.self, forKey: .error),
                message: try c.decodeIfPresent(String.self, forKey: .message)
            )
        case "october":
            self = .october(try c.decode(OctoberLink.self, forKey: .october))
        case "phone":
            self = .phone(try PhoneModel.State(from: decoder))
        case "historyResult":
            self = .history(
                requestId: try c.decode(String.self, forKey: .requestId),
                agentId: try c.decode(String.self, forKey: .agentId),
                supported: try c.decode(Bool.self, forKey: .supported),
                messages: try c.decode([ChatMessage].self, forKey: .messages),
                busy: try c.decodeIfPresent(Bool.self, forKey: .busy) ?? false
            )
        case "models":
            self = .models(try ModelList(from: decoder))
        case "installed":
            self = .installed(
                try c.decode(Installed.self, forKey: .installed), harness: try c.decodeIfPresent(HarnessStatus.self, forKey: .harness)
            )
        case "harness":
            self = .harness(try HarnessStatus(from: decoder))
        case "computerHold": self = .computerHold(try ControlSnapshot(from: decoder))
        case "controlResult": self = .controlResult(try RoutineReceipt(from: decoder))
        case "routines": self = .routines(try RoutineSnapshot(from: decoder))
        case "routineResult": self = .routineResult(try RoutineReceipt(from: decoder))
        case "cards":
            self = .cards(try c.decode([CardLaunch].self, forKey: .launches))
        case "cardResult":
            self = .cardResult(
                requestId: try c.decode(String.self, forKey: .requestId), cardId: try c.decode(String.self, forKey: .cardId),
                ok: try c.decode(Bool.self, forKey: .ok), message: try c.decodeIfPresent(String.self, forKey: .message)
            )
        case "computer":
            self = .computer(try ComputerStatus(from: decoder))
        case "showCapture", "showQuery", "showState":
            self = .show(try ShowMessage(from: decoder))
        case "launchResult", "attachResult":
            let id = try c.decode(String.self, forKey: .requestId)
            let ok = try c.decode(Bool.self, forKey: .ok)
            // A launch that worked can still carry a warning (e.g. it started without October Bus).
            let message = try c.decodeIfPresent(String.self, forKey: .message) ?? c.decodeIfPresent(String.self, forKey: .warning)
            self = try c.decode(String.self, forKey: .type) == "launchResult"
                ? .launchResult(requestId: id, ok: ok, message: message, session: try c.decodeIfPresent(String.self, forKey: .session))
                : .attachResult(requestId: id, ok: ok, message: message)
        default:
            self = .other
        }
    }
}

/// What the person chose for an agent's question (engine `Answer`).
enum AgentAnswer: Hashable {
    /// Allow once; start on the plan.
    case yes
    /// The permission prompt's "don't ask again" option.
    case always
    /// Deny; keep planning; don't answer Claude's question.
    case no
    /// The options picked for each of Claude's questions, in order (0-based).
    case picks([[Int]])

    var kind: String {
        switch self {
        case .yes: "yes"
        case .always: "always"
        case .no: "no"
        case .picks: "picks"
        }
    }

    var json: [String: Any] {
        if case .picks(let picks) = self { return ["kind": kind, "picks": picks] }
        return ["kind": kind]
    }
}

/// What the engine says about a Show me guide (protocol/README.md, "Show me").
struct ShowMessage: Decodable {
    enum Kind: String, Decodable { case showCapture, showQuery, showState }
    let type: Kind
    let runId: String
    let step: Int
    /// showQuery: the round, its capture, the guide's goal (after the first round), the controls to
    /// send (`[id, kind, label, [x, y, w, h]]`, image pixels) and the steps already shown.
    let round: Int?
    let captureId: String?
    let issuedAt: Double?
    let expiresAt: Double?
    let goalId: String?
    let elements: [ShowElement]?
    let shown: [String]?
    /// showState: looking, asking, showing, notVisible, finished, changed, failed or ended.
    let phase: String?
    /// showing: the target in the picture's pixels, and whether Lantern's cursor points at it.
    let target: Target?
    let live: Bool?
    let reason: String?
    let message: String?

    struct Target: Decodable { let image: [Double] }
}

/// One control in a showQuery: `[id, kind, label, [x, y, w, h]]`, kept as the JSON October takes.
struct ShowElement: Decodable {
    let json: [Any]

    init(from decoder: Decoder) throws {
        var c = try decoder.unkeyedContainer()
        let id = try c.decode(Int.self)
        let kind = try c.decode(String.self)
        let label = try c.decodeIfPresent(String.self)
        let box = try c.decode([Double].self)
        json = [id, kind, label as Any? ?? NSNull(), box]
    }
}

struct ControlSnapshot: Decodable { let held: Bool; let message: String? }
