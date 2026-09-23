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
        didSet {
            guard oldValue != panel else { return }
            closeChat()
            // Leaving Waiting means you've seen what was in it.
            if oldValue == .inbox { markInboxSeen() }
        }
    }
    @Published var pillExpanded = false
    @Published var targetId: String?
    @Published var draft = ""
    @Published var toast: String?
    @Published var composeFocusToken = 0
    @Published private(set) var dismissed: Set<String>
    /// Turns you've already seen in Waiting. They stay listed (under Earlier) but don't count.
    @Published private(set) var seen: Set<String>
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
    private var firstSnapshot = true
    private var knownTurns: Set<String> = []
    private let prefs = Preferences.shared
    var onNotify: ((Agent) -> Void)?

    init() {
        dismissed = Set(UserDefaults.standard.stringArray(forKey: "dismissedTurns") ?? [])
        seen = Set(UserDefaults.standard.stringArray(forKey: "seenTurns") ?? [])
        engine.onAgents = { [weak self] agents in self?.update(agents) }
        engine.onReplyResult = { [weak self] id, ok, message in self?.replyFinished(id, ok: ok, message: message) }
        engine.onHistory = { [weak self] agentId, supported, messages in
            guard let self, agentId == self.chatAgentId else { return }
            self.chatLoading = false
            self.chatSupported = supported
            if messages != self.chatMessages { self.chatMessages = messages }
        }
        // Phone: the engine hosts the October phone app connection (see PhoneCard.swift).
        engine.onPhone = { data in PhoneModel.shared.receive(data) }
        PhoneModel.shared.send = { [weak engine] obj in engine?.phone(obj) }
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

    /// Agents waiting on you (in harnesses you haven't muted), newest first.
    var inbox: [Agent] {
        agents
            .filter { $0.state.wantsYou && !dismissed.contains($0.turnKey) && prefs.counts($0.kind) }
            .sorted { ($0.stateSince ?? 0) > ($1.stateSince ?? 0) }
    }

    /// Waiting turns you haven't looked at yet. A question for you always counts.
    var newInbox: [Agent] { inbox.filter { $0.state == .needsInput || !seen.contains($0.turnKey) } }
    var earlierInbox: [Agent] { inbox.filter { $0.state != .needsInput && seen.contains($0.turnKey) } }

    var badgeCount: Int { newInbox.count }

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

    func dismiss(_ agent: Agent) { dismiss([agent]) }

    /// "Done": removes turns from Waiting until the agent's state changes again.
    func dismiss(_ list: [Agent]) {
        dismissed.formUnion(list.map(\.turnKey))
        // Keep only keys for turns that still exist.
        let live = Set(agents.map(\.turnKey))
        dismissed = dismissed.filter { live.contains($0) }
        UserDefaults.standard.set(Array(dismissed), forKey: "dismissedTurns")
    }

    /// Marks everything currently in Waiting as seen (called while the Waiting list is on screen).
    func markInboxSeen() {
        let keys = Set(inbox.map(\.turnKey))
        guard !keys.isSubset(of: seen) else { return }
        let live = Set(agents.map(\.turnKey))
        seen = seen.union(keys).filter { live.contains($0) }
        UserDefaults.standard.set(Array(seen), forKey: "seenTurns")
    }

    /// Brings the agent's own tab to the front (or opens a Terminal for a background session).
    func open(_ agent: Agent) {
        nextRequest += 1
        engine.focus(requestId: "f\(nextRequest)", agentId: agent.id)
        if let host = agent.host, let app = NSRunningApplication(processIdentifier: host.pid) {
            app.activate()
        }
    }

    /// Answers a permission prompt: "1" allows once, Escape declines.
    func answerPermission(_ agent: Agent, allow: Bool) {
        nextRequest += 1
        let id = "k\(nextRequest)"
        pending[id] = agent.id
        engine.keys(requestId: id, agentId: agent.id, keys: [allow ? "1" : "Escape"])
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
            // Terminals Lantern can't type into (Ghostty, VS Code, Warp...): copy, and bring it forward.
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
        noticeNewTurns()
        if let targetId, !fresh.contains(where: { $0.id == targetId }) { self.targetId = nil }
        // Keep an open conversation current.
        if let chatAgentId {
            if fresh.contains(where: { $0.id == chatAgentId }) { engine.history(agentId: chatAgentId) } else { closeChat() }
        }
    }

    /// Works out which turns are new since the last snapshot, for notifications. On the very first
    /// snapshot after launch, anything that's been waiting over two hours counts as already seen, so
    /// opening Lantern isn't a wall of old sessions.
    private func noticeNewTurns() {
        let waiting = agents.filter { $0.state.wantsYou && prefs.counts($0.kind) }
        if firstSnapshot {
            firstSnapshot = false
            let stale = Date().addingTimeInterval(-2 * 3600)
            let old = waiting.filter { ($0.since ?? .distantPast) < stale && $0.state != .needsInput }
            if !old.isEmpty {
                seen.formUnion(old.map(\.turnKey))
                UserDefaults.standard.set(Array(seen), forKey: "seenTurns")
            }
            knownTurns = Set(waiting.map(\.turnKey))
            return
        }
        for agent in waiting where !knownTurns.contains(agent.turnKey) && !seen.contains(agent.turnKey) {
            onNotify?(agent)
        }
        knownTurns = Set(waiting.map(\.turnKey))
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
