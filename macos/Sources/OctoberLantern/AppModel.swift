import AppKit
import SwiftUI

enum PanelMode: Equatable {
    case inbox, agents, newSession, october

    /// The inbox and agent list share tabs and the composer; the others are standalone.
    var isList: Bool { self == .inbox || self == .agents }
}

@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var agents: [Agent] = []
    /// Changing panels (or closing it) also leaves any open conversation.
    @Published var panel: PanelMode? {
        didSet { if oldValue != panel { closeChat() } }
    }
    @Published var pillExpanded = false
    @Published var targetId: String?
    @Published var draft = ""
    @Published var toast: String?
    @Published var composeFocusToken = 0
    @Published private(set) var dismissed: Set<String>
    @Published private(set) var installedKinds: [AgentKind] = []
    @Published private(set) var tmuxAvailable = false
    @Published private(set) var launching = false
    /// The agent whose conversation is open in the panel, if any.
    @Published private(set) var chatAgentId: String?
    @Published private(set) var chatMessages: [ChatMessage] = []
    @Published private(set) var chatSupported = true
    @Published private(set) var chatLoading = false
    let dictation = Dictation()

    private let engine = EngineClient()
    private var pending: [String: String] = [:]  // requestId → agentId
    private var nextRequest = 0
    private var toastTask: Task<Void, Never>?
    private let launchedAt = Date()

    init() {
        dismissed = Set(UserDefaults.standard.stringArray(forKey: "dismissedTurns") ?? [])
        engine.onAgents = { [weak self] agents in self?.update(agents) }
        engine.onReplyResult = { [weak self] id, ok, message in self?.replyFinished(id, ok: ok, message: message) }
        engine.onHistory = { [weak self] agentId, supported, messages in
            guard let self, agentId == self.chatAgentId else { return }
            self.chatLoading = false
            self.chatSupported = supported
            if messages != self.chatMessages { self.chatMessages = messages }
        }
        engine.onInstalled = { [weak self] installed in
            self?.installedKinds = installed.kinds
            self?.tmuxAvailable = installed.tmux
        }
        engine.onActionResult = { [weak self] ok, message in
            guard let self else { return }
            if self.launching {
                self.launching = false
                if ok { self.panel = .agents }
                self.show(ok ? "Session started" : "Couldn't start the session: \(message ?? "unknown error")")
            } else if !ok {
                self.show(message ?? "Something went wrong")
            }
        }
        dictation.onText = { [weak self] text in self?.draft = text }
        dictation.onError = { [weak self] message in self?.show(message) }
    }

    func start() { engine.start() }
    func stop() { engine.stop() }

    // MARK: Derived state

    /// Agents waiting on you, newest first. A turn that ended before Lantern launched and more
    /// than 30 minutes ago counts as already seen, so a fresh launch isn't a wall of badges.
    var inbox: [Agent] {
        agents
            .filter { $0.state.wantsYou && !dismissed.contains($0.turnKey) }
            .sorted { ($0.stateSince ?? 0) > ($1.stateSince ?? 0) }
    }

    var badgeCount: Int {
        let staleBefore = launchedAt.addingTimeInterval(-30 * 60)
        return inbox.filter { $0.state == .needsInput || ($0.since ?? .distantPast) > staleBefore }.count
    }

    /// Agents in the order the pill shows them: needs you, then your turn, then working, then the rest.
    var ranked: [Agent] {
        func rank(_ s: AgentState) -> Int {
            switch s {
            case .needsInput: 0
            case .waiting: 1
            case .working: 2
            case .idle, .unknown: 3
            }
        }
        return agents.sorted {
            rank($0.state) != rank($1.state) ? rank($0.state) < rank($1.state) : ($0.stateSince ?? 0) > ($1.stateSince ?? 0)
        }
    }

    var anyWorking: Bool { agents.contains { $0.state == .working } }

    var target: Agent? {
        if let targetId, let a = agents.first(where: { $0.id == targetId }) { return a }
        return inbox.first ?? agents.first
    }

    // MARK: Actions

    func toggle(_ mode: PanelMode) {
        panel = panel == mode ? nil : mode
    }

    func compose(to agent: Agent?) {
        if let agent { targetId = agent.id }
        if !(panel?.isList ?? false) { panel = .inbox }
        composeFocusToken += 1
    }

    var chatAgent: Agent? { chatAgentId.flatMap { id in agents.first { $0.id == id } } }

    /// Opens the conversation with `agent` in the panel; the composer then talks to it.
    func openChat(_ agent: Agent) {
        if chatAgentId != agent.id {
            chatMessages = []
            chatLoading = true
        }
        if !(panel?.isList ?? false) { panel = .inbox }
        chatAgentId = agent.id
        targetId = agent.id
        engine.history(agentId: agent.id)
        composeFocusToken += 1
    }

    func closeChat() {
        chatAgentId = nil
        chatMessages = []
    }

    func dismiss(_ agent: Agent) {
        dismissed.insert(agent.turnKey)
        // Keep only keys for agents that still exist.
        let live = Set(agents.map(\.turnKey))
        dismissed = dismissed.filter { live.contains($0) }
        UserDefaults.standard.set(Array(dismissed), forKey: "dismissedTurns")
    }

    func open(_ agent: Agent) {
        // A tmux session nobody is attached to (e.g. one Lantern started in the background).
        if agent.host == nil, agent.tmux != nil {
            nextRequest += 1
            engine.attach(requestId: "a\(nextRequest)", agentId: agent.id)
            return
        }
        guard let host = agent.host, let app = NSRunningApplication(processIdentifier: host.pid) else {
            show("Can't find the app @\(agent.handle) is running in")
            return
        }
        app.activate()
    }

    func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, let agent = target else { return }
        if dictation.isRecording { dictation.stop() }
        if agent.canReply {
            nextRequest += 1
            let id = "r\(nextRequest)"
            pending[id] = agent.id
            engine.reply(requestId: id, agentId: agent.id, text: text)
        } else {
            // v1 can only type into tmux panes. Anywhere else: copy, and bring the agent's app forward.
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            open(agent)
            show("Copied. Paste into @\(agent.handle) in \(agent.host?.app ?? "its terminal") with ⌘V")
            draft = ""
        }
    }

    // MARK: New sessions

    /// Folders to offer for a new session: ones you've used before, then where agents are running.
    var recentFolders: [String] {
        let used = UserDefaults.standard.stringArray(forKey: "recentFolders") ?? []
        let running = ranked.compactMap(\.cwd)
        var seen = Set<String>()
        return (used + running).filter { seen.insert($0).inserted && $0 != NSHomeDirectory() }.prefix(8).map { $0 }
    }

    func launch(kind: AgentKind, folder: String, prompt: String, background: Bool) {
        var used = UserDefaults.standard.stringArray(forKey: "recentFolders") ?? []
        used.removeAll { $0 == folder }
        used.insert(folder, at: 0)
        UserDefaults.standard.set(Array(used.prefix(8)), forKey: "recentFolders")
        launching = true
        nextRequest += 1
        engine.launch(requestId: "l\(nextRequest)", kind: kind, cwd: folder, prompt: prompt, background: background)
    }

    func toggleDictation() {
        if dictation.isRecording {
            dictation.stop()
        } else {
            compose(to: nil)
            dictation.start(prefix: draft)
        }
    }

    func show(_ message: String) {
        toast = message
        toastTask?.cancel()
        toastTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(4))
            if !Task.isCancelled { self?.toast = nil }
        }
    }

    // MARK: Engine events

    private func update(_ fresh: [Agent]) {
        agents = fresh
        if let targetId, !fresh.contains(where: { $0.id == targetId }) { self.targetId = nil }
        // Keep an open conversation current.
        if let chatAgentId {
            if fresh.contains(where: { $0.id == chatAgentId }) { engine.history(agentId: chatAgentId) } else { closeChat() }
        }
    }

    private func replyFinished(_ requestId: String, ok: Bool, message: String?) {
        let agentId = pending.removeValue(forKey: requestId)
        let handle = agents.first { $0.id == agentId }?.handle ?? "agent"
        if ok {
            draft = ""
            show("Sent to @\(handle)")
            if let chatAgentId { engine.history(agentId: chatAgentId) }
        } else {
            show("Couldn't send to @\(handle): \(message ?? "unknown error")")
        }
    }
}
