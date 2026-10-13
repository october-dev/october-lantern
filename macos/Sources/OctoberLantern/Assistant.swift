import SwiftUI

/// Lantern's assistant, opened by clicking the lantern: a chat with October's AI (the same
/// endpoint as Point & Ask, without a screenshot) that knows what your agents are doing. When the
/// answer asks for work, an agent starts on it in the background and stays in the conversation:
/// its replies stream into the thread and what you type next goes to it. Conversations are kept
/// on this Mac (the most recent 50) and listed on Ask Lantern's home.
@MainActor
final class Assistant: ObservableObject {
    static let shared = Assistant()
    weak var model: AppModel?

    /// A task the answer asked Lantern to start, waiting out its cancel window.
    struct PendingTask: Identifiable, Equatable {
        let id = UUID()
        let prompt: String
        let kind: AgentKind
        let folder: String
        var secondsLeft: Int
    }

    struct Turn: Identifiable, Codable {
        var id = UUID()
        let question: String
        var answer = ""
        var failed = false
        var note: String?
        /// "Started Claude Code in october-lantern", once a task from this answer launched.
        var started: String?
        /// The agent this answer started, shown in the conversation.
        var task: TaskLink?
    }

    /// An agent Ask Lantern started: found by its tmux session, then by its id. Its last messages
    /// are kept, so the conversation still shows what it did after it ends.
    struct TaskLink: Codable, Equatable {
        let kind: String
        let folder: String
        let prompt: String
        var session: String?
        var agentId: String?
        var error: String?
        /// Started without a tmux session (a Terminal window): nothing to show here.
        var inTerminal = false
        var transcript: [ChatMessage] = []
    }

    struct Conversation: Identifiable, Codable {
        var id = UUID()
        var title: String
        var updated: Date
        var turns: [Turn]
    }

    @Published var text = ""
    @Published private(set) var pending: PendingTask?
    private var countdown: Task<Void, Never>?
    @Published private(set) var turns: [Turn] = [] { didSet { if !restoring { saveSoon() } } }
    @Published private(set) var asking = false
    /// Earlier conversations, newest first (the current one included once it has a question).
    @Published private(set) var conversations: [Conversation] = []
    private var currentId = UUID()
    private var restoring = false
    /// With an agent at work in the conversation, the box talks to it; this sends the next
    /// message to Lantern instead.
    @Published var askLanternInstead = false
    let dictation = Dictation()
    private var task: Task<Void, Never>?
    private var saving: Task<Void, Never>?

    private init() {
        dictation.onText = { [weak self] text, _ in self?.text = text }
        conversations = Self.load()
    }

    func submit() {
        if let agent = activeAgent, !askLanternInstead {
            reply(to: agent)
        } else {
            ask()
            askLanternInstead = false
        }
    }

    /// A new chat (the current one is kept in the list).
    func clear() {
        task?.cancel()
        cancelTask()
        save()
        restoring = true
        turns = []
        restoring = false
        currentId = UUID()
        asking = false
        askLanternInstead = false
        model?.watch(nil)
    }

    /// Reopens a saved conversation; an agent from it that's still running is live again.
    func open(_ conversation: Conversation) {
        clear()
        restoring = true
        turns = conversation.turns
        restoring = false
        currentId = conversation.id
    }

    func delete(_ conversation: Conversation) {
        conversations.removeAll { $0.id == conversation.id }
        if conversation.id == currentId { clear() }
        Self.store(conversations)
    }

    // MARK: The agent in the conversation

    /// The running agent from the latest task in this conversation.
    var activeAgent: Agent? {
        guard let link = turns.last(where: { $0.task != nil })?.task else { return nil }
        return agent(for: link).flatMap { $0.isLive ? $0 : nil }
    }

    /// The agent a task started, once it's up: by id once known, else by its tmux session.
    func agent(for link: TaskLink) -> Agent? {
        guard let model else { return nil }
        if let id = link.agentId { return model.agents.first { $0.id == id } }
        guard let session = link.session else { return nil }
        return model.agents.first { $0.tmux?.target.hasPrefix("\(session):") ?? false }
    }

    /// Remembers which agent a task became and what it has said, as its messages arrive.
    func track(_ link: TaskLink, agent: Agent?, messages: [ChatMessage]) {
        guard let i = turns.lastIndex(where: { $0.task == link }) else { return }
        var updated = link
        if let agent, updated.agentId == nil { updated.agentId = agent.id }
        if !messages.isEmpty { updated.transcript = Array(messages.suffix(60)) }
        if updated != link { turns[i].task = updated }
    }

    private func reply(to agent: Agent) {
        let message = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !message.isEmpty, let model else { return }
        dictation.stop()
        model.sendDirect(message, to: agent)
        text = ""
        Analytics.shared.capture("assistant_task_reply", ["kind": agent.kind.rawValue])
    }

    /// For snapshots: recent chats on the home, or a thread with a finished task in it. Nothing
    /// is saved.
    func demo(thread: Bool) {
        restoring = true
        defer { restoring = false }
        let now = Date()
        var link = TaskLink(kind: "claude", folder: NSHomeDirectory(), prompt: "Find me my thesis document")
        link.agentId = "demo"
        link.transcript = [
            ChatMessage(role: .user, text: "Find me my thesis document", at: nil),
            ChatMessage(role: .tool, text: "bash · mdfind -name thesis", at: nil),
            ChatMessage(role: .agent, text: "Found it: **/Users/you/Documents/University/Thesis-final.pdf** (edited 3 days ago). There's also an older draft in Downloads.", at: nil),
        ]
        let turn = Turn(question: "Find me my thesis document", answer: "On it: Claude Code will look for it.", started: "Started Claude Code in Home folder", task: link)
        let convos = [
            Conversation(title: "Find me my thesis document", updated: now.addingTimeInterval(-120), turns: [turn]),
            Conversation(title: "Which agent needs me first?", updated: now.addingTimeInterval(-3600), turns: []),
            Conversation(title: "Fix the Wi-Fi", updated: now.addingTimeInterval(-86400), turns: [turn]),
        ]
        conversations = convos
        turns = thread ? [turn] : []
    }

    // MARK: Saved conversations

    private static var file: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/October Lantern/ask-lantern.json")
    }

    private static func load() -> [Conversation] {
        guard let data = try? Data(contentsOf: file) else { return [] }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return (try? decoder.decode([Conversation].self, from: data)) ?? []
    }

    private static func store(_ conversations: [Conversation]) {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        guard let data = try? encoder.encode(conversations) else { return }
        let dir = file.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try? data.write(to: file, options: [.atomic])
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
    }

    /// Writes the conversation list a moment after changes settle (answers stream in pieces).
    private func saveSoon() {
        saving?.cancel()
        saving = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(1))
            guard !Task.isCancelled else { return }
            self?.save()
        }
    }

    private func save() {
        saving?.cancel()
        let kept = turns.filter { !$0.question.isEmpty }
        guard !kept.isEmpty else { return }
        let conversation = Conversation(id: currentId, title: Self.title(kept[0].question), updated: Date(), turns: kept)
        conversations.removeAll { $0.id == currentId }
        conversations.insert(conversation, at: 0)
        conversations = Array(conversations.prefix(50))
        Self.store(conversations)
    }

    static func title(_ question: String) -> String {
        let line = question.split(whereSeparator: \.isNewline).first.map(String.init) ?? question
        return line.count > 60 ? String(line.prefix(59)) + "…" : line
    }

    private func ask() {
        let question = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !question.isEmpty, !asking, OctoberAccount.shared.signedIn else { return }
        dictation.stop()
        text = ""
        turns.append(Turn(question: question))
        asking = true
        let id = turns[turns.count - 1].id
        let request = OctoberAI.Request(
            turns: turns.dropLast().filter { !$0.failed }.map { ($0.question, $0.answer) },
            question: question, image: nil, context: agentSummary(), mode: "chat"
        )
        task = Task {
            do {
                for try await piece in OctoberAI.stream(request) {
                    guard !Task.isCancelled, let i = turns.firstIndex(where: { $0.id == id }) else { return }
                    switch piece {
                    case .text(let t): turns[i].answer += t
                    case .note(let n): turns[i].note = n
                    case .task(let prompt, let agent, let project): propose(prompt: prompt, agent: agent, project: project, turn: id)
                    }
                }
                if let i = turns.firstIndex(where: { $0.id == id }), turns[i].answer.isEmpty {
                    turns[i].answer = "No answer came back. Try again."
                    turns[i].failed = true
                }
            } catch is CancellationError {
            } catch {
                if let i = turns.firstIndex(where: { $0.id == id }) {
                    turns[i].answer = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
                    turns[i].failed = true
                }
            }
            guard !Task.isCancelled else { return }
            asking = false
            Analytics.shared.capture("assistant_asked", ["follow_up": turns.count > 1])
        }
    }

    // MARK: Tasks the assistant starts

    /// What the agent is asked: your own words when you asked for it outright, not October's
    /// rewording ("find the user's thesis"). A follow-up ("yes, do that") only makes sense with
    /// the conversation, so then October's self-contained version is used.
    static func taskPrompt(question: String, suggested: String, followUp: Bool) -> String {
        let mine = question.trimmingCharacters(in: .whitespacesAndNewlines)
        return followUp || mine.isEmpty ? suggested : mine
    }

    /// The answer asked for work to be done: start it after a short window to cancel.
    private func propose(prompt: String, agent: String?, project: String?, turn: UUID) {
        guard let model else { return }
        let installed = model.installedKinds ?? []
        let named = agent.flatMap { AgentKind(rawValue: $0) }.flatMap { installed.contains($0) ? $0 : nil }
        guard let kind = named ?? model.defaultAgentKind else { return }
        let folder = Self.folder(for: project, model: model)
        let index = turns.firstIndex { $0.id == turn }
        let own = index.map { Self.taskPrompt(question: turns[$0].question, suggested: prompt, followUp: $0 > 0) } ?? prompt
        cancelTask()
        pending = PendingTask(prompt: own, kind: kind, folder: folder, secondsLeft: 3)
        countdown = Task { [weak self] in
            for left in stride(from: 2, through: 0, by: -1) {
                try? await Task.sleep(for: .seconds(1))
                guard let self, !Task.isCancelled, self.pending != nil else { return }
                if left > 0 { self.pending?.secondsLeft = left } else { self.startPending(turn: turn) }
            }
        }
    }

    func startNow() {
        guard let turn = turns.last?.id else { return }
        countdown?.cancel()
        startPending(turn: turn)
    }

    func cancelTask() {
        countdown?.cancel()
        pending = nil
    }

    private func startPending(turn: UUID) {
        guard let p = pending, let model else { return }
        pending = nil
        guard let i = turns.firstIndex(where: { $0.id == turn }) else { return }
        // In the background (tmux) when possible, shown right here in the conversation; Open
        // shows its terminal. Without tmux, a Terminal window. Waits for October Harness to
        // finish installing when that's the agent.
        turns[i].task = TaskLink(kind: p.kind.rawValue, folder: p.folder, prompt: p.prompt, inTerminal: !model.tmuxAvailable)
        let started = model.launchTask(prompt: p.prompt, agent: p.kind, folder: p.folder, background: model.tmuxAvailable) { [weak self] outcome in
            guard let self, let i = self.turns.firstIndex(where: { $0.id == turn }) else { return }
            switch outcome {
            case .started(let session):
                if let session { self.turns[i].task?.session = session }
                self.turns[i].started = "Started \(p.kind.displayName) in \(URL(fileURLWithPath: p.folder).lastPathComponent)"
            case .failed(let message):
                self.turns[i].task?.error = message
            }
        }
        if !started { turns[i].task?.error = "Another session is still starting. Try again in a moment." }
        Analytics.shared.capture("assistant_task", ["kind": p.kind.rawValue])
    }

    /// The agent tasks start with: the one you picked in Settings if it's installed, else October,
    /// Claude Code, Codex, then whatever is installed.
    static func defaultKind(_ installed: [AgentKind]) -> AgentKind? {
        let preferred = UserDefaults.standard.string(forKey: "defaultAgent").flatMap(AgentKind.init(rawValue:))
        let order = [preferred] + ["october", "claude", "codex"].map { AgentKind(rawValue: $0) }
        for k in order.compactMap({ $0 }) where installed.contains(k) { return k }
        return installed.first
    }

    /// The project you named (matched against recent folders and agents' folders); otherwise
    /// your home folder.
    static func folder(for project: String?, model: AppModel) -> String {
        let agentFolders = model.agents.sorted { ($0.stateSince ?? 0) > ($1.stateSince ?? 0) }.compactMap(\.cwd)
        let candidates = model.recentFolders + agentFolders
        if let project = project?.lowercased(), !project.isEmpty,
           let hit = candidates.first(where: { URL(fileURLWithPath: $0).lastPathComponent.lowercased() == project })
            ?? candidates.first(where: { URL(fileURLWithPath: $0).lastPathComponent.lowercased().contains(project) }) {
            return hit
        }
        // No project named: start in the home folder, where the agent can look anywhere
        // ("find my passport" isn't about whichever repo was used last).
        return FileManager.default.homeDirectoryForCurrentUser.path
    }

    /// A recent folder whose name the request mentions ("fix the tests in october-lantern").
    static func guessFolder(_ request: String, recent: [String]) -> String? {
        let words = request.lowercased()
        return recent.first { folder in
            let name = URL(fileURLWithPath: folder).lastPathComponent.lowercased()
            return name.count >= 3 && words.contains(name)
        }
    }

    /// What the agents are doing, for the model: one line each, most urgent first.
    private func agentSummary() -> String {
        guard let model else { return "" }
        let agents = model.agents.filter(\.isLive).sorted { rank($0) < rank($1) }
        guard !agents.isEmpty else { return "Lantern's assistant chat (no screenshot). No coding agents are running on this Mac." }
        var lines = ["Lantern's assistant chat (no screenshot). The user's coding agents on this Mac right now:"]
        for a in agents.prefix(20) {
            var line = "- @\(a.handle) (\(a.kind.displayName)) in \(a.project ?? "?"): \(a.state.label)"
            if let t = a.title { line += "; working on \"\(t.prefix(80))\"" }
            if let q = a.question, a.state.wantsYou { line += "; asking: \(q.prefix(160))" }
            if let m = a.lastMessage { line += "; last said: \(m.replacingOccurrences(of: "\n", with: " ").prefix(200))" }
            lines.append(line)
        }
        return String(lines.joined(separator: "\n").prefix(6000))
    }

    private func rank(_ a: Agent) -> Int { a.state.wantsYou ? 0 : a.state == .working ? 1 : 2 }
}

/// The assistant panel: Ask or Do, a strip for agents waiting on you, and the conversation.
/// The assistant's conversation, shown in the panel in place of the agent list.
struct AssistantThread: View {
    @ObservedObject var model: AppModel
    @ObservedObject var assistant = Assistant.shared

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Button { assistant.clear() } label: {
                    Label("New chat", systemImage: "chevron.left")
                        .font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.muted)
                }
                .buttonStyle(.plain)
                .help("Start a new chat. This one stays in Recent.")
                Spacer()
                Button { model.panel = nil } label: {
                    Image(systemName: "xmark").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.muted).frame(width: 22, height: 22)
                }
                .buttonStyle(.plain).help("Close")
            }
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        ForEach(assistant.turns) { turn in
                            TurnView(turn: turn).id(turn.id)
                            if let link = turn.task {
                                TaskCard(link: link, latest: turn.id == assistant.turns.last(where: { $0.task != nil })?.id, model: model)
                            }
                        }
                        if let p = assistant.pending { pendingCard(p) }
                    }
                    .padding(.vertical, 4)
                }
                .frame(maxHeight: max(200, PanelMetrics.shared.maxHeight - 220))
                .fixedSize(horizontal: false, vertical: true)
                .onChange(of: assistant.turns.last?.answer) {
                    if let last = assistant.turns.last { proxy.scrollTo(last.id, anchor: .bottom) }
                }
                .onChange(of: model.watchedMessages.count) { proxy.scrollTo("task-bottom", anchor: .bottom) }
            }
        }
        .padding(.horizontal, 16).padding(.top, 14).padding(.bottom, 6)
        // Only the latest task's agent is read live; the rest show what they last said.
        .onChange(of: assistant.activeAgent?.id, initial: true) { _, id in model.watch(id) }
        .onDisappear { model.watch(nil) }
    }

    private func pendingCard(_ p: Assistant.PendingTask) -> some View {
        HStack(spacing: 10) {
            ProgressView().controlSize(.small)
            VStack(alignment: .leading, spacing: 2) {
                Text("Starting \(p.kind.displayName) in \(URL(fileURLWithPath: p.folder).lastPathComponent)… \(p.secondsLeft)")
                    .font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.ink)
                Text(p.prompt).font(.system(size: 11)).foregroundStyle(Theme.muted).lineLimit(2)
            }
            Spacer()
            Button("Cancel") { Assistant.shared.cancelTask() }.buttonStyle(.plain)
                .font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.muted)
            Button("Start") { Assistant.shared.startNow() }.buttonStyle(.plain)
                .font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.amber)
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.amber.opacity(0.1)))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Theme.amber.opacity(0.3)))
    }
}

/// An agent Ask Lantern started, inside the conversation: who and where, its state, and what it
/// has said (live for the latest task, as last seen for earlier ones).
private struct TaskCard: View {
    let link: Assistant.TaskLink
    let latest: Bool
    @ObservedObject var model: AppModel
    @ObservedObject var assistant = Assistant.shared

    private var kind: AgentKind { AgentKind(rawValue: link.kind) }
    private var agent: Agent? { assistant.agent(for: link) }

    /// The conversation without your first message (it's the question above).
    private var messages: [ChatMessage] {
        let live = latest && agent != nil && agent?.id == model.watchedAgentId && !model.watchedMessages.isEmpty
        let all = live ? model.watchedMessages : link.transcript
        guard let first = all.first, first.role == .user,
              Bubble.withoutLanternNotes(first.text).trimmingCharacters(in: .whitespacesAndNewlines) == link.prompt else { return all }
        return Array(all.dropFirst())
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            header
            if let agent, agent.isPermissionPrompt {
                Text(agent.question ?? "").font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.amber)
                    .fixedSize(horizontal: false, vertical: true)
                PermissionButtons(agent: agent, model: model)
            }
            ForEach(ChatView.keyed(messages), id: \.key) { item in Bubble(message: item.message) }
            status
            Color.clear.frame(height: 1).id(latest ? "task-bottom" : "task-\(link.prompt.hashValue)")
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Color.black.opacity(0.18)))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Theme.hairline))
        .onChange(of: agent?.id, initial: true) { _, _ in assistant.track(link, agent: agent, messages: []) }
        .onChange(of: model.watchedMessages) { _, now in
            if latest, let agent, agent.id == model.watchedAgentId { assistant.track(link, agent: agent, messages: now) }
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            if let agent { AgentBadge(agent: agent, size: 20) } else if let img = Assets.harness(kind) {
                Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 20, height: 20)
                    .clipShape(RoundedRectangle(cornerRadius: 4.4, style: .continuous))
            }
            VStack(alignment: .leading, spacing: 0) {
                Text(agent.map { "@\($0.handle)" } ?? kind.displayName).font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.ink)
                Text(folderName).font(.system(size: 10.5)).foregroundStyle(Theme.muted).lineLimit(1)
            }
            Spacer(minLength: 6)
            if let agent {
                AgentStateChip(agent: agent)
                SmallButton(title: "Open", symbol: "arrow.up.forward.app") { model.open(agent) }
            }
        }
    }

    @ViewBuilder private var status: some View {
        if let error = link.error {
            Text("Couldn't start it: \(error)").font(.system(size: 11.5)).foregroundStyle(Theme.red)
                .fixedSize(horizontal: false, vertical: true)
        } else if let agent {
            if agent.state == .working {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.mini)
                    Text("Working…").font(.system(size: 11)).foregroundStyle(Theme.muted)
                }
            }
        } else if link.inTerminal {
            Text("Running in a Terminal window.").font(.system(size: 11)).foregroundStyle(Theme.muted)
        } else if link.agentId != nil || !link.transcript.isEmpty {
            Text("Finished. It isn't running anymore.").font(.system(size: 11)).foregroundStyle(Theme.muted)
        } else if latest {
            HStack(spacing: 6) {
                ProgressView().controlSize(.mini)
                Text("Starting \(kind.displayName)…").font(.system(size: 11)).foregroundStyle(Theme.muted)
            }
        }
    }

    private var folderName: String {
        link.folder == NSHomeDirectory() ? "Home folder" : URL(fileURLWithPath: link.folder).lastPathComponent
    }
}

private struct TurnView: View {
    let turn: Assistant.Turn

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(turn.question).font(.system(size: 12.5, weight: .semibold)).foregroundStyle(Theme.ink)
                .frame(maxWidth: .infinity, alignment: .leading)
            if turn.answer.isEmpty {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Thinking…").font(.system(size: 12)).foregroundStyle(Theme.muted)
                }
            } else {
                Text((try? AttributedString(markdown: turn.answer, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(turn.answer))
                    .font(.system(size: 12.5)).lineSpacing(2)
                    .foregroundStyle(turn.failed ? Theme.red : Theme.ink.opacity(0.92))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let started = turn.started, turn.task == nil {
                Label(started, systemImage: "play.circle.fill").font(.system(size: 11.5, weight: .medium)).foregroundStyle(Theme.green)
            }
            if let note = turn.note { Text(note).font(.system(size: 11)).foregroundStyle(Theme.muted) }
        }
    }
}

/// "Ask Lantern…" at the bottom of the agent panel: questions get answers; requests for work
/// start an agent.
struct AskBox: View {
    @ObservedObject var model: AppModel
    /// Lantern's home uses a bigger box.
    var large = false
    @ObservedObject var assistant = Assistant.shared
    @ObservedObject var dictation = Assistant.shared.dictation
    @ObservedObject var account = OctoberAccount.shared
    @FocusState private var focused: Bool

    var body: some View {
        Group {
            if account.signedIn {
                VStack(alignment: .leading, spacing: 0) {
                HStack(alignment: .bottom, spacing: 6) {
                    TextField(placeholder, text: $assistant.text, axis: .vertical)
                        .textFieldStyle(.plain).font(.system(size: large ? 15 : 13)).lineLimit(large ? 2...6 : 1...6)
                        .focused($focused)
                        .onSubmit { assistant.submit() }
                    Button {
                        if dictation.isActive { dictation.stop() } else { dictation.start(prefix: assistant.text, owner: "assistant") }
                    } label: {
                        Image(systemName: dictation.isRecording ? "mic.fill" : "mic").font(.system(size: 12, weight: .medium))
                            .foregroundStyle(dictation.isRecording ? Theme.red : Theme.muted).frame(width: 22, height: 22)
                    }
                    .buttonStyle(.plain).help(dictation.isRecording ? "Stop listening" : "Speak")
                    Button { assistant.submit() } label: {
                        Image(systemName: "arrow.up").font(.system(size: 11, weight: .bold)).foregroundStyle(.black.opacity(0.85))
                            .frame(width: 22, height: 22)
                            .background(Circle().fill(canSubmit ? Theme.amber : Theme.amber.opacity(0.35)))
                    }
                    .buttonStyle(.plain).disabled(!canSubmit).help("Ask (Return)")
                }
                .padding(.leading, large ? 14 : 11).padding(.trailing, 8).padding(.vertical, large ? 11 : 7)
                .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Color.black.opacity(0.3)))
                .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(focused ? Theme.amber.opacity(0.5) : Theme.stroke))
                if let agent = assistant.activeAgent {
                    Button {
                        assistant.askLanternInstead.toggle()
                        focused = true
                    } label: {
                        Label(assistant.askLanternInstead ? "Reply to @\(agent.handle) instead" : "Ask Lantern instead",
                              systemImage: assistant.askLanternInstead ? "arrowshape.turn.up.left" : "sparkles")
                            .font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
                    }
                    .buttonStyle(.plain)
                    .padding(.leading, 4).padding(.top, 6)
                }
                }
            } else {
                // Sign in right here; afterwards Lantern comes back to this box.
                VStack(alignment: .leading, spacing: 8) {
                    Text("Sign in to ask Lantern").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                    Text("Ask anything about your agents, or tell Lantern what to do. Free accounts get 20 questions a day.")
                        .font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                    AccountCard(account: account, showsHeader: false)
                }
            }
        }
        .padding(.horizontal, 12).padding(.bottom, 12).padding(.top, 4)
        .onChange(of: model.composeFocusToken) { focused = true }
    }

    private var replying: Bool { assistant.activeAgent != nil && !assistant.askLanternInstead }

    private var placeholder: String {
        if dictation.isRecording { return "Listening…" }
        if replying, let agent = assistant.activeAgent { return "Reply to @\(agent.handle)…" }
        return assistant.turns.isEmpty ? "Ask Lantern, or tell it what to do" : "Ask a follow-up"
    }

    private var canSubmit: Bool {
        !assistant.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && (replying || !assistant.asking)
    }
}

/// Lantern's home (click the lantern): Ask Lantern. What's waiting is on the stack button.
struct AssistantHome: View {
    static let ideas: [(symbol: String, text: String)] = [
        ("wifi", "Fix the Wi-Fi."),
        ("video", "Join the call."),
        ("photo", "Find that photo."),
    ]

    @ObservedObject var model: AppModel
    @ObservedObject var assistant = Assistant.shared
    @ObservedObject var account = OctoberAccount.shared
    @State private var showAll = false

    /// Earlier chats: the last three, or all of them (scrolling) after Show all.
    private var recent: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text("Recent").font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.muted)
                Spacer()
                if assistant.conversations.count > 3 {
                    Button(showAll ? "Show less" : "Show all") { showAll.toggle() }.buttonStyle(.plain)
                        .font(.system(size: 11.5, weight: .medium)).foregroundStyle(Theme.muted)
                }
            }
            .padding(.top, 10).padding(.bottom, 2)
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(showAll ? assistant.conversations : Array(assistant.conversations.prefix(3))) { c in
                        Button { assistant.open(c) } label: {
                            HStack(spacing: 8) {
                                Image(systemName: c.turns.contains { $0.task != nil } ? "play.circle" : "bubble.left")
                                    .font(.system(size: 11)).foregroundStyle(Theme.muted).frame(width: 16)
                                Text(c.title).font(.system(size: 12.5)).foregroundStyle(Theme.ink.opacity(0.9)).lineLimit(1)
                                Spacer(minLength: 8)
                                Text(c.updated, format: .relative(presentation: .numeric, unitsStyle: .narrow))
                                    .font(.system(size: 11)).foregroundStyle(Theme.muted)
                            }
                            .padding(.horizontal, 6).padding(.vertical, 7)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .contextMenu { Button("Delete", role: .destructive) { assistant.delete(c) } }
                    }
                }
            }
            .frame(maxHeight: showAll ? 260 : nil)
            .fixedSize(horizontal: false, vertical: !showAll)
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !assistant.turns.isEmpty {
                AssistantThread(model: model)
            } else {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Text("Ask Lantern").font(.system(size: 17, weight: .bold)).foregroundStyle(Theme.ink)
                        Spacer()
                        Button { model.panel = nil } label: {
                            Image(systemName: "xmark").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.muted).frame(width: 22, height: 22)
                        }
                        .buttonStyle(.plain).keyboardShortcut(.cancelAction).help("Close")
                    }
                    if account.signedIn {
                        Text("Ask anything, or tell Lantern what to do and an agent gets on it.")
                            .font(.system(size: 13)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                            .padding(.bottom, 2)
                        ForEach(Self.ideas, id: \.text) { idea in
                            Button { assistant.text = idea.text } label: {
                                HStack(spacing: 10) {
                                    Image(systemName: idea.symbol).font(.system(size: 14, weight: .medium)).foregroundStyle(Theme.amber)
                                        .frame(width: 22)
                                    Text(idea.text).font(.system(size: 14, weight: .medium)).foregroundStyle(Theme.ink.opacity(0.9))
                                    Spacer()
                                }
                                .padding(.horizontal, 12).padding(.vertical, 10)
                                .background(RoundedRectangle(cornerRadius: 11, style: .continuous).fill(Theme.faint))
                            }
                            .buttonStyle(.plain)
                        }
                        if !assistant.conversations.isEmpty { recent }
                    }
                }
                .padding(.horizontal, 16).padding(.top, 16).padding(.bottom, 8)
            }
            AskBox(model: model, large: true, account: account)
        }
        .onAppear { model.composeFocusToken += 1 }
    }
}
