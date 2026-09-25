import AppKit
import LanternCore
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
    /// Drafts, one per conversation, and the recipient they're addressed to (see `Drafts`).
    @Published private(set) var drafts = Drafts()
    /// The agent the composer is addressed to. Once chosen (or once you start typing), it stays
    /// chosen: a draft never quietly changes recipient because the agent list changed. Setting it
    /// switches conversation; each agent keeps its own draft.
    var targetId: String? {
        get { drafts.recipient }
        set {
            guard newValue != drafts.recipient else { return }
            stopDictationIfAway(from: newValue)
            drafts.address(newValue)
        }
    }
    /// The draft for the current recipient.
    var draft: String {
        get { drafts.text }
        set { drafts.type(newValue, fallback: target?.id) }
    }
    @Published var toast: String?
    @Published var composeFocusToken = 0
    @Published private(set) var dismissed: Set<String>
    /// Turns you've already seen in Waiting. They stay listed (under Earlier) but don't count.
    @Published private(set) var seen: Set<String>
    /// Installed agents; `nil` until the engine has checked.
    @Published private(set) var installedKinds: [AgentKind]?
    @Published private(set) var tmuxAvailable = false
    @Published private(set) var launching = false
    /// Models each agent can start with, by kind, once the engine has listed them.
    @Published private(set) var models: [String: EngineMessage.ModelList] = [:]
    /// Why the last start failed, shown in the New Session form.
    @Published private(set) var launchError: String?
    private var launchingKind = ""
    private var launchingInBackground = false
    private var launchingWithScreenshot = false
    /// The app a Task was started for (its bundle id), for usage counts.
    private var launchingTask = "none"
    private var launchingModel = "default"
    /// A session Lantern just started: its conversation opens as soon as the agent shows up.
    private var openWhenSeen: (session: String, until: Date)?
    private var chatPoll: Task<Void, Never>?
    /// The agent whose conversation is open in the panel, if any.
    @Published private(set) var chatAgentId: String?
    @Published private(set) var chatMessages: [ChatMessage] = []
    @Published private(set) var chatSupported = true
    @Published private(set) var chatLoading = false
    @Published private(set) var octoberLink: OctoberLink?
    /// Requests the engine hasn't answered yet, by request id.
    @Published private(set) var pending: [String: Pending] = [:]
    @Published private(set) var engineHealth: EngineHealth = .starting
    let dictation = Dictation()

    enum Pending {
        /// The ticket says which draft revision was sent, so only that is cleared when it arrives.
        case reply(Drafts.Ticket)
        case keys(agentId: String)
        /// A message sent without the composer (Point & Ask).
        case direct(agentId: String)
        case launch
        case focus(agentId: String)
    }

    private let engine = EngineClient()
    private var nextRequest = 0
    private var toastTask: Task<Void, Never>?
    private var firstSnapshot = true
    private var knownTurns: Set<String> = []
    /// When each pending request was sent; unanswered after `requestDeadline`, it fails.
    private var pendingSince: [String: Date] = [:]
    private var deadlineTask: Task<Void, Never>?
    /// Longer than the engine can take to answer a reply (15 s to start typing, then time-limited
    /// helpers), so a timeout here means the engine is stuck, not merely slow.
    static let requestDeadline: TimeInterval = 60
    /// Replies and keys the engine didn't answer in time. It may still have typed them, so a late
    /// answer is still applied (and a late "sent" clears the draft).
    private var overdue: [String: Pending] = [:]
    /// Starting a session may wait for the agent to come up before typing its first message.
    static let launchDeadline: TimeInterval = 45
    private let prefs = Preferences.shared
    var onNotify: ((Agent) -> Void)?

    init() {
        dismissed = Set(UserDefaults.standard.stringArray(forKey: "dismissedTurns") ?? [])
        seen = Set(UserDefaults.standard.stringArray(forKey: "seenTurns") ?? [])
        engine.onAgents = { [weak self] agents in self?.update(agents) }
        engine.onReplyResult = { [weak self] id, ok, error, message in
            self?.replyFinished(id, ok: ok, uncertain: error == "uncertain", message: message)
        }
        engine.onActionResult = { [weak self] id, ok, message in self?.actionFinished(id, ok: ok, message: message) }
        engine.onLaunched = { [weak self] session in
            self?.openWhenSeen = (session, Date().addingTimeInterval(30))
            self?.openLaunched()
        }
        engine.onHistory = { [weak self] agentId, supported, messages in
            guard let self, agentId == self.chatAgentId else { return }
            self.chatLoading = false
            self.chatSupported = supported
            if messages != self.chatMessages { self.chatMessages = messages }
        }
        engine.onOctober = { [weak self] link in
            if link.status == "connected", self?.octoberLink?.status != "connected" {
                Analytics.shared.capture("october_desktop_connected")
            }
            self?.octoberLink = link
        }
        engine.onPhone = { state in PhoneModel.shared.receive(state) }
        PhoneModel.shared.engine = engine
        engine.onModels = { [weak self] list in self?.models[list.kind.rawValue] = list }
        engine.onInstalled = { [weak self] installed in
            self?.installedKinds = installed.kinds
            self?.tmuxAvailable = installed.tmux
        }
        engine.onReady = { [weak self] in
            OctoberAccount.shared.engineReady()
            // Fetch October Bus a minute after start, so the first session on it doesn't wait.
            Task { @MainActor [weak self] in
                try? await Task.sleep(for: .seconds(60))
                if let self, self.prefs.busForSessions { self.engine.prepareBus() }
            }
        }
        engine.onStopped = { [weak self] message in self?.engineStopped(message) }
        engine.onHealth = { [weak self] health in
            if case .failed = health { Analytics.shared.capture("engine_failed") }
            self?.engineHealth = health
        }
        // Dictated text goes to the draft it was started in, whichever conversation is showing.
        dictation.onText = { [weak self] text, owner in self?.drafts.set(text, for: owner) }
        dictation.onError = { [weak self] message in self?.show(message) }
    }

    func start() { engine.start() }
    func stop() { engine.stop() }

    // MARK: Derived state

    /// Agents waiting on you (in harnesses you haven't muted), newest first.
    var inbox: [Agent] {
        agents
            .filter { $0.isLive && $0.state.wantsYou && !dismissed.contains($0.turnKey) && prefs.counts($0.kind) }
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
        // App sessions that aren't running come last.
        func order(_ a: Agent) -> Int { a.isLive ? rank(a.state) : 4 }
        return agents.sorted {
            order($0) != order($1) ? order($0) < order($1) : ($0.stateSince ?? 0) > ($1.stateSince ?? 0)
        }
    }

    var anyWorking: Bool { agents.contains { $0.state == .working } }

    /// The composer's recipient. A chosen agent that has gone away is `nil` (and `targetMissing`),
    /// never silently replaced by another agent.
    var target: Agent? {
        if let targetId { return agents.first { $0.id == targetId } }
        return inbox.first ?? agents.first
    }

    var targetMissing: Bool { targetId != nil && target == nil }

    /// A reply is on its way to the current target; Send waits for the answer.
    var sending: Bool {
        pending.values.contains { if case .reply(let ticket) = $0 { ticket.recipient == target?.id } else { false } }
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
        if chatAgentId != agent.id { Analytics.shared.capture("chat_opened", ["kind": agent.kind.rawValue]) }
        chatAgentId = agent.id
        targetId = agent.id  // the composer switches to this agent's own draft
        engine.history(agentId: agent.id)
        pollChat(agent.id)
        composeFocusToken += 1
    }

    func closeChat() {
        chatAgentId = nil
        chatMessages = []
        chatPoll?.cancel()
        chatPoll = nil
    }

    /// Opens the conversation of the session Lantern just started, once it appears.
    private func openLaunched() {
        guard let (session, until) = openWhenSeen else { return }
        if Date() > until { return openWhenSeen = nil }
        guard let agent = agents.first(where: { $0.tmux?.target.hasPrefix("\(session):") ?? false }) else { return }
        openWhenSeen = nil
        openChat(agent)
    }

    /// While a conversation is open it refreshes every two seconds, so a long turn shows its
    /// progress (snapshots only arrive when an agent's state changes).
    private func pollChat(_ agentId: String) {
        chatPoll?.cancel()
        chatPoll = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(2))
                guard let self, self.chatAgentId == agentId, !Task.isCancelled else { return }
                self.engine.history(agentId: agentId)
            }
        }
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
    /// The recipient menu: sends the draft being written to `agent` instead. The text moves with it
    /// unless `agent` already has a draft of its own.
    func readdress(to agent: Agent) {
        stopDictationIfAway(from: agent.id)
        drafts.readdress(agent.id)
    }

    func open(_ agent: Agent) {
        Analytics.shared.capture("agent_opened", ["kind": agent.kind.rawValue, "route": agent.route?.via ?? "none"])
        let id = request("f")
        track(id, .focus(agentId: agent.id), sent: engine.focus(requestId: id, agentId: agent.id))
        if let host = agent.host {
            // An app session that isn't running: open its app.
            if host.pid == 0 {
                NSWorkspace.shared.open(URL(fileURLWithPath: host.bundlePath))
            } else if let app = NSRunningApplication(processIdentifier: host.pid) {
                app.activate()
            }
        }
    }

    /// Sends `text` to `agent` without touching the composer's drafts (Point & Ask). Agents Lantern
    /// can't type into get it copied, and their app comes forward.
    func sendDirect(_ text: String, to agent: Agent) {
        if agent.canReply {
            let id = request("d")
            track(id, .direct(agentId: agent.id), sent: engine.reply(requestId: id, agentId: agent.id, text: text))
        } else {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            open(agent)
            show("Copied. Paste into @\(agent.handle) in \(agent.host?.app ?? "its terminal") with ⌘V")
        }
    }

    /// Answers a permission prompt: "1" allows once, Escape declines. The engine refuses if the
    /// prompt on screen is no longer the one shown here.
    func answerPermission(_ agent: Agent, allow: Bool) {
        guard let promptId = agent.promptId else { return }
        Analytics.shared.capture("permission_answered", ["allow": allow, "route": agent.route?.via ?? "none"])
        let id = request("k")
        track(
            id, .keys(agentId: agent.id),
            sent: engine.keys(requestId: id, agentId: agent.id, keys: [allow ? "1" : "Escape"], promptId: promptId)
        )
    }

    func send() {
        guard let agent = target, !sending else { return }
        // Lock the recipient (a fallback target becomes the chosen one) before taking the ticket.
        if drafts.recipient == nil { drafts.readdress(agent.id) }
        guard let ticket = drafts.ticket(), ticket.recipient == agent.id else { return }
        let text = ticket.text
        dictation.stop()
        if agent.canReply {
            let id = request("r")
            track(id, .reply(ticket), sent: engine.reply(requestId: id, agentId: agent.id, text: text))
        } else {
            // Terminals Lantern can't type into (Ghostty, VS Code, Warp...): copy, and bring it forward.
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            open(agent)
            show("Copied. Paste into @\(agent.handle) in \(agent.host?.app ?? "its terminal") with ⌘V")
            Analytics.shared.capture("reply_copied", ["kind": agent.kind.rawValue])
            drafts.sent(ticket)
        }
    }

    func october(_ action: String) { engine.october(action) }

    // MARK: New sessions

    /// Folders to offer for a new session: ones you've used before, then where agents are running.
    var recentFolders: [String] {
        let used = UserDefaults.standard.stringArray(forKey: "recentFolders") ?? []
        let running = ranked.compactMap(\.cwd)
        var seen = Set<String>()
        return (used + running).filter { seen.insert($0).inserted && $0 != NSHomeDirectory() }.prefix(8).map { $0 }
    }

    /// `screenshot`: a saved screenshot (ScreenCapture.save) the agent gets with its first message.
    /// Asks the engine which models `kind` offers (it answers from a cache after the first time).
    func loadModels(_ kind: AgentKind) { engine.models(kind: kind) }

    /// The model last chosen for each agent; nil is the agent's own default.
    func lastModel(_ kind: AgentKind) -> String? {
        (UserDefaults.standard.dictionary(forKey: "lastModels") as? [String: String])?[kind.rawValue]
    }

    /// Opens New Session, reading the app you're in first (before anything else can come to the
    /// front): in an app other than a terminal it opens as a task for that app.
    func startNew() {
        if panel == .newSession { return panel = nil }
        AppContext.shared.capture()
        panel = .newSession
    }

    func refreshToolkit() { engine.refreshToolkit() }

    /// The folder a Task for this app used last time.
    func taskFolder(for bundleId: String?) -> String? {
        guard let bundleId else { return nil }
        return (UserDefaults.standard.dictionary(forKey: "taskFolders") as? [String: String])?[bundleId]
    }

    func launch(
        kind: AgentKind, folder: String, prompt: String, screenshot: URL? = nil, model: String? = nil, context: String? = nil,
        toolkit: Bool = false, taskApp: String? = nil, background: Bool
    ) {
        if let taskApp {
            var folders = UserDefaults.standard.dictionary(forKey: "taskFolders") as? [String: String] ?? [:]
            folders[taskApp] = folder
            UserDefaults.standard.set(folders, forKey: "taskFolders")
        }
        var last = UserDefaults.standard.dictionary(forKey: "lastModels") as? [String: String] ?? [:]
        last[kind.rawValue] = model
        UserDefaults.standard.set(last, forKey: "lastModels")
        var used = UserDefaults.standard.stringArray(forKey: "recentFolders") ?? []
        used.removeAll { $0 == folder }
        used.insert(folder, at: 0)
        UserDefaults.standard.set(Array(used.prefix(8)), forKey: "recentFolders")
        launching = true
        launchError = nil
        launchingKind = kind.rawValue
        launchingInBackground = background
        launchingWithScreenshot = screenshot != nil
        launchingTask = taskApp ?? "none"
        launchingModel = model ?? "default"
        let id = request("l")
        track(
            id, .launch,
            sent: engine.launch(
                requestId: id, kind: kind, cwd: folder, prompt: prompt, screenshot: screenshot?.path, model: model, context: context,
                toolkit: toolkit, bus: prefs.busForSessions, background: background
            )
        )
    }

    func toggleDictation() {
        if dictation.isActive {
            dictation.stop()
        } else {
            compose(to: nil)
            if drafts.recipient == nil, let t = target { drafts.readdress(t.id) }
            dictation.start(prefix: draft, owner: drafts.recipient ?? "")
            Analytics.shared.capture("dictation_started")
        }
    }

    /// Dictation belongs to one conversation's draft; moving to another conversation ends it.
    private func stopDictationIfAway(from recipient: String?) {
        if let owner = dictation.target, owner != (recipient ?? "") { dictation.stop() }
    }

    func show(_ message: String) {
        toast = message
        toastTask?.cancel()
        toastTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(4))
            if !Task.isCancelled { self?.toast = nil }
        }
    }

    private func request(_ prefix: String) -> String {
        nextRequest += 1
        return "\(prefix)\(nextRequest)"
    }

    /// Records a request as waiting for the engine's answer. One the engine couldn't be given fails
    /// straight away; one it never answers fails after `requestDeadline`.
    func track(_ id: String, _ op: Pending, sent: Bool) {
        pending[id] = op
        guard sent else {
            fail(id, "Lantern's engine isn't running.", reached: false)
            return
        }
        pendingSince[id] = Date()
        guard deadlineTask == nil else { return }
        deadlineTask = Task { @MainActor [weak self] in
            while let self, !self.pendingSince.isEmpty {
                try? await Task.sleep(for: .seconds(2))
                let late = self.pendingSince.filter { id, since in
                    let limit = if case .launch = self.pending[id] { Self.launchDeadline } else { Self.requestDeadline }
                    return -since.timeIntervalSinceNow > limit
                }.keys
                for id in late { self.expire(id) }
            }
            self?.deadlineTask = nil
        }
    }

    /// `reached`: the engine had the request, so a reply or key may have been typed after all.
    private func fail(_ id: String, _ message: String, reached: Bool) {
        switch pending[id] {
        case .reply, .keys, .direct: replyFinished(id, ok: false, uncertain: reached, message: message)
        case .launch, .focus: actionFinished(id, ok: false, message: message)
        case nil: pendingSince[id] = nil
        }
    }

    /// No answer in time. A reply or key isn't reported as failed (that would invite sending it
    /// twice): it's uncertain, and a late answer is still applied.
    func expire(_ id: String) {
        guard let op = pending[id] else { return pendingSince[id] = nil }
        switch op {
        case .reply, .keys, .direct:
            fail(id, "no answer from Lantern's engine yet. It may still type it: check the terminal before sending again", reached: true)
            overdue[id] = op
        case .launch, .focus:
            fail(id, "no answer from Lantern's engine", reached: true)
        }
    }

    private func handle(_ agentId: String?) -> String {
        agents.first { $0.id == agentId }?.handle ?? "agent"
    }

    // MARK: Engine events

    func update(_ fresh: [Agent]) {
        agents = fresh
        openLaunched()
        Analytics.shared.dailyActive(kinds: Dictionary(fresh.map { ($0.kind.rawValue, 1) }, uniquingKeysWith: +))
        drafts.prune(keeping: Set(fresh.map(\.id)))
        noticeNewTurns()
        // Keep an open conversation current.
        if let chatAgentId {
            if fresh.contains(where: { $0.id == chatAgentId }) { engine.history(agentId: chatAgentId) } else { closeChat() }
        }
    }

    /// Works out which turns are new since the last snapshot, for notifications. On the very first
    /// snapshot after launch, anything that's been waiting over two hours counts as already seen, so
    /// opening Lantern isn't a wall of old sessions.
    private func noticeNewTurns() {
        let waiting = agents.filter { $0.isLive && $0.state.wantsYou && prefs.counts($0.kind) }
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

    func replyFinished(_ requestId: String, ok: Bool, uncertain: Bool, message: String?) {
        pendingSince[requestId] = nil
        if let late = overdue.removeValue(forKey: requestId), case .reply(let ticket) = late {
            if ok { drafts.sent(ticket) }
            show(ok ? "Sent to @\(handle(ticket.recipient)) after all" : "@\(handle(ticket.recipient)): \(message ?? "not sent")")
            return
        }
        guard let request = pending.removeValue(forKey: requestId) else { return }
        switch request {
        case .reply(let ticket):
            let agentId = ticket.recipient
            let agent = agents.first { $0.id == agentId }
            Analytics.shared.capture(
                ok ? "reply_sent" : uncertain ? "reply_uncertain" : "reply_failed",
                ["kind": agent?.kind.rawValue ?? "unknown", "route": agent?.route?.via ?? "unknown"]
            )
            if ok {
                // Only the draft revision that was sent is cleared; anything typed since stays.
                drafts.sent(ticket)
                show("Sent to @\(handle(agentId))")
                if let chatAgentId { engine.history(agentId: chatAgentId) }
            } else if uncertain {
                // It may have gone in: keep the draft, but don't suggest sending it again.
                show("@\(handle(agentId)): \(message ?? "not sure it went in. Check the terminal.")")
            } else {
                show("Couldn't send to @\(handle(agentId)): \(message ?? "unknown error")")
            }
        case .keys(let agentId), .direct(let agentId):
            if ok {
                show("Sent to @\(handle(agentId))")
            } else if uncertain {
                show("@\(handle(agentId)): \(message ?? "not sure it went in. Check the terminal.")")
            } else {
                show("Couldn't answer @\(handle(agentId)): \(message ?? "unknown error")")
            }
        case .launch, .focus:
            break
        }
    }

    private func actionFinished(_ requestId: String, ok: Bool, message: String?) {
        pendingSince[requestId] = nil
        guard let request = pending.removeValue(forKey: requestId) else { return }
        switch request {
        case .launch:
            launching = false
            Analytics.shared.capture(ok ? "session_started" : "session_start_failed", ["kind": launchingKind, "background": launchingInBackground, "screenshot": launchingWithScreenshot, "model": launchingModel, "task": launchingTask, "bus": prefs.busForSessions])
            if ok {
                panel = .agents
                show(message.map { "Session started. \($0)" } ?? "Session started")
            } else {
                launchError = message ?? "unknown error"
            }
        case .focus(let agentId):
            if !ok { show("Couldn't open @\(handle(agentId)): \(message ?? "unknown error")") }
        case .reply, .keys, .direct:
            break
        }
    }

    /// The engine is gone: nothing outstanding will be answered, and nothing it reported is current.
    func engineStopped(_ message: String) {
        // Replies it was typing may or may not have gone in; drafts stay (including other
        // conversations': the agents come back with the same ids in the next snapshot).
        let unsure = pending.values.compactMap { op -> String? in
            switch op {
            case .reply(let ticket): ticket.recipient
            case .keys(let agentId), .direct(let agentId): agentId
            default: nil
            }
        }
        let names = unsure.map { "@\(handle($0))" }.joined(separator: ", ")
        pending.removeAll()
        pendingSince.removeAll()
        overdue.removeAll()
        launching = false
        firstSnapshot = true
        agents = []
        closeChat()
        PhoneModel.shared.reset()
        show(unsure.isEmpty ? message : "\(message) Check \(names): what was being sent may or may not have gone in.")
    }
}
