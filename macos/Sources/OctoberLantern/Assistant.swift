import SwiftUI
import UniformTypeIdentifiers

/// Lantern's assistant, opened by clicking the lantern: a chat with October's AI (the same
/// endpoint as Point & Ask, without a screenshot) that knows what your agents are doing. When the
/// answer asks for work, an agent starts on it in the background and stays in the conversation:
/// its replies stream into the thread and what you type next goes to it. Conversations are kept
/// on this Mac (the most recent 50) and listed on Ask Lantern's home.
@MainActor
final class Assistant: ObservableObject {
    static let shared = Assistant()
    weak var model: AppModel?

    /// A task the answer asked Lantern to start: it starts right away.
    struct PendingTask: Identifiable, Equatable {
        let id = UUID()
        let prompt: String
        let kind: AgentKind
        let folder: String
        /// From Point & Ask: the selection, which the agent gets, and where the user was.
        var screenshot: URL?
        var context: String?
        /// Images you added in the conversation (pasted, dropped or picked).
        var images: [URL] = []
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
        /// Why no agent could be started for this answer's task.
        var taskProblem: String?
        /// Out of free questions or credit: where to upgrade.
        var upgrade: URL?
        /// Images you added to this message (paths in the screenshots folder).
        var images: [String]?
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
        /// Started in a Terminal window because the agent has never been signed in.
        var signIn: Bool?
        /// The images it was given, so Try again gives them again.
        var images: [String]?
        var transcript: [ChatMessage] = []
        /// Sent to an agent that was already running, rather than started for this.
        var sent: Bool?
        /// How it reached that agent ("Sent through October Bus"), once the engine says.
        var delivery: String?
        /// This send's own id, the same when it's tried again (so October Bus keeps one copy), and
        /// when it was first sent (what the agent said before then isn't its answer).
        var sendKey: String?
        var sentAt: Date?
    }

    struct Conversation: Identifiable, Codable {
        var id = UUID()
        var title: String
        var updated: Date
        var turns: [Turn]
    }

    @Published var text = ""
    @Published private(set) var pending: PendingTask?
    @Published private(set) var turns: [Turn] = [] { didSet { if !restoring { saveSoon() } } }
    @Published private(set) var asking = false
    /// Earlier conversations, newest first (the current one included once it has a question).
    @Published private(set) var conversations: [Conversation] = []
    private var currentId = UUID()
    private var restoring = false
    /// With an agent at work in the conversation, the box talks to it; this sends the next
    /// message to Lantern instead.
    @Published var askLanternInstead = false
    /// Images waiting to go with the next message.
    @Published private(set) var attachments: [URL] = []
    /// Why the last image couldn't be added (too many, unreadable).
    @Published var attachProblem: String?
    /// Ask Lantern's box has the keyboard: Command-V with an image on the clipboard adds it.
    var boxFocused = false
    private var pasteMonitor: Any?
    let dictation = Dictation()
    private var task: Task<Void, Never>?
    private var saving: Task<Void, Never>?

    private init() {
        dictation.onText = { [weak self] text, _ in self?.text = text }
        conversations = Self.load()
        // The text field's own paste ignores pictures: an image on the clipboard is added here.
        pasteMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            guard event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,
                  event.charactersIgnoringModifiers == "v" else { return event }
            let handled = MainActor.assumeIsolated { () -> Bool in
                guard Assistant.shared.boxFocused, let images = ImageAttachments.fromPasteboard() else { return false }
                Assistant.shared.attach(images)
                return true
            }
            return handled ? nil : event
        }
    }

    // MARK: Images

    /// Adds saved images to the next message (up to `ImageAttachments.limit`).
    func attach(_ images: [URL]) {
        attachProblem = nil
        guard !images.isEmpty else {
            attachProblem = "That image couldn't be added. Try saving it as a PNG or JPEG first."
            return
        }
        let room = ImageAttachments.limit - attachments.count
        if images.count > room { attachProblem = "Up to \(ImageAttachments.limit) images on one message." }
        attachments += images.prefix(max(room, 0))
    }

    /// Adds image files (dropped or picked).
    func attach(files: [URL]) {
        let images = files.filter(ImageAttachments.isImage)
        guard !images.isEmpty else {
            attachProblem = "Only images can be added here."
            return
        }
        attach(images.compactMap(ImageAttachments.save(file:)))
    }

    func removeAttachment(_ url: URL) {
        attachments.removeAll { $0 == url }
        attachProblem = nil
    }

    /// Choose images with the Open dialog.
    func pickImages() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.allowsMultipleSelection = true
        panel.message = "Choose images to add"
        // Above Lantern's panel, which floats over other windows.
        panel.level = NSWindow.Level(rawValue: NSWindow.Level.statusBar.rawValue + 1)
        NSApp.activate()
        guard panel.runModal() == .OK else { return }
        attach(files: panel.urls)
    }

    /// The images added anywhere in this conversation, for a task it starts (the request may come
    /// in a follow-up: "yes, do that").
    private var conversationImages: [URL] {
        var seen: [URL] = []
        for path in turns.flatMap({ $0.images ?? [] }) {
            let url = URL(fileURLWithPath: path)
            if !seen.contains(url), FileManager.default.fileExists(atPath: path) { seen.append(url) }
        }
        return Array(seen.suffix(10))
    }

    /// The lines that tell an agent where the images are.
    static func imageLines(_ images: [URL]) -> String {
        switch images.count {
        case 0: ""
        case 1: "\n\nAn image I added (open it to see it): \(images[0].path)"
        default: "\n\nImages I added (open them to see them):\n" + images.map { "- \($0.path)" }.joined(separator: "\n")
        }
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
        attachments = []
        attachProblem = nil
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
        var message = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !message.isEmpty || !attachments.isEmpty, let model else { return }
        if message.isEmpty { message = attachments.count == 1 ? "Take a look at this image." : "Take a look at these images." }
        dictation.stop()
        model.sendDirect(message + Self.imageLines(attachments), to: agent)
        text = ""
        attachments = []
        attachProblem = nil
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

    /// How the saved conversations loaded, for Send Logs.
    private(set) static var loadNote = "none saved yet"
    /// The file is there but couldn't be read: it isn't written over until it can be.
    private static var unreadable = false

    /// One conversation that can't be read is left out (and the file kept as a copy), rather
    /// than losing the whole list the next time it's saved.
    private static func load() -> [Conversation] {
        guard FileManager.default.fileExists(atPath: file.path) else { return [] }
        guard let data = try? Data(contentsOf: file) else {
            unreadable = true
            loadNote = "the file couldn't be opened"
            AppLog.write("Ask Lantern: saved chats couldn't be opened; not saving over them")
            return []
        }
        unreadable = false
        switch decode(data) {
        case .success(let read):
            loadNote = "\(read.loaded.count) loaded"
            if read.skipped > 0 {
                let copy = keepCopy()
                loadNote += ", \(read.skipped) couldn't be read (kept in \(copy))"
                AppLog.write("Ask Lantern: \(read.skipped) saved chats couldn't be read; the file is kept as \(copy). First problem: \(read.problem ?? "?")")
            }
            return read.loaded
        case .failure(let error):
            let copy = keepCopy()
            loadNote = "none could be read (kept in \(copy))"
            AppLog.write("Ask Lantern: saved chats couldn't be read (\(error)); the file is kept as \(copy)")
            return []
        }
    }

    /// The saved conversations in `data`, without any that can't be read: how many those were,
    /// and why the first couldn't be. A failure when it isn't a list of conversations at all.
    static func decode(_ data: Data) -> Result<(loaded: [Conversation], skipped: Int, problem: String?), Error> {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        do {
            let all = try decoder.decode([Lossy<Conversation>].self, from: data)
            let loaded = all.compactMap(\.value)
            return .success((loaded, all.count - loaded.count, all.compactMap(\.problem).first))
        } catch {
            return .failure(error)
        }
    }

    /// A copy of the saved chats as they are, before they're next saved over.
    private static func keepCopy() -> String {
        let stamp = DateFormatter()
        stamp.dateFormat = "yyyyMMdd-HHmmss"
        let copy = file.deletingLastPathComponent().appendingPathComponent("ask-lantern-unreadable-\(stamp.string(from: Date())).json")
        try? FileManager.default.copyItem(at: file, to: copy)
        return copy.lastPathComponent
    }

    private static func store(_ conversations: [Conversation]) {
        if unreadable {
            // Try again: the chats on disk come first, then they're saved with the new ones.
            let earlier = load()
            if unreadable { return }
            let merged = conversations + earlier.filter { old in !conversations.contains { $0.id == old.id } }
            Assistant.shared.conversations = Array(merged.prefix(50))
            return store(Assistant.shared.conversations)
        }
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

    /// One saved conversation, or why it couldn't be read.
    private struct Lossy<T: Decodable>: Decodable {
        let value: T?
        let problem: String?
        init(from decoder: Decoder) {
            do { value = try T(from: decoder); problem = nil } catch { value = nil; problem = "\(error)" }
        }
    }

    static func title(_ question: String) -> String {
        let line = question.split(whereSeparator: \.isNewline).first.map(String.init) ?? question
        return line.count > 60 ? String(line.prefix(59)) + "…" : line
    }

    private func ask() {
        var question = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let images = attachments
        guard !question.isEmpty || !images.isEmpty, !asking, OctoberAccount.shared.signedIn else { return }
        if question.isEmpty { question = images.count == 1 ? "Take a look at this image." : "Take a look at these images." }
        dictation.stop()
        text = ""
        attachments = []
        attachProblem = nil
        var turn = Turn(question: question)
        if !images.isEmpty { turn.images = images.map(\.path) }
        turns.append(turn)
        asking = true
        let id = turn.id
        // October sees the first image; an agent this starts gets them all.
        let request = OctoberAI.Request(
            turns: turns.dropLast().filter { !$0.failed }.map { ($0.question, $0.answer) },
            question: question, image: images.first.flatMap(ImageAttachments.jpegForOctober),
            context: agentSummary(question: question, images: images.count), mode: "chat"
        )
        task = Task {
            do {
                for try await piece in OctoberAI.stream(request) {
                    guard !Task.isCancelled, let i = turns.firstIndex(where: { $0.id == id }) else { return }
                    switch piece {
                    case .text(let t): turns[i].answer += t
                    case .note(let n): turns[i].note = n
                    case .task(let prompt, let agent, let project, let to):
                        if let to, let running = model?.agents.first(where: { $0.isLive && $0.handle == to }) {
                            send(prompt, to: running, turn: id)
                        } else {
                            propose(prompt: prompt, agent: agent, project: project, turn: id)
                        }
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
                    turns[i].upgrade = (error as? OctoberAI.Failure)?.upgradeURL
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

    /// Point & Ask decided its question is work: a new conversation here with that question and
    /// answer, starting the task with the selection's screenshot. October's prompt is used as it
    /// is: it says what the screenshot shows, which the user's own words ("move these") don't.
    func adopt(question: String, answer: String, prompt: String, agent: String?, project: String?, screenshot: URL?, context: String?) {
        clear()
        let turn = Turn(question: question, answer: answer)
        turns = [turn]
        propose(prompt: prompt, agent: agent, project: project, turn: turn.id, screenshot: screenshot, context: context)
    }

    /// The answer asked for work to be done: start it now.
    private func propose(
        prompt: String, agent: String?, project: String?, turn: UUID, screenshot: URL? = nil, context: String? = nil
    ) {
        guard let model else { return }
        guard let installed = model.installedKinds else {
            // Lantern is still finding out which agents this Mac has (just after it starts).
            return waitForAgents { [weak self] in
                self?.propose(prompt: prompt, agent: agent, project: project, turn: turn, screenshot: screenshot, context: context)
            } failed: { [weak self] in
                self?.setTaskProblem("Lantern couldn't check which agents this Mac has, so nothing was started. Try again in a moment.", turn: turn)
            }
        }
        let named = agent.flatMap { AgentKind(rawValue: $0) }.flatMap { installed.contains($0) ? $0 : nil }
        guard let kind = named ?? model.defaultAgentKind else {
            model.refreshInstalled()
            return setTaskProblem("Lantern has no agent to do this with. Set one up in New Session, then try again.", turn: turn)
        }
        let folder = Self.folder(for: project, model: model)
        let index = turns.firstIndex { $0.id == turn }
        let own = screenshot != nil ? prompt
            : index.map { Self.taskPrompt(question: turns[$0].question, suggested: prompt, followUp: $0 > 0) } ?? prompt
        cancelTask()
        pending = PendingTask(prompt: own, kind: kind, folder: folder, screenshot: screenshot, context: context, images: conversationImages)
        startPending(turn: turn)
    }

    func cancelTask() {
        pending = nil
    }

    /// Hands the work to an agent that's already running (October named it): through October Bus,
    /// or typed in. Its card joins the conversation, as for an agent the conversation started.
    private func send(_ prompt: String, to agent: Agent, turn: UUID) {
        guard let model, let i = turns.firstIndex(where: { $0.id == turn }) else { return }
        // October's message for that agent, not your words: those were to Lantern ("tell claude-1
        // to …"), and the agent would take them as something to pass on.
        let own = prompt
        let images = (turns[i].images ?? []).map(URL.init(fileURLWithPath:))
        var link = TaskLink(kind: agent.kind.rawValue, folder: agent.cwd ?? NSHomeDirectory(), prompt: own, agentId: agent.id)
        link.sent = true
        link.images = turns[i].images
        link.sendKey = UUID().uuidString
        link.sentAt = Date()
        turns[i].task = link
        turns[i].started = "Sent to @\(agent.handle)"
        model.sendFromAssistant(own + Self.imageLines(images), to: agent, key: link.sendKey) { [weak self] ok, uncertain, message in
            guard let self, let i = self.turns.firstIndex(where: { $0.id == turn }), self.turns[i].task != nil else { return }
            if ok {
                self.turns[i].task?.delivery = message ?? "Sent"
            } else {
                self.turns[i].task?.error = message ?? (uncertain ? "Lantern isn't sure it went in. Check its terminal." : "It didn't go in.")
            }
        }
        Analytics.shared.capture("assistant_sent", ["kind": agent.kind.rawValue, "route": agent.route?.via ?? "none", "bus": agent.busId != nil])
    }

    /// What an agent it was sent to said after it: from its last message that is the one sent, or
    /// Lantern's nudge to read it on October Bus, made after `since` (when it was sent). A nudge
    /// counts only with a time that shows it's this send's: every send's nudge has the same words.
    /// Nothing until that shows up.
    static func after(sent prompt: String, since: Date?, in messages: [ChatMessage]) -> [ChatMessage] {
        let start = String(prompt.trimmingCharacters(in: .whitespacesAndNewlines).prefix(60))
        func at(_ m: ChatMessage) -> Date? {
            guard let s = m.at else { return nil }
            let f = ISO8601DateFormatter()
            f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            return f.date(from: s) ?? ISO8601DateFormatter().date(from: s)
        }
        // A little slack for the two clocks (the agent's transcript and this app).
        let fresh = { (m: ChatMessage) -> Bool? in
            guard let since, let when = at(m) else { return nil }
            return when >= since.addingTimeInterval(-2)
        }
        guard let i = messages.lastIndex(where: { m in
            guard m.role == .user else { return false }
            if m.text.contains(start) { return fresh(m) ?? true }
            return m.text.contains("message on October Bus") && fresh(m) == true
        }) else { return [] }
        return Array(messages[(i + 1)...])
    }

    /// Sends again a message that didn't reach its agent.
    private func resend(_ link: TaskLink, turn: UUID) {
        guard let agent = agent(for: link), agent.isLive else {
            if let i = turns.firstIndex(where: { $0.id == turn }) { turns[i].task?.error = "that agent isn't running anymore." }
            return
        }
        guard let model, let i = turns.firstIndex(where: { $0.id == turn }) else { return }
        turns[i].task?.error = nil
        let images = (link.images ?? []).map(URL.init(fileURLWithPath:))
        model.sendFromAssistant(link.prompt + Self.imageLines(images), to: agent, key: link.sendKey) { [weak self] ok, uncertain, message in
            guard let self, let i = self.turns.firstIndex(where: { $0.id == turn }) else { return }
            if ok { self.turns[i].task?.delivery = message ?? "Sent" } else { self.turns[i].task?.error = message ?? "It didn't go in." }
        }
    }

    /// Runs `then` once Lantern knows which agents are installed (within 20 seconds), else `failed`.
    private func waitForAgents(then: @escaping () -> Void, failed: @escaping () -> Void) {
        model?.refreshInstalled()
        Task { @MainActor [weak self] in
            for _ in 0..<40 {
                try? await Task.sleep(for: .milliseconds(500))
                if self?.model?.installedKinds != nil { return then() }
            }
            failed()
        }
    }

    private func setTaskProblem(_ problem: String, turn: UUID) {
        guard let i = turns.firstIndex(where: { $0.id == turn }) else { return }
        turns[i].taskProblem = problem
    }

    /// Starts a task that couldn't start again, with the same agent, folder and message.
    func retry(_ link: TaskLink) {
        guard let i = turns.firstIndex(where: { $0.task == link }) else { return }
        if link.sent == true { return resend(link, turn: turns[i].id) }
        turns[i].task = nil
        pending = PendingTask(
            prompt: link.prompt, kind: AgentKind(rawValue: link.kind), folder: link.folder, screenshot: nil, context: nil,
            images: (link.images ?? []).map(URL.init(fileURLWithPath:)).filter { FileManager.default.fileExists(atPath: $0.path) }
        )
        startPending(turn: turns[i].id)
    }

    private func startPending(turn: UUID) {
        guard let p = pending, let model else { return }
        pending = nil
        guard let i = turns.firstIndex(where: { $0.id == turn }) else { return }
        // In the background (tmux) when possible, shown right here in the conversation; Open
        // shows its terminal. Without tmux, a Terminal window. An agent never signed in also gets
        // a Terminal window, where it asks you to sign in. Waits for October Harness to finish
        // installing when that's the agent.
        let signIn = model.neverSetUpKinds.contains(p.kind)
        turns[i].taskProblem = nil
        turns[i].task = TaskLink(
            kind: p.kind.rawValue, folder: p.folder, prompt: p.prompt, inTerminal: !model.tmuxAvailable, signIn: signIn ? true : nil,
            images: p.images.isEmpty ? nil : p.images.map(\.path)
        )
        let started = model.launchTask(
            prompt: p.prompt, agent: p.kind, folder: p.folder, background: model.tmuxAvailable && !signIn, screenshot: p.screenshot,
            images: p.images, context: p.context
        ) { [weak self] outcome in
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
    private func agentSummary(question: String, images: Int = 0) -> String {
        guard let model else { return "" }
        let chat = switch images {
        case 0: "Lantern's assistant chat (no screenshot)."
        case 1: "Lantern's assistant chat. The user added the image attached (an agent started for this gets it too)."
        default: "Lantern's assistant chat. The user added \(images) images; the first is attached (an agent started for this gets them all)."
        }
        let live = model.agents.filter(\.isLive)
        guard !live.isEmpty else { return "\(chat) No coding agents are running on this Mac." }
        let agents = Self.summaryOrder(live, question: question)
        var lines = ["\(chat) The user's coding agents on this Mac right now:"]
        for a in agents.prefix(20) {
            var line = "- @\(a.handle) (\(a.kind.displayName)) in \(a.project ?? "?"): \(a.state.label)"
            if let t = a.title { line += "; working on \"\(t.prefix(80))\"" }
            if let q = a.question, a.state.wantsYou { line += "; asking: \(q.prefix(160))" }
            if let m = a.lastMessage { line += "; last said: \(m.replacingOccurrences(of: "\n", with: " ").prefix(200))" }
            lines.append(line)
        }
        if agents.count > 20 { lines.append("(\(agents.count - 20) more running, not listed.)") }
        return String(lines.joined(separator: "\n").prefix(6000))
    }

    /// Which agents October hears about first (it gets 20): any the question names ("claude-35",
    /// "@codex-2"), then those that need you, then the most recently active.
    static func summaryOrder(_ agents: [Agent], question: String) -> [Agent] {
        let words = Set(question.lowercased().split { !($0.isLetter || $0.isNumber || $0 == "-") }.map(String.init))
        func rank(_ a: Agent) -> Int {
            if words.contains(a.handle.lowercased()) { return 0 }
            return a.state.wantsYou ? 1 : a.state == .working ? 2 : 3
        }
        return agents.sorted { (rank($0), -($0.stateSince ?? 0)) < (rank($1), -($1.stateSince ?? 0)) }
    }

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

}

/// An agent Ask Lantern started, inside the conversation: what it has said (live for the latest
/// task, as last seen for earlier ones) and how it's going in plain words. Which agent it is, and
/// where, stays in a small footer with Open agent for anyone who wants the full session.
private struct TaskCard: View {
    let link: Assistant.TaskLink
    let latest: Bool
    @ObservedObject var model: AppModel
    @ObservedObject var assistant = Assistant.shared

    private var kind: AgentKind { AgentKind(rawValue: link.kind) }
    private var agent: Agent? { assistant.agent(for: link) }

    /// The conversation without your first message (it's the question above). For an agent it was
    /// sent to, only what came after it.
    private var messages: [ChatMessage] {
        let live = latest && agent != nil && agent?.id == model.watchedAgentId && !model.watchedMessages.isEmpty
        let all = live ? model.watchedMessages : link.transcript
        if link.sent == true { return Assistant.after(sent: link.prompt, since: link.sentAt, in: all) }
        guard let first = all.first, first.role == .user,
              Bubble.withoutLanternNotes(first.text).trimmingCharacters(in: .whitespacesAndNewlines) == link.prompt else { return all }
        return Array(all.dropFirst())
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(ChatView.keyed(messages), id: \.key) { item in Bubble(message: item.message) }
            if let agent, agent.isAnswerable {
                QuestionCard(agent: agent, model: model, keys: latest)
            }
            status
            footer
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

    /// Which agent and Open agent, small, under the conversation; the folder is in the tooltip.
    private var footer: some View {
        HStack(spacing: 5) {
            if let img = Assets.harness(kind) {
                Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 12, height: 12)
                    .clipShape(RoundedRectangle(cornerRadius: 2.6, style: .continuous))
            }
            Text(link.sent == true ? agent.map { "@\($0.handle)" } ?? kind.displayName : kind.displayName)
            if link.sent == true {
                Text("·")
                Text(link.delivery ?? "Sending…")
            }
            if let agent {
                Text("·")
                Button { model.open(agent) } label: {
                    HStack(spacing: 2) {
                        Text("Open agent")
                        Image(systemName: "arrow.up.forward").font(.system(size: 8.5, weight: .semibold))
                    }
                }
                .buttonStyle(.plain).help("Open @\(agent.handle)'s full session")
            }
            Spacer(minLength: 0)
        }
        .font(.system(size: 10.5)).foregroundStyle(Theme.muted)
        .help("\(kind.displayName) in \(folderName)")
    }

    @ViewBuilder private var status: some View {
        if let error = link.error {
            VStack(alignment: .leading, spacing: 5) {
                Text(link.sent == true ? "Couldn't send it: \(error)" : "Couldn't start it: \(error)").font(.system(size: 11.5)).foregroundStyle(Theme.red)
                    .fixedSize(horizontal: false, vertical: true)
                if latest {
                    Button("Try again") { assistant.retry(link) }
                        .buttonStyle(.plain).font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.ink)
                }
            }
        } else if link.signIn == true, agent?.state != .working {
            note("\(kind.displayName) needs you to sign in first. It's open in a Terminal window: sign in there and it starts on this.", color: Theme.amber)
        } else if let agent {
            // Only what Lantern can see: a finished turn or a closed agent isn't proof the task
            // was done, so nothing here claims it.
            switch agent.state {
            case .working: busy("Working on it…")
            case .needsInput where !agent.isAnswerable: note("Needs your answer. Reply below.", color: Theme.amber)
            case .needsInput: EmptyView()
            case .waiting: note("Finished its turn. Reply below if there's more.", color: Theme.muted)
            case .idle: note("Open, but not working on anything.", color: Theme.muted)
            case .unknown: note("Running.", color: Theme.muted)
            }
        } else if link.inTerminal {
            note("Running in a Terminal window.", color: Theme.muted)
        } else if link.agentId != nil || !link.transcript.isEmpty {
            note("Closed. It isn't running anymore.", color: Theme.muted)
        } else if latest, kind == .october, model.harness?.status == "installing" {
            busy("Setting up October. The first time takes a few minutes…")
        } else if latest {
            busy("Getting started…")
        }
    }

    private func busy(_ text: String) -> some View {
        HStack(spacing: 6) {
            ProgressView().controlSize(.mini)
            Text(text).font(.system(size: 11)).foregroundStyle(Theme.muted)
        }
    }

    private func note(_ text: String, color: Color) -> some View {
        Text(text).font(.system(size: 11)).foregroundStyle(color).fixedSize(horizontal: false, vertical: true)
    }

    private var folderName: String {
        link.folder == NSHomeDirectory() ? "Home folder" : URL(fileURLWithPath: link.folder).lastPathComponent
    }
}

private struct TurnView: View {
    let turn: Assistant.Turn
    @State private var expanded = false

    /// A long message (a pasted prompt) shows its first lines in normal weight, with Show more.
    private var long: Bool { turn.question.count > 180 || turn.question.split(separator: "\n").count > 3 }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if long {
                VStack(alignment: .leading, spacing: 3) {
                    Text(turn.question).font(.system(size: 12.5)).foregroundStyle(Theme.ink.opacity(0.85))
                        .lineLimit(expanded ? nil : 4)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Button(expanded ? "Show less" : "Show more") { expanded.toggle() }
                        .buttonStyle(.plain).font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.muted)
                }
            } else {
                Text(turn.question).font(.system(size: 12.5, weight: .semibold)).foregroundStyle(Theme.ink)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let images = turn.images, !images.isEmpty {
                HStack(spacing: 6) {
                    ForEach(images, id: \.self) { path in
                        Button { NSWorkspace.shared.open(URL(fileURLWithPath: path)) } label: {
                            Thumbnail(url: URL(fileURLWithPath: path), size: 40)
                        }
                        .buttonStyle(.plain).help("Open this image")
                    }
                }
            }
            if turn.answer.isEmpty {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Thinking…").font(.system(size: 12)).foregroundStyle(Theme.muted)
                }
            } else {
                Text((try? AttributedString(markdown: turn.answer, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(turn.answer))
                    .font(.system(size: 12.5)).lineSpacing(2)
                    .foregroundStyle(turn.upgrade != nil ? Theme.amber : turn.failed ? Theme.red : Theme.ink.opacity(0.92))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let url = turn.upgrade { UpgradeButton(url: url, from: "assistant") }
            if let started = turn.started, turn.task == nil {
                Label(started, systemImage: "play.circle.fill").font(.system(size: 11.5, weight: .medium)).foregroundStyle(Theme.green)
            }
            if let note = turn.note { Text(note).font(.system(size: 11)).foregroundStyle(Theme.muted) }
            if let problem = turn.taskProblem {
                Text(problem).font(.system(size: 11.5)).foregroundStyle(Theme.amber).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// A small square preview of an image you added (blank once the file has been cleared away).
struct Thumbnail: View {
    let url: URL
    let size: CGFloat
    /// Small copies, made once: the conversation redraws while an agent works.
    private static let cache = NSCache<NSURL, NSImage>()

    private var image: NSImage? {
        if let hit = Self.cache.object(forKey: url as NSURL) { return hit }
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let small = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                  kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: 160,
                  kCGImageSourceCreateThumbnailWithTransform: true,
              ] as CFDictionary) else { return nil }
        let image = NSImage(cgImage: small, size: .zero)
        Self.cache.setObject(image, forKey: url as NSURL)
        return image
    }

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image).resizable().aspectRatio(contentMode: .fill)
            } else {
                Image(systemName: "photo").font(.system(size: size * 0.35)).foregroundStyle(Theme.muted)
            }
        }
        .frame(width: size, height: size)
        .background(Theme.faint)
        .clipShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 7, style: .continuous).strokeBorder(Theme.hairline))
    }
}

/// Opens October's pricing page, under an answer that hit a plan limit.
struct UpgradeButton: View {
    let url: URL
    let from: String

    var body: some View {
        Button {
            Analytics.shared.capture("upgrade_opened", ["from": from])
            NSWorkspace.shared.open(url)
        } label: {
            HStack(spacing: 3) {
                Text("Upgrade to October Pro")
                Image(systemName: "arrow.up.forward").font(.system(size: 9, weight: .semibold))
            }
        }
        .buttonStyle(SecondaryButtonStyle())
        .help("Opens October's plans in your browser")
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
    @State private var dropping = false

    /// Images dropped on the box: files from Finder, or a picture dragged from a browser.
    private func drop(_ providers: [NSItemProvider]) -> Bool {
        var handled = false
        for provider in providers {
            if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
                handled = true
                _ = provider.loadObject(ofClass: URL.self) { url, _ in
                    guard let url else { return }
                    Task { @MainActor in Assistant.shared.attach(files: [url]) }
                }
            } else if provider.canLoadObject(ofClass: NSImage.self) {
                handled = true
                _ = provider.loadObject(ofClass: NSImage.self) { image, _ in
                    guard let image = image as? NSImage else { return }
                    Task { @MainActor in Assistant.shared.attach(ImageAttachments.save(image).map { [$0] } ?? []) }
                }
            }
        }
        return handled
    }

    var body: some View {
        Group {
            if account.signedIn {
                VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: 8) {
                if !assistant.attachments.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 6) {
                            ForEach(assistant.attachments, id: \.self) { url in
                                Thumbnail(url: url, size: 46).overlay(alignment: .topTrailing) {
                                    Button { assistant.removeAttachment(url) } label: {
                                        Image(systemName: "xmark.circle.fill").font(.system(size: 13)).foregroundStyle(.white, .black.opacity(0.7))
                                    }
                                    .buttonStyle(.plain).help("Remove this image").offset(x: 4, y: -4)
                                }
                            }
                        }
                        .padding(.top, 4).padding(.trailing, 4)
                    }
                }
                HStack(alignment: .bottom, spacing: 6) {
                    TextField(placeholder, text: $assistant.text, axis: .vertical)
                        .textFieldStyle(.plain).font(.system(size: large ? 15 : 13)).lineLimit(large ? 2...6 : 1...6)
                        .focused($focused)
                        .onSubmit { assistant.submit() }
                    Button { assistant.pickImages() } label: {
                        Image(systemName: "paperclip").font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.muted)
                            .frame(width: 22, height: 22)
                    }
                    .buttonStyle(.plain).help("Add images (you can also paste or drop them here)")
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
                }
                .padding(.leading, large ? 14 : 11).padding(.trailing, 8).padding(.vertical, large ? 11 : 7)
                .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Color.black.opacity(0.3)))
                .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(
                    dropping ? Theme.amber : focused ? Theme.amber.opacity(0.5) : Theme.stroke, lineWidth: dropping ? 2 : 1))
                .onDrop(of: [.fileURL, .image], isTargeted: $dropping) { providers in drop(providers) }
                .onChange(of: focused, initial: true) { _, now in assistant.boxFocused = now }
                .onDisappear { assistant.boxFocused = false }
                if let problem = assistant.attachProblem {
                    Text(problem).font(.system(size: 11)).foregroundStyle(Theme.amber).padding(.top, 5)
                }
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
        (!assistant.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !assistant.attachments.isEmpty)
            && (replying || !assistant.asking)
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
