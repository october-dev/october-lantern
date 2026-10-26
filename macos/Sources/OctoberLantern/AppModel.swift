import AppKit
import Combine
import LanternCore
import SwiftUI

enum PanelMode: Equatable {
    case inbox, agents, newSession, october, assistant

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
    /// Cua Driver's state, once the engine has reported it.
    @Published private(set) var computer: EngineMessage.ComputerStatus?
    /// Installed agents signed in to your own plan (Claude, ChatGPT, x.ai, Google), best first.
    @Published private(set) var signedInKinds: [AgentKind] = []
    /// Installed Claude Code or Codex never signed in: started, it would only ask you to sign in.
    @Published private(set) var neverSetUpKinds: [AgentKind] = []
    @Published private(set) var tmuxAvailable = false
    /// October Harness (built in, installed on first use); `nil` until the engine has said.
    @Published private(set) var harness: EngineMessage.HarnessStatus?
    /// A launch waiting for October Harness to finish installing.
    private var launchAfterHarness: (() -> Void)?
    private var octoberSessionWatch: AnyCancellable?
    private var signInWatch: AnyCancellable?
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
    /// A session starting in Lantern (in the background), shown as its conversation right away:
    /// your message, and "Starting…" until the real conversation has something to show.
    @Published private(set) var launchPreview: LaunchPreview?
    /// Starts the previewed session again in a Terminal window instead.
    private var previewRetry: (() -> Void)?
    /// The preview was closed: the session still starts, but its conversation doesn't open.
    private var previewClosed = false
    /// Who started the launch in flight, when it isn't the panel: Ask Lantern keeps the agent in
    /// its own conversation, so the panel doesn't switch to it.
    private var launchReport: ((LaunchOutcome) -> Void)?
    /// An agent Ask Lantern shows inside its conversation: its messages, read every two seconds
    /// like an open chat's.
    @Published private(set) var watchedAgentId: String?
    @Published private(set) var watchedMessages: [ChatMessage] = []
    private var watchPoll: Task<Void, Never>?
    private var previewTimeout: Task<Void, Never>?
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
        /// Ask Lantern's message for a running agent (through October Bus); `report` gets whether it
        /// went in, whether that's uncertain, and the engine's message.
        case send(agentId: String, report: (_ ok: Bool, _ uncertain: Bool, _ message: String?) -> Void)
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
            if let report = self?.launchReport {
                self?.launchReport = nil
                return report(.started(session: session))
            }
            guard let self, !self.previewClosed else { return }
            self.openWhenSeen = (session, Date().addingTimeInterval(30))
            self.openLaunched()
        }
        engine.onHistory = { [weak self] agentId, supported, messages in
            if let self, agentId == self.watchedAgentId, messages != self.watchedMessages { self.watchedMessages = messages }
            guard let self, agentId == self.chatAgentId else { return }
            if self.chatLoading { self.chatLoading = false }
            // The real conversation takes over from the preview once it has something to show.
            if self.launchPreview != nil, !messages.isEmpty || !supported { self.endPreview() }
            if self.chatSupported != supported { self.chatSupported = supported }
            if messages != self.chatMessages { self.chatMessages = messages }
        }
        engine.onOctober = { [weak self] link in
            if link.status == "connected", self?.octoberLink?.status != "connected" {
                Analytics.shared.capture("october_desktop_connected")
            }
            if self?.octoberLink != link { self?.octoberLink = link }
        }
        engine.onPhone = { state in PhoneModel.shared.receive(state) }
        PhoneModel.shared.engine = engine
        engine.onModels = { [weak self] list in self?.models[list.kind.rawValue] = list }
        engine.onInstalled = { [weak self] installed in
            self?.installedKinds = installed.kinds
            self?.signedInKinds = installed.signedIn ?? []
            self?.neverSetUpKinds = installed.neverSetUp ?? []
            self?.tmuxAvailable = installed.tmux
            self?.installHarnessIfNoOtherAgent()
        }
        engine.onHarness = { [weak self] status in self?.harnessChanged(status) }
        engine.onComputer = { [weak self] status in if self?.computer != status { self?.computer = status } }
        // October launches sign in with your October account: the engine keeps the access token
        // in memory (never the refresh token).
        octoberSessionWatch = OctoberAccount.shared.$session.sink { [weak self] session in
            self?.engine.octoberSession(accessToken: session?.accessToken, expiresAt: session?.expiresAt)
        }
        // Back from the browser: Lantern comes forward where you were (Ask Lantern if nothing was
        // open), with your question still in its box.
        signInWatch = OctoberAccount.shared.$signedInCount.dropFirst().sink { [weak self] _ in
            guard let self else { return }
            NSApp.activate()
            if self.panel == nil { self.panel = .assistant }
            self.composeFocusToken += 1
            let who = OctoberAccount.shared.session?.email.map { "Signed in as \($0)" } ?? "Signed in to October"
            self.show(who)
        }
        engine.onReady = { [weak self] in
            OctoberAccount.shared.engineReady()
            if let self {
                let s = OctoberAccount.shared.session
                self.engine.octoberSession(accessToken: s?.accessToken, expiresAt: s?.expiresAt)
                if self.launchAfterHarness != nil { self.engine.prepareHarness() }
                // Agents can use this Mac's apps: get Cua Driver ready (downloaded on first use).
                if self.prefs.computerUse { self.engine.prepareComputer(enabled: true) }
            }
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

    /// The agents shown as dots on the pill: running ones whose kind counts. Those that need you
    /// come first (only 8 show), then finished, working and the rest; in a steady order within each.
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

    /// Set by the window controller: whether the panel is actually on screen.
    var panelOnScreen: () -> Bool = { true }
    /// Asks the window controller to put the panel back on screen.
    let reshowPanel = PassthroughSubject<Void, Never>()

    /// Opens `mode`'s panel, or closes it when it's the one showing. Lantern believing its panel
    /// is open when it isn't on screen would make every click close or reopen nothing: it's shown
    /// again instead.
    func toggle(_ mode: PanelMode) {
        if panel == mode, !panelOnScreen() {
            AppLog.write("panel: \(mode) was open but not on screen; showing it again")
            return reshowPanel.send()
        }
        panel = panel == mode ? nil : mode
    }

    /// Reply to an agent (opens its conversation, where the composer is), or with no agent, the
    /// "Ask Lantern" box.
    func compose(to agent: Agent?) {
        if let agent { return openChat(agent) }
        if panel != .assistant { panel = .assistant }
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
        if launchPreview != nil { endPreview() }
        engine.cancelHistory()
        if chatAgentId != nil { chatAgentId = nil }
        if !chatMessages.isEmpty { chatMessages = [] }
        chatPoll?.cancel()
        chatPoll = nil
    }

    /// Closes the preview of a starting session; it keeps starting in the background (and shows
    /// in Your agents) but its conversation won't open by itself.
    func closePreview() {
        if launching || pending.values.contains(where: { if case .launch = $0 { return true }; return false }) { previewClosed = true }
        openWhenSeen = nil
        endPreview()
        closeChat()
        if panel == .inbox { panel = .agents }
    }

    /// Starts the previewed session again in a Terminal window (after it failed in Lantern).
    func retryPreviewInTerminal() {
        guard let retry = previewRetry, !launching else { return }
        endPreview()
        retry()
    }

    private func reportLaunch(_ outcome: LaunchOutcome) {
        guard let report = launchReport else { return }
        launchReport = nil
        report(outcome)
    }

    /// For snapshots: shows a preview without starting anything.
    func showPreview(_ preview: LaunchPreview) { launchPreview = preview }

    private func endPreview() {
        previewTimeout?.cancel()
        previewTimeout = nil
        previewRetry = nil
        if launchPreview != nil { launchPreview = nil }
    }

    /// The session started; if its conversation hasn't taken over after a while (the agent is
    /// slow to show up or to write its session file), stop waiting and show what there is.
    private func waitForLaunched() {
        previewTimeout?.cancel()
        previewTimeout = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(30))
            guard let self, !Task.isCancelled, self.launchPreview != nil else { return }
            self.endPreview()
            if self.chatAgentId == nil {
                self.panel = .agents
                self.show("Session started. It shows in Your agents when it's up.")
            }
        }
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
            // The process above the agent can be one of the app's helpers (October's terminals
            // run under a helper process), so bring the app forward by its bundle.
            let bundle = URL(fileURLWithPath: host.bundlePath).standardizedFileURL
            let running = NSWorkspace.shared.runningApplications.first { $0.bundleURL?.standardizedFileURL == bundle }
                ?? (host.pid == 0 ? nil : NSRunningApplication(processIdentifier: host.pid))
            if let running, running.activationPolicy == .regular {
                running.activate()
            } else {
                NSWorkspace.shared.open(bundle)
            }
        }
    }

    /// Sends `text` to `agent` without touching the composer's drafts (Point & Ask). Agents Lantern
    /// can't type into get it copied, and their app comes forward.
    /// Ask Lantern's message for a running agent: through October Bus when it's on it (with a
    /// nudge in its terminal), else typed in.
    func sendFromAssistant(_ text: String, to agent: Agent, report: @escaping (_ ok: Bool, _ uncertain: Bool, _ message: String?) -> Void) {
        let id = request("s")
        track(id, .send(agentId: agent.id, report: report), sent: engine.send(requestId: id, agentId: agent.id, text: text))
    }

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

    /// What was sent for each question (by prompt id), shown as "Sent" on its card until the agent
    /// moves on. Cleared again when the engine couldn't press the keys.
    @Published var answered: [String: AgentAnswer] = [:]
    private var answering: [String: String] = [:]

    /// Answers an agent's question from its card. The engine refuses if the question on screen is
    /// no longer the one shown here.
    func answer(_ agent: Agent, _ answer: AgentAnswer) {
        guard let promptId = agent.promptId, answered[promptId] == nil else { return }
        Analytics.shared.capture(
            agent.questionKind == "permission" ? "permission_answered" : "question_answered",
            ["answer": answer.kind, "kind": agent.questionKind ?? "none", "route": agent.route?.via ?? "none"]
        )
        let id = request("k")
        answered[promptId] = answer
        answering[id] = promptId
        let sent = engine.answer(requestId: id, agentId: agent.id, promptId: promptId, answer: answer)
        track(id, .keys(agentId: agent.id), sent: sent)
        if !sent { answered[promptId] = nil }
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
        refreshInstalled()
        panel = .newSession
    }

    // MARK: Default agent and October Harness

    /// The agent new sessions and tasks start with: the `defaultAgent` preference if it's set and
    /// can start; else an agent signed in to a plan you already pay for (Claude Code, Codex, Grok,
    /// Gemini), so your own subscription is used before October's; else October (counted when
    /// Lantern can install October Harness), Claude Code, Codex, then the first installed agent.
    /// A Claude Code or Codex never signed in comes last. `nil` until Lantern knows what's
    /// installed.
    var defaultAgentKind: AgentKind? {
        guard let installed = installedKinds else { return nil }
        // October Harness counts once it's here (on PATH or installed by Lantern), or when there's
        // no Claude Code or Codex set up: then Lantern installs it in the background (see
        // `installHarnessIfNoOtherAgent`) and a launch waits for it.
        // After a failed install, October is tried again only when there's nothing else to start.
        let other = installed.contains { !self.neverSetUpKinds.contains($0) }
        let usable = { (k: AgentKind) in
            installed.contains(k)
                || (k == .october && (self.harness?.isReady == true || (self.needsHarness && (self.harness?.status != "failed" || !other))))
        }
        let chosen = prefs.defaultAgent.isEmpty ? [] : [AgentKind(rawValue: prefs.defaultAgent)]
        for kind in chosen where usable(kind) { return kind }
        for kind in signedInKinds + [.october, .claude, .codex] where usable(kind) && !neverSetUpKinds.contains(kind) { return kind }
        return installed.first { !neverSetUpKinds.contains($0) } ?? installed.first
    }

    /// No Claude Code or Codex that's been set up: October Harness is what this Mac can run.
    var needsHarness: Bool {
        guard let installed = installedKinds else { return false }
        return ![AgentKind.claude, .codex].contains { installed.contains($0) && !neverSetUpKinds.contains($0) }
    }

    /// Looks for installed agents again, so one installed since Lantern started shows up.
    func refreshInstalled() { engine.refreshInstalled() }

    /// Without Claude Code or Codex, installs October Harness in the background once (about
    /// 170 MB, plus Node.js if this Mac has none) so Ask Lantern's tasks and New Session have an
    /// agent to start. Macs with either one download nothing.
    func installHarnessIfNoOtherAgent() {
        guard needsHarness, !octoberReady, harness?.status != "installing", harness?.status != "failed" else { return }
        prepareHarness()
    }

    /// Turns "Let agents use this Mac's apps" on or off.
    func setComputerUse(_ on: Bool) {
        prefs.computerUse = on
        engine.prepareComputer(enabled: on)
        Analytics.shared.capture("computer_use", ["on": on])
    }

    /// Starts Lantern's Cua Driver again, so it picks up a permission just granted.
    func restartComputer() {
        guard prefs.computerUse else { return }
        engine.prepareComputer(enabled: true, restart: true)
    }

    /// October can start now: on PATH, or October Harness installed.
    var octoberReady: Bool { harness?.isReady == true || (installedKinds ?? []).contains(.october) }

    /// Installs October Harness in the background if it isn't ready (no UI waits on it; `harness`
    /// follows its progress). Safe to call often.
    func prepareHarness() {
        guard !octoberReady, harness?.status != "installing" else { return }
        engine.prepareHarness()
    }

    private func harnessChanged(_ status: EngineMessage.HarnessStatus) {
        if harness != status { harness = status }
        guard let next = launchAfterHarness else { return }
        if status.isReady {
            launchAfterHarness = nil
            launching = false
            next()
        } else if status.status == "failed" {
            launchAfterHarness = nil
            launching = false
            launchError = "October couldn't be set up. Check your internet connection, then try again. (\(status.message ?? "unknown error"))"
            launchPreview?.error = launchError
            reportLaunch(.failed(launchError ?? ""))
        }
    }

    /// Starts a task: `prompt` for `agent` (nil: `defaultAgentKind`) in `folder`, with the toolkit
    /// list, like a Task from New Session. October waits for October Harness to install first when
    /// needed (`launching` is true meanwhile); other agents start right away. The outcome shows
    /// the usual way (`launching`, `launchError`, the "Session started" toast). False when no
    /// agent can start or another session is still starting.
    /// `report`: for a launch that stays out of the panel (Ask Lantern's), told how it went.
    @discardableResult
    func launchTask(
        prompt: String, agent: AgentKind?, folder: String, background: Bool, screenshot: URL? = nil, images: [URL] = [],
        context: String? = nil, report: ((LaunchOutcome) -> Void)? = nil
    ) -> Bool {
        guard let kind = agent ?? defaultAgentKind, !launching else { return false }
        launch(
            kind: kind, folder: folder, prompt: prompt, screenshot: screenshot, images: images, context: context, toolkit: true,
            background: background, report: report
        )
        return true
    }

    /// Reads `agentId`'s conversation every two seconds into `watchedMessages` (nil stops).
    func watch(_ agentId: String?) {
        guard agentId != watchedAgentId else { return }
        watchPoll?.cancel()
        watchedAgentId = agentId
        watchedMessages = []
        guard let agentId else { return }
        watchPoll = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let self, self.watchedAgentId == agentId else { return }
                self.engine.history(agentId: agentId)
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    /// A message (and folder) for New Session to start with, from the assistant's Do.
    private(set) var newSessionPrefill: (prompt: String, folder: String?)?

    /// Opens New Session filled in with a task: a plain session, not a task for the app in front.
    func startNew(prompt: String, folder: String?) {
        newSessionPrefill = (prompt, folder)
        if panel == .newSession { panel = nil }
        panel = .newSession
    }

    func takeNewSessionPrefill() -> (prompt: String, folder: String?)? {
        defer { newSessionPrefill = nil }
        return newSessionPrefill
    }

    func refreshToolkit() { engine.refreshToolkit() }

    /// The folder a Task for this app used last time.
    func taskFolder(for bundleId: String?) -> String? {
        guard let bundleId else { return nil }
        return (UserDefaults.standard.dictionary(forKey: "taskFolders") as? [String: String])?[bundleId]
    }

    func launch(
        kind: AgentKind, folder: String, prompt: String, screenshot: URL? = nil, images: [URL] = [], model: String? = nil,
        context: String? = nil, toolkit: Bool = false, taskApp: String? = nil, background: Bool, report: ((LaunchOutcome) -> Void)? = nil
    ) {
        guard !launching, !pending.values.contains(where: { if case .launch = $0 { return true }; return false }) else {
            report?(.failed("Another session is still starting."))
            return
        }
        launchReport = report
        if report != nil {
            // Ask Lantern shows it in its own conversation.
        } else if background {
            // Show the conversation now; the real one replaces it when the agent is up.
            if chatAgentId != nil { closeChat() }
            launchPreview = LaunchPreview(kind: kind, prompt: prompt, folder: folder)
            previewClosed = false
            previewRetry = { [weak self] in
                self?.launch(
                    kind: kind, folder: folder, prompt: prompt, screenshot: screenshot, images: images, model: model, context: context,
                    toolkit: toolkit, taskApp: taskApp, background: false
                )
            }
            if !(panel?.isList ?? false) { panel = .inbox }
        } else if launchPreview != nil {
            endPreview()
        }
        // October without October Harness yet: install it first, then launch (the engine's launch
        // deadline is too short for a first install).
        if kind == .october, installedKinds != nil, !octoberReady {
            launching = true
            launchError = nil
            launchAfterHarness = { [weak self] in
                self?.launch(
                    kind: kind, folder: folder, prompt: prompt, screenshot: screenshot, images: images, model: model, context: context,
                    toolkit: toolkit, taskApp: taskApp, background: background, report: report
                )
            }
            if harness?.status != "installing" { engine.prepareHarness() }
            return
        }
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
        launchingWithScreenshot = screenshot != nil || !images.isEmpty
        launchingTask = taskApp ?? "none"
        launchingModel = model ?? "default"
        let id = request("l")
        track(
            id, .launch,
            sent: engine.launch(
                requestId: id, kind: kind, cwd: folder, prompt: prompt, screenshot: screenshot?.path, images: images.map(\.path), model: model,
                context: context,
                toolkit: toolkit, bus: prefs.busForSessions, computer: prefs.computerUse, askFirst: prefs.askFirst, background: background
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
        case .reply, .keys, .direct, .send: replyFinished(id, ok: false, uncertain: reached, message: message)
        case .launch, .focus: actionFinished(id, ok: false, message: message)
        case nil: pendingSince[id] = nil
        }
    }

    /// No answer in time. A reply or key isn't reported as failed (that would invite sending it
    /// twice): it's uncertain, and a late answer is still applied.
    func expire(_ id: String) {
        guard let op = pending[id] else { return pendingSince[id] = nil }
        switch op {
        case .reply, .keys, .direct, .send:
            fail(id, "no answer from Lantern's engine yet. It may still type it: check the terminal before sending again", reached: true)
            overdue[id] = op
        case .launch:
            // Preparation may still be finishing, or the session may already have started.
            // Keep its ticket and disable duplicate launches until the engine confirms the
            // outcome. The engine uses this same deadline before creating any session.
            pendingSince[id] = nil
            show("Still preparing the session. Waiting for the engine to confirm its outcome.")
            // A hard ceiling, so a launch the engine never answers can't block new sessions.
            DispatchQueue.main.asyncAfter(deadline: .now() + 180) { [weak self] in
                guard let self, case .launch = self.pending[id] else { return }
                self.pending[id] = nil
                self.show("The engine never confirmed that session. Check Agents before starting it again.")
            }
        case .focus:
            fail(id, "no answer from Lantern's engine", reached: true)
        }
    }

    private func handle(_ agentId: String?) -> String {
        agents.first { $0.id == agentId }?.handle ?? "agent"
    }

    // MARK: Engine events

    func update(_ fresh: [Agent]) {
        if agents != fresh { agents = fresh }
        openLaunched()
        Analytics.shared.dailyActive(kinds: Dictionary(fresh.map { ($0.kind.rawValue, 1) }, uniquingKeysWith: +))
        var pruned = drafts
        pruned.prune(keeping: Set(fresh.map(\.id)))
        if pruned != drafts { drafts = pruned }
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
            show(ok ? "\(message ?? "Sent") to @\(handle(ticket.recipient)) after all" : "@\(handle(ticket.recipient)): \(message ?? "not sent")")
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
                show("\(message ?? "Sent") to @\(handle(agentId))")
                if let chatAgentId { engine.history(agentId: chatAgentId) }
            } else if uncertain {
                // It may have gone in: keep the draft, but don't suggest sending it again.
                show("@\(handle(agentId)): \(message ?? "not sure it went in. Check the terminal.")")
            } else {
                show("Couldn't send to @\(handle(agentId)): \(message ?? "unknown error")")
            }
        case .send(_, let report):
            report(ok, uncertain, message)
        case .keys(let agentId), .direct(let agentId):
            if let promptId = answering.removeValue(forKey: requestId), !ok, !uncertain { answered[promptId] = nil }
            if ok {
                show("\(message ?? "Sent") to @\(handle(agentId))")
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
            if launchReport != nil {
                if ok {
                    // The tmux session (if any) comes right after, in onLaunched.
                    DispatchQueue.main.async { [weak self] in self?.reportLaunch(.started(session: nil)) }
                } else {
                    reportLaunch(.failed(message ?? "unknown error"))
                }
            } else if ok, launchPreview != nil {
                waitForLaunched()
            } else if ok {
                if !previewClosed { panel = .agents }
                previewClosed = false
                show(message.map { "Session started. \($0)" } ?? "Session started")
            } else {
                launchError = message ?? "unknown error"
                launchPreview?.error = launchError
            }
        case .focus(let agentId):
            if !ok { show("Couldn't open @\(handle(agentId)): \(message ?? "unknown error")") }
        case .reply, .keys, .direct, .send:
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
            case .keys(let agentId), .direct(let agentId), .send(let agentId, _): agentId
            default: nil
            }
        }
        let names = unsure.map { "@\(handle($0))" }.joined(separator: ", ")
        pending.removeAll()
        pendingSince.removeAll()
        overdue.removeAll()
        launching = false
        launchAfterHarness = nil
        launchPreview?.error = "Lantern's engine stopped before the session started."
        reportLaunch(.failed("Lantern's engine stopped before the session started."))
        firstSnapshot = true
        agents = []
        closeChat()
        PhoneModel.shared.reset()
        show(unsure.isEmpty ? message : "\(message) Check \(names): what was being sent may or may not have gone in.")
    }
}

/// A session starting in Lantern, shown before its real conversation exists.
struct LaunchPreview: Equatable {
    let kind: AgentKind
    let prompt: String
    let folder: String
    /// Why it couldn't start.
    var error: String?
}

/// How a launch for Ask Lantern went: started (in a tmux session when it has one), or why not.
enum LaunchOutcome {
    case started(session: String?)
    case failed(String)
}
