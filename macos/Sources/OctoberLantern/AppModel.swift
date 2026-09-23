import AppKit
import SwiftUI

enum PanelMode: Equatable {
    case inbox, agents
}

@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var agents: [Agent] = []
    @Published var panel: PanelMode?
    @Published var targetId: String?
    @Published var draft = ""
    @Published var toast: String?
    @Published var composeFocusToken = 0
    @Published private(set) var dismissed: Set<String>
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
        if panel == nil { panel = .inbox }
        composeFocusToken += 1
    }

    func dismiss(_ agent: Agent) {
        dismissed.insert(agent.turnKey)
        // Keep only keys for agents that still exist.
        let live = Set(agents.map(\.turnKey))
        dismissed = dismissed.filter { live.contains($0) }
        UserDefaults.standard.set(Array(dismissed), forKey: "dismissedTurns")
    }

    func open(_ agent: Agent) {
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
    }

    private func replyFinished(_ requestId: String, ok: Bool, message: String?) {
        let agentId = pending.removeValue(forKey: requestId)
        let handle = agents.first { $0.id == agentId }?.handle ?? "agent"
        if ok {
            draft = ""
            show("Sent to @\(handle)")
        } else {
            show("Couldn't send to @\(handle): \(message ?? "unknown error")")
        }
    }
}
