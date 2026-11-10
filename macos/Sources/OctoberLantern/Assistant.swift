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
        /// What the failed answer offers: the trial and Lifetime, your key's settings, or asking
        /// October's AI instead.
        var fix: OctoberAI.Fix?
        /// Your own key answered: its provider ("OpenAI").
        var via: String?
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
        /// Where this task's conversation begins: its own first message in the agent's session (the
        /// send, or the prompt it was started with), once Lantern has found it there. Until then
        /// nothing of the agent's counts as this task's. `transcript` then holds what came after
        /// it, and that message stays the boundary by its id: a later copy of the same words, or
        /// another October Bus nudge, isn't it.
        var start: ChatMessage?
        /// The session (protocol `historyResult.source`) `start` is in: what another session of
        /// the same process says (after `/clear` or `/new`) isn't this task's.
        var source: String?
        /// For one it was sent to: the entries the agent's session (`baselineSource`) already
        /// had when it was sent, when Lantern had read it. A copy of the send without a time
        /// counts as this one only if it's new since then.
        var baseline: [String]?
        var baselineSource: String?
        /// When it was started, and when Lantern first saw its agent: for Send Logs.
        var startedAt: Date?
        var seenAt: Date?
        /// What you sent its agent from here since, into this conversation (the newest few): so the
        /// card can say when one isn't in it (the agent changed conversation as it went in).
        var replies: [Reply]?
    }

    struct Reply: Codable, Equatable {
        let text: String
        let at: Date
        /// Found in the task's conversation once: it stays found after it leaves what's kept.
        var seen: Bool?
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

    /// Saves chats to disk; not in tests.
    private let persisting: Bool

    private convenience init() {
        self.init(persisting: true)
    }

    /// For tests: nothing is read from or saved to disk, and the engine is `seams`.
    convenience init(testing model: AppModel, seams: Seams) {
        self.init(persisting: false)
        self.model = model
        self.seams = seams
    }

    private init(persisting: Bool) {
        self.persisting = persisting
        dictation.onText = { [weak self] text, _ in self?.text = text }
        guard persisting else { return }
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

    /// What was in the box when it was sent. A send that waits (see `submit`) sends this, not
    /// what the box holds by the time it goes.
    struct Draft: Equatable {
        var text: String
        var images: [URL]
    }

    /// A message for the agent waiting on a read of its conversation: what it is, which agent and
    /// chat it's for. Only the newest one, while it's still this chat's, goes anywhere.
    struct Submission: Equatable {
        let id = UUID()
        let draft: Draft
        let agentId: String
        let conversation: UUID
        let link: TaskLink
    }

    /// The engine, for tests: a read made now, a reply typed in, a send routed to an agent.
    struct Seams {
        var readNow: (String, @escaping (HistoryRead?) -> Void) -> Void
        var reply: (String, Agent) -> Void
        var route: (String, Agent, String?, @escaping (Bool, Bool, String?) -> Void) -> Void
    }
    private var seams: Seams?

    private func readNow(_ agentId: String, _ done: @escaping (HistoryRead?) -> Void) {
        if let seams { return seams.readNow(agentId, done) }
        guard let model else { return done(nil) }
        model.readNow(agentId, done)
    }

    /// A send waiting on a read (see `submit`).
    @Published private(set) var submitting: Submission?
    /// Checking which conversation the agent is in before sending.
    var checking: Bool { submitting != nil }
    /// Messages a chat was left with before they went (by chat): back in the box when it's opened.
    private var unsent: [UUID: Draft] = [:]
    /// Changes when the chat does: a routed send's read that comes back after doesn't send.
    private var epoch = 0
    /// Turns whose routed send is waiting on its read.
    private var presending: Set<UUID> = []

    func submit() {
        guard !checking else { return }
        if let agent = activeAgent, !askLanternInstead {
            // What's in the box now is what goes: it's taken out before anything is waited on.
            guard let draft = takeDraft() else { return }
            // Which conversation the agent is in decides where this goes, and the last poll may
            // be from before a `/clear`: a read asked for now settles it. A task not yet placed in
            // a session has nothing to compare, so the reply goes straight in.
            guard let link = turns.last(where: { $0.task != nil })?.task, link.source != nil else {
                return reply(draft, to: agent)
            }
            let submission = Submission(draft: draft, agentId: agent.id, conversation: currentId, link: link)
            submitting = submission
            readNow(agent.id) { [weak self] read in self?.checked(submission, read) }
        } else {
            ask()
            askLanternInstead = false
        }
    }

    /// The read for `submission` is in. Nothing goes if it's been cancelled (a new chat, another
    /// one opened) or the chat's agent isn't that one now; then, or without a read, the message
    /// goes back in the box.
    func checked(_ submission: Submission, _ read: HistoryRead?) {
        guard submitting?.id == submission.id else { return }
        submitting = nil
        guard currentId == submission.conversation, let agent = activeAgent, agent.id == submission.agentId else {
            restore(submission.draft)
            attachProblem = "That message wasn't sent: the agent in this chat changed while Lantern was checking. It's back in the box."
            return
        }
        switch Self.destination(for: submission.link, now: read) {
        case .sameConversation: reply(submission.draft, to: agent)
        case .newConversation: followUp(submission.draft, to: agent, baseline: read)
        case .unknown:
            restore(submission.draft)
            attachProblem = "Lantern couldn't check which conversation @\(agent.handle) is in just now, so nothing was sent. Try again."
        }
    }

    /// Takes the message out of the box (nil when there's none).
    private func takeDraft() -> Draft? {
        let message = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !message.isEmpty || !attachments.isEmpty else { return nil }
        dictation.stop()
        let draft = Draft(text: message, images: attachments)
        text = ""
        attachments = []
        attachProblem = nil
        return draft
    }

    /// Puts a message that didn't go back in the box, before anything typed since.
    private func restore(_ draft: Draft) {
        let typed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        text = typed.isEmpty ? draft.text : draft.text.isEmpty ? typed : draft.text + "\n\n" + typed
        var images = draft.images
        for url in attachments where !images.contains(url) { images.append(url) }
        attachments = Array(images.prefix(ImageAttachments.limit))
    }

    /// The words that go for a message with only images.
    private static func words(_ draft: Draft) -> String {
        if !draft.text.isEmpty { return draft.text }
        return draft.images.count == 1 ? "Take a look at this image." : "Take a look at these images."
    }

    /// A new chat (the current one is kept in the list).
    func clear() {
        task?.cancel()
        cancelTask()
        epoch += 1
        // A message still waiting on its read stays with this chat, not sent and not lost.
        if let waiting = submitting {
            submitting = nil
            unsent[waiting.conversation] = waiting.draft
        }
        for id in presending {
            guard let i = turns.firstIndex(where: { $0.id == id }), turns[i].task?.delivery == nil else { continue }
            turns[i].task?.error = "It wasn't sent: the chat was closed before it went in."
        }
        presending = []
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
        if let draft = unsent.removeValue(forKey: conversation.id) {
            restore(draft)
            attachProblem = "This message wasn't sent before you left the chat. It's back in the box."
        }
    }

    func delete(_ conversation: Conversation) {
        conversations.removeAll { $0.id == conversation.id }
        if conversation.id == currentId { clear() }
        Self.store(conversations)
    }

    // MARK: The agent in the conversation

    /// The running agent from the latest task in this conversation.
    /// Whether `link` is the newest task on its agent (a later one, sent to the same agent, takes
    /// over showing its state and questions).
    func isNewest(_ link: TaskLink) -> Bool {
        guard let id = link.agentId else { return true }
        return turns.last(where: { $0.task?.agentId == id })?.task == link
    }

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
    func track(_ link: TaskLink, agent: Agent?, read: HistoryRead?) {
        guard let i = turns.lastIndex(where: { $0.task == link }) else { return }
        var updated = link
        if let agent, updated.agentId == nil {
            updated.agentId = agent.id
            updated.seenAt = Date()
            let after = link.startedAt.map { " after \(Int(Date().timeIntervalSince($0))) s" } ?? ""
            AppLog.write("Ask Lantern: \(link.kind) task is agent \(agent.id)\(after)")
        }
        if let read { updated = Self.keeping(read, in: updated) }
        if updated != link { turns[i].task = updated }
    }

    /// `link` reconciled with a newer read of its agent's conversation.
    ///
    /// - Until the task's own first message (`start`) shows up, nothing counts (see `startIndex`).
    /// - While it's in the read, what follows it is the whole truth: an entry updated since is its
    ///   new version, and one taken back (a rewind) is gone.
    /// - A complete read without it: it was taken back, and so was everything after it.
    /// - Otherwise the read has moved past it: reconciled with what was kept (`merged`), so an
    ///   answer stays after it leaves the engine's window.
    ///
    /// Another session of the same process (`movedOn`) leaves it as it is.
    static func keeping(_ read: HistoryRead, in link: TaskLink) -> TaskLink {
        if movedOn(link, source: read.source) { return link }
        var link = link
        let messages = read.messages
        guard let start = link.start else {
            guard let i = startIndex(link, in: read) else { return link }
            link.start = messages[i]
            link.source = read.source
            link.transcript = retained(Array(messages[(i + 1)...]))
            return link
        }
        if let i = index(of: start, in: messages) {
            link.transcript = retained(Array(messages[(i + 1)...]))
        } else if read.complete {
            link.transcript = []
        } else {
            let since = link.sentAt ?? link.startedAt
            link.transcript = retained(merged(link.transcript, messages.filter { fresh($0, since: since) != false }))
        }
        if var replies = link.replies, let last = replies.last, last.seen != true, contains(link.transcript, last) {
            replies[replies.count - 1].seen = true
            link.replies = replies
        }
        return link
    }

    /// Whether `source` is another session than the one the task's messages came from.
    static func movedOn(_ link: TaskLink, source: String?) -> Bool {
        guard let bound = link.source, let source else { return false }
        return bound != source
    }

    /// Where the task's first message is in a read that doesn't have it yet: the send (or Lantern's
    /// October Bus nudge for it), or the prompt a started task was given, found by its first
    /// words (so Lantern's notes, images or the engine's shortening don't hide it).
    ///
    /// It must be new: from after the task began (no slack: Lantern notes the time before the
    /// agent can write it), and for a send, not among the entries the session already had when
    /// Lantern read it just before sending (`baseline`). A copy without a time counts only by
    /// that second rule. The engine gives a started agent's history only from a session that
    /// process wrote after it started, so an earlier run's same words aren't in it.
    static func startIndex(_ link: TaskLink, in read: HistoryRead) -> Int? {
        let known = link.baselineSource == read.source ? link.baseline.map(Set.init) : nil
        let new = { (m: ChatMessage) -> Bool in
            if let known, let id = m.id, known.contains(id) { return false }
            if let fresh = fresh(m, since: link.sentAt ?? link.startedAt, slack: 0) { return fresh }
            return known != nil
        }
        if link.sent == true {
            return sendIndex(link.prompt, since: nil, in: read.messages, accept: new)
        }
        let start = String(oneLine(link.prompt).prefix(60))
        guard !start.isEmpty else { return nil }
        return read.messages.firstIndex { m in m.role == .user && oneLine(m.text).contains(start) && new(m) }
    }

    /// Where `start` is in `messages`: by its id; by what it says and when only for an entry
    /// without one (from a reader that can't name entries).
    static func index(of start: ChatMessage, in messages: [ChatMessage]) -> Int? {
        if let id = start.id { return messages.firstIndex { $0.id == id } }
        return messages.firstIndex { $0.id == nil && $0.role == start.role && $0.text == start.text && $0.at == start.at }
    }

    /// Kept messages reconciled with a read that has moved past the task's start. The read is the
    /// session as it is now from its first entry on, so from the first kept entry it holds, it
    /// replaces what was kept: an entry updated since (a tool call filled in) is its new version,
    /// and one taken back is gone. What's older than the read stays.
    ///
    /// A read that holds none of the kept entries either came after them all (the window moved
    /// on) or replaced them (a deep rewind). Times tell which: a kept entry from after the read's
    /// earliest entry, yet not in it, was taken back.
    ///
    /// Entries are the same by id; by what they say and when only without ids (saved before
    /// entries had ids). Two entries with different ids are never the same, however alike.
    static func merged(_ saved: [ChatMessage], _ window: [ChatMessage]) -> [ChatMessage] {
        guard !saved.isEmpty else { return window }
        guard !window.isEmpty else { return saved }
        let ids = Set(window.compactMap(\.id))
        let unnamed = Set(window.filter { $0.id == nil }.map(Self.content))
        let inWindow = { (m: ChatMessage) -> Bool in
            if let id = m.id { return ids.contains(id) }
            return unnamed.contains(Self.content(m))
        }
        if let first = saved.firstIndex(where: inWindow) { return Array(saved[..<first]) + window }
        guard let earliest = window.compactMap({ date($0.at) }).min() else { return saved + window }
        return saved.filter { m in date(m.at).map { $0 <= earliest } ?? true } + window
    }

    private static func content(_ m: ChatMessage) -> String { "\(m.role.rawValue)|\(m.at ?? "")|\(m.text)" }

    /// The newest `limit` messages of the conversation and the newest `limit` tool steps, in
    /// order: steps don't push the agent's answers out of what a task keeps.
    static func retained(_ messages: [ChatMessage], limit: Int = 60) -> [ChatMessage] {
        let talk = messages.indices.filter { messages[$0].role != .tool }.suffix(limit)
        let steps = messages.indices.filter { messages[$0].role == .tool }.suffix(limit)
        let keep = Set(talk).union(steps)
        return messages.indices.filter(keep.contains).map { messages[$0] }
    }

    /// What a task card shows: what it kept, reconciled with `live` when it's read live.
    static func taskMessages(_ link: TaskLink, live: HistoryRead?) -> [ChatMessage] {
        let kept = live.map { keeping($0, in: link) } ?? link
        if kept.start != nil { return kept.transcript }
        // Saved before tasks had a start, and not placed since: the raw tail it kept then.
        if link.sent == true { return after(sent: link.prompt, since: link.sentAt, in: kept.transcript) }
        let all = kept.transcript
        guard let first = all.first, first.role == .user,
              Bubble.withoutLanternNotes(first.text).trimmingCharacters(in: .whitespacesAndNewlines) == link.prompt else { return all }
        return Array(all.dropFirst())
    }

    /// The task's first message hasn't shown up in its agent's conversation well after it began
    /// (Ask first read it too late, or the agent moved on): Lantern can't tell which part is
    /// this task's, so it shows none rather than guess.
    ///
    /// Not while the agent is still working, or a send hasn't gone in yet: a message waiting its
    /// turn isn't missing.
    static func unplaced(_ link: TaskLink, live: HistoryRead?, working: Bool, now: Date = Date()) -> Bool {
        guard link.start == nil, !working, link.sent != true || link.delivery != nil, link.error == nil,
              let live, live.source != nil, !live.messages.isEmpty,
              let began = link.sentAt ?? link.startedAt ?? link.seenAt else { return false }
        return now.timeIntervalSince(began) > 30
    }

    enum Destination: Equatable { case sameConversation, newConversation, unknown }

    /// Where a message for the latest task's agent goes, from a read made just now: into the
    /// task's own conversation, into the one the agent has moved to (as a task of its own), or,
    /// without a read, nowhere yet (the draft stays).
    static func destination(for link: TaskLink, now read: HistoryRead?) -> Destination {
        guard let read else { return .unknown }
        // No session at all now: the task's isn't the one it's in (a `/clear` not written yet).
        guard let source = read.source else { return .newConversation }
        return link.source == nil || link.source == source ? .sameConversation : .newConversation
    }

    /// The latest task's agent is in another conversation now (`/clear`, `/new`) than the one the
    /// task is in, as last read: what you send next goes there, as a task of its own.
    var inNewConversation: Bool {
        guard let link = turns.last(where: { $0.task != nil })?.task, let agent = activeAgent, let model,
              model.watchedAgentId == agent.id else { return false }
        return Self.movedOn(link, source: model.watchedRead?.source)
    }

    /// A follow-up to an agent that has moved to another conversation: sent as a task of its own
    /// (as October's hand-offs are), so its answer shows under it once the send is found there.
    /// The earlier task keeps its own conversation.
    private func followUp(_ draft: Draft, to agent: Agent, baseline: HistoryRead?) {
        let message = Self.words(draft)
        var turn = Turn(question: message)
        turn.answer = "@\(agent.handle) is in a new conversation now, so this goes there."
        if !draft.images.isEmpty { turn.images = draft.images.map(\.path) }
        turns.append(turn)
        send(message, to: agent, turn: turn.id, baseline: baseline)
        Analytics.shared.capture("assistant_task_reply", ["kind": agent.kind.rawValue, "newConversation": true])
    }

    /// Typed into the latest task's conversation. Noted on the task, so its card can say if it
    /// isn't found there (the agent changed conversation as it went in).
    private func reply(_ draft: Draft, to agent: Agent) {
        let message = Self.words(draft)
        if let i = turns.lastIndex(where: { $0.task?.agentId == agent.id }) {
            var replies = turns[i].task?.replies ?? []
            replies.append(Reply(text: message, at: Date()))
            turns[i].task?.replies = Array(replies.suffix(5))
        }
        let text = message + Self.imageLines(draft.images)
        if let seams { seams.reply(text, agent) } else { model?.sendDirect(text, to: agent) }
        Analytics.shared.capture("assistant_task_reply", ["kind": agent.kind.rawValue])
    }

    /// Whether `messages` has `reply` (by its first words, from after it was sent).
    static func contains(_ messages: [ChatMessage], _ reply: Reply) -> Bool {
        let words = String(oneLine(reply.text).prefix(60))
        return messages.contains { m in m.role == .user && oneLine(m.text).contains(words) && fresh(m, since: reply.at) != false }
    }

    /// Your newest message to the task's agent from here that isn't in what the card shows, once
    /// that's clear: the agent is in another conversation now (the message went there, or
    /// `/clear` came between), or it has been idle half a minute since it was sent. Nil while it
    /// could still show up.
    static func missingReply(_ link: TaskLink, shown: [ChatMessage], live: HistoryRead?, working: Bool,
                             now: Date = Date()) -> Reply? {
        guard link.start != nil, let last = link.replies?.last, last.seen != true, !contains(shown, last) else { return nil }
        if let live, movedOn(link, source: live.source) { return last }
        return !working && now.timeIntervalSince(last.at) > 30 ? last : nil
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
        guard persisting else { return }
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
        // October sees what was attached and, for a follow-up, earlier pictures in this
        // conversation (six at most, newest first). One it can't read stops the question.
        let earlier = turns.dropLast().reversed().flatMap { ($0.images ?? []).map(URL.init(fileURLWithPath:)) }
            .filter { FileManager.default.fileExists(atPath: $0.path) }
        let pictures = Array((images + earlier).prefix(6))
        guard let jpegs = ImageAttachments.jpegsForOctober(pictures) else {
            turns.removeLast()
            asking = false
            attachments = images
            attachProblem = "Lantern couldn't read one of the pictures. Remove it and try again."
            return
        }
        var request = OctoberAI.Request(
            turns: turns.dropLast().filter { !$0.failed }.map { ($0.question, $0.answer) },
            question: question, image: jpegs.count == 1 ? jpegs[0] : nil,
            context: agentSummary(question: question, images: images.count), mode: "chat"
        )
        if jpegs.count > 1 { request.images = jpegs }
        run(request, turn: id)
    }

    /// Questions whose own-key provider failed before answering, for "Ask October's AI instead".
    /// Only in memory: a reopened conversation doesn't offer it.
    private var retries: [UUID: OctoberAI.Request] = [:]

    func canAskOctober(_ id: UUID) -> Bool { retries[id] != nil }

    /// Ask October's AI instead: the same question, for the same turn, without your key.
    func askOctober(_ id: UUID) {
        guard !asking, var request = retries.removeValue(forKey: id), let i = turns.firstIndex(where: { $0.id == id }),
              turns[i].answer.isEmpty || turns[i].fix == .useOctober else { return }
        turns[i].answer = ""
        turns[i].failed = false
        turns[i].fix = nil
        turns[i].via = nil
        request.ownKey = false
        asking = true
        Analytics.shared.capture("own_key_fallback", [:])
        run(request, turn: id)
    }

    private func run(_ request: OctoberAI.Request, turn id: UUID) {
        task = Task {
            do {
                for try await piece in OctoberAI.stream(request) {
                    guard !Task.isCancelled, let i = turns.firstIndex(where: { $0.id == id }) else { return }
                    switch piece {
                    case .text(let t): turns[i].answer += t
                    case .via(let p): turns[i].via = p
                    case .note(let n): turns[i].note = n
                    case .task(let prompt, let agent, let project, let to):
                        if let to, let running = model?.agents.first(where: { $0.isLive && $0.handle == to }) {
                            send(prompt, to: running, turn: id)
                        } else {
                            propose(prompt: prompt, agent: agent, project: project, turn: id)
                        }
                    case .guide: break
                    }
                }
                if let i = turns.firstIndex(where: { $0.id == id }), turns[i].answer.isEmpty {
                    turns[i].answer = "No answer came back. Try again."
                    turns[i].failed = true
                }
            } catch is CancellationError {
            } catch {
                if let i = turns.firstIndex(where: { $0.id == id }) {
                    let message = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
                    var fix = (error as? OctoberAI.Failure)?.fix
                    // Never after part of an answer: it could repeat work the answer started.
                    if fix == .useOctober {
                        if turns[i].answer.isEmpty { retries[id] = request } else { fix = nil }
                    }
                    turns[i].answer += (turns[i].answer.isEmpty ? "" : "\n\n") + message
                    turns[i].failed = true
                    turns[i].fix = fix
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
    /// the conversation, so then October's self-contained version is used. When October picked a
    /// recipe for the work, its first paragraph says so, and it stays in front.
    static func taskPrompt(question: String, suggested: String, followUp: Bool) -> String {
        let mine = question.trimmingCharacters(in: .whitespacesAndNewlines)
        if followUp || mine.isEmpty { return suggested }
        if let end = suggested.range(of: "\n\n"), suggested.hasPrefix(recipeLead) {
            return suggested[..<end.lowerBound] + "\n\n" + mine
        }
        return mine
    }

    /// How October's prompt starts when the work has a recipe (see `recipes/` in the repo).
    static let recipeLead = "Follow the recipe at https://lantern.october.dev/recipes/"

    /// Point & Ask decided its question is work: a new conversation here with that question and
    /// answer, starting the task with the selection's screenshot. October's prompt is used as it
    /// is: it says what the screenshot shows, which the user's own words ("move these") don't.
    /// Show me for an answer: Lantern's panel closes so the screen is clear, and Lantern points the
    /// way on the display under the pointer.
    func showMe(_ question: String) {
        model?.panel = nil
        let mouse = NSEvent.mouseLocation
        let screen = NSScreen.screens.first { NSMouseInRect(mouse, $0.frame, false) }
        // Let the panel close before the picture is taken.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { ShowMe.shared.start(question: question, on: screen) }
    }

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
        if let project, project.hasPrefix("/") {
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(atPath: project, isDirectory: &isDirectory), isDirectory.boolValue else {
                return setTaskProblem("The requested project folder does not exist. Create it or choose an existing folder, then try again.", turn: turn)
            }
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
    /// `baseline` is a read of the agent's conversation just made; without one, one is made first.
    private func send(_ prompt: String, to agent: Agent, turn: UUID, baseline: HistoryRead?? = nil) {
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
        // What its session has just before it goes in: an earlier copy of the same words (or an
        // earlier Bus nudge) is one of these, so it isn't taken for this send.
        guard let baseline else {
            // Not sent if the chat changes meanwhile (`clear` says so on its card).
            let epoch = self.epoch
            presending.insert(turn)
            return readNow(agent.id) { [weak self] read in
                guard let self, self.epoch == epoch, self.presending.remove(turn) != nil else { return }
                self.send(prompt, to: agent, turn: turn, baseline: .some(read))
            }
        }
        if let read = baseline {
            link.baseline = read.messages.compactMap(\.id)
            link.baselineSource = read.source
        }
        link.sentAt = Date()
        turns[i].task = link
        let report: (Bool, Bool, String?) -> Void = { [weak self] ok, uncertain, message in
            guard let self, let i = self.turns.firstIndex(where: { $0.id == turn }), self.turns[i].task != nil else { return }
            if ok {
                self.turns[i].task?.delivery = message ?? "Sent"
            } else {
                self.turns[i].task?.error = message ?? (uncertain ? "Lantern isn't sure it went in. Check its terminal." : "It didn't go in.")
            }
        }
        if let seams {
            seams.route(own + Self.imageLines(images), agent, link.sendKey, report)
        } else {
            model.sendFromAssistant(own + Self.imageLines(images), to: agent, key: link.sendKey, report: report)
        }
        Analytics.shared.capture("assistant_sent", ["kind": agent.kind.rawValue, "route": agent.route?.via ?? "none", "bus": agent.busId != nil])
    }

    /// What an agent it was sent to said after it: from its last message that is the one sent, or
    /// Lantern's nudge to read it on October Bus, made after `since` (when it was sent). A nudge
    /// counts only with a time that shows it's this send's: every send's nudge has the same words.
    /// Nothing until that shows up.
    ///
    /// Typed in, the message arrives on one line (the engine turns line breaks into spaces so they
    /// don't press Return), so it's compared with all whitespace runs as single spaces.
    static func after(sent prompt: String, since: Date?, in messages: [ChatMessage]) -> [ChatMessage] {
        guard let i = sendIndex(prompt, since: since, in: messages) else { return [] }
        return Array(messages[(i + 1)...])
    }

    /// Where the send is in `messages` (see `after`). With `accept`, a copy counts only if it says
    /// so, in place of the time check after `since`.
    static func sendIndex(_ prompt: String, since: Date?, in messages: [ChatMessage],
                          accept: ((ChatMessage) -> Bool)? = nil) -> Int? {
        let start = String(oneLine(prompt).prefix(60))
        return messages.lastIndex(where: { m in
            guard m.role == .user else { return false }
            if let accept {
                return accept(m) && ((!start.isEmpty && oneLine(m.text).contains(start)) || m.text.contains("message on October Bus"))
            }
            if !start.isEmpty, oneLine(m.text).contains(start) { return fresh(m, since: since) ?? true }
            return m.text.contains("message on October Bus") && fresh(m, since: since) == true
        })
    }

    /// Whether a message is from after `since`, with a little slack for the two clocks (the
    /// agent's transcript and this app). Nil without both times.
    static func fresh(_ m: ChatMessage, since: Date?, slack: TimeInterval = 2) -> Bool? {
        guard let since, let when = date(m.at) else { return nil }
        return when >= since.addingTimeInterval(-slack)
    }

    /// An entry's time (ISO 8601, with or without fractions of a second).
    static func date(_ at: String?) -> Date? {
        guard let at else { return nil }
        return fractional.date(from: at) ?? whole.date(from: at)
    }

    private static let fractional: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
    private static let whole = ISO8601DateFormatter()

    /// `text` trimmed, with every run of whitespace (line breaks included) as one space.
    static func oneLine(_ text: String) -> String {
        text.split(whereSeparator: { $0.isWhitespace || $0.isNewline }).joined(separator: " ")
    }

    /// Questions answered in the agent's own terminal (an approval, a plan, a choice, a form or
    /// question the engine reads as `other`), not with a message typed in.
    static func answeredInTerminal(_ kind: String?) -> Bool { ["permission", "plan", "choice", "other"].contains(kind ?? "") }

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
            images: p.images.isEmpty ? nil : p.images.map(\.path), startedAt: Date()
        )
        let how = signIn ? "in a Terminal window to sign in" : model.tmuxAvailable ? "in the background" : "in a Terminal window"
        AppLog.write("Ask Lantern: starting \(p.kind.rawValue) \(how)")
        let started = model.launchTask(
            prompt: p.prompt, agent: p.kind, folder: p.folder, background: model.tmuxAvailable && !signIn, screenshot: p.screenshot,
            images: p.images, context: p.context
        ) { [weak self] outcome in
            guard let self, let i = self.turns.firstIndex(where: { $0.id == turn }) else { return }
            switch outcome {
            case .started(let session):
                AppLog.write("Ask Lantern: \(p.kind.rawValue) started, session \(session ?? "none (a Terminal window)")")
                if let session { self.turns[i].task?.session = session }
                self.turns[i].started = "Started \(p.kind.displayName) in \(URL(fileURLWithPath: p.folder).lastPathComponent)"
            case .failed(let message):
                AppLog.write("Ask Lantern: \(p.kind.rawValue) couldn't start: \(message)")
                self.turns[i].task?.error = message
            }
        }
        if !started {
            AppLog.write("Ask Lantern: \(p.kind.rawValue) not started, another session is still starting")
            turns[i].task?.error = "Another session is still starting. Try again in a moment."
        }
        checkSeen(turn: turn)
        Analytics.shared.capture("assistant_task", ["kind": p.kind.rawValue])
    }

    /// A minute after a task started, notes in the log if Lantern still can't see its agent, with
    /// the sessions it can see, so Send Logs shows why the task stayed on "Getting started".
    private func checkSeen(turn: UUID) {
        guard let started = turns.first(where: { $0.id == turn })?.task?.startedAt else { return }
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(60))
            guard let self, let model, let link = turns.first(where: { $0.id == turn })?.task,
                  link.startedAt == started, link.agentId == nil, link.error == nil, !link.inTerminal, agent(for: link) == nil else { return }
            let panes = model.agents.compactMap { a in a.tmux.map { "\(a.id) at \($0.socket.map { "-L \($0) " } ?? "")\($0.target)" } }
            AppLog.write("Ask Lantern: \(link.kind) session \(link.session ?? "unknown") not seen after 60 s; agents in tmux: \(panes.isEmpty ? "none" : panes.joined(separator: ", "))")
        }
    }

    /// The latest tasks, for Send Logs: what each shows and what Lantern knows about it. Never
    /// the message or the folder.
    func reportLines() -> String {
        let all = turns + conversations.flatMap(\.turns).filter { t in !turns.contains { $0.id == t.id } }
        let tasks = all.compactMap(\.task).filter { !($0.sent ?? false) }
            .sorted { ($0.startedAt ?? .distantPast) > ($1.startedAt ?? .distantPast) }.prefix(5)
        guard !tasks.isEmpty else { return "none" }
        let stamp = ISO8601DateFormatter()
        return tasks.map { link in
            var parts = [link.kind, "started \(link.startedAt.map(stamp.string) ?? "?")", "session \(link.session ?? "none")"]
            if let id = link.agentId { parts.append("agent \(id)") }
            if let seen = link.seenAt, let start = link.startedAt { parts.append("seen after \(Int(seen.timeIntervalSince(start))) s") }
            parts.append("shows: \(shown(link))")
            return "- " + parts.joined(separator: ", ")
        }.joined(separator: "\n")
    }

    /// What the conversation shows for a task (TaskView's status, in words).
    private func shown(_ link: TaskLink) -> String {
        let agent = agent(for: link)
        if let error = link.error { return "couldn't start it: \(error)" }
        if link.signIn == true, agent?.state != .working { return "needs sign-in in a Terminal window" }
        if let agent { return "agent \(agent.state)" }
        if link.inTerminal { return "running in a Terminal window" }
        if link.agentId != nil || !link.transcript.isEmpty { return "closed" }
        return "Getting started (no agent seen yet)"
    }

    /// The project you named (matched against recent folders and agents' folders); otherwise
    /// your home folder.
    static func folder(for project: String?, model: AppModel) -> String {
        if let project, project.hasPrefix("/") { return project }
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
                    VStack(alignment: .leading, spacing: 22) {
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
                // The newest message, not the count: past the engine's 120-message window the count
                // stays the same while new replies arrive.
                .onChange(of: model.watchedMessages.last) { proxy.scrollTo("task-bottom", anchor: .bottom) }
            }
        }
        .padding(.horizontal, 16).padding(.top, 14).padding(.bottom, 6)
        // Only the latest task's agent is read live; the rest show what they last said.
        .onChange(of: assistant.activeAgent?.id, initial: true) { _, id in model.watch(id) }
        .onDisappear { model.watch(nil) }
        // Whether Claude Code's approval prompts can reach this conversation (see TaskCard).
        .onAppear { Hooks.shared.refresh() }
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
    @ObservedObject private var hooks = Hooks.shared
    @State private var showingActivity = false

    private var kind: AgentKind { AgentKind(rawValue: link.kind) }
    private var agent: Agent? { assistant.agent(for: link) }

    /// Read live: the latest task, its agent the one being read, and a read in.
    private var read: HistoryRead? {
        guard latest, let agent, agent.id == model.watchedAgentId else { return nil }
        return model.watchedRead
    }

    /// The conversation after your first message (it's the question above).
    private var messages: [ChatMessage] { Assistant.taskMessages(link, live: read) }

    /// Its agent is in another conversation now (`/clear`, `/new`), even one with nothing in it
    /// yet: this card keeps what it had, and what you send next goes there as a task of its own.
    private var movedOn: Bool { read.map { Assistant.movedOn(link, source: $0.source) } ?? false }

    /// The agent's state, question and status belong on this card: it's the newest task on that
    /// agent, in the conversation the agent is in.
    private var current: Bool { !movedOn && assistant.isNewest(link) }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            // Once per render: it reconciles the live read with what the task kept.
            let shown = messages
            agentHeader
            ForEach(ChatView.keyed(shown.filter { $0.role != .tool }), id: \.key) { item in
                Bubble(message: item.message, inAssistant: true)
            }
            // Kept after the agent exits: its questions, progress and result.
            if let launch = model.launch(forAgent: agent?.id ?? link.agentId) {
                LaunchCards(launch: launch, model: model, keys: latest && agent?.isAnswerable != true)
            }
            if current, let agent, agent.isAnswerable {
                QuestionCard(agent: agent, model: model, keys: latest)
            }
            if let missing = Assistant.missingReply(link, shown: shown, live: read, working: agent?.state == .working) {
                note("Your message “\(Assistant.title(missing.text))” isn't in this conversation: @\(agent?.handle ?? "the agent") \(movedOn ? "had moved to a new one as it went in" : "may have taken it somewhere else"), so its answer can't show here. Open agent to see it.", color: Theme.muted)
            } else if movedOn {
                note("@\(agent?.handle ?? "It") is in a new conversation now (cleared, or a new one started), so this shows what it said before. What you send next goes to the new one.", color: Theme.muted)
            } else if Assistant.unplaced(link, live: read, working: agent?.state == .working) {
                note("Lantern can't find this message in @\(agent?.handle ?? "the agent")'s conversation, so it can't tell which part is the answer. Open agent to see it.", color: Theme.muted)
            }
            status
            let activity = shown.filter { $0.role == .tool }
            if !activity.isEmpty {
                DisclosureGroup(isExpanded: $showingActivity) {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(ChatView.keyed(activity), id: \.key) { item in
                            Text(item.message.text).font(.system(size: 11, design: .monospaced))
                                .foregroundStyle(Theme.muted).textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                    .padding(.top, 6)
                } label: {
                    Text("Activity · \(activity.count) step\(activity.count == 1 ? "" : "s")")
                        .font(.system(size: 11)).foregroundStyle(Theme.muted)
                }
                .tint(Theme.muted)
            }
            Color.clear.frame(height: 1).id(latest ? "task-bottom" : "task-\(link.prompt.hashValue)")
        }
        .padding(.vertical, 4)
        .onChange(of: agent?.id, initial: true) { _, _ in assistant.track(link, agent: agent, read: nil) }
        .onChange(of: model.watchedRead) { _, now in
            if latest, let agent, agent.id == model.watchedAgentId { assistant.track(link, agent: agent, read: now) }
        }
    }

    /// One quiet source row; the folder stays in the tooltip.
    private var agentHeader: some View {
        HStack(spacing: 5) {
            if let img = Assets.harness(kind) {
                Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 12, height: 12)
                    .clipShape(RoundedRectangle(cornerRadius: 2.6, style: .continuous))
            }
            Text(link.sent == true ? agent.map { "@\($0.handle)" } ?? kind.displayName : kind.displayName)
            if kind == .october, link.sent != true {
                let tier = OctoberTier.current
                Text("·")
                Text(tier.label).help(tier.explanation)
            }
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
        } else if agent != nil, !current {
            EmptyView()
        } else if let agent {
            // Only what Lantern can see: a finished turn or a closed agent isn't proof the task
            // was done, so nothing here claims it.
            switch agent.state {
            case .working:
                busy("Working on it…")
                // Claude Code's approval prompts reach Lantern only through its hooks: without
                // them a task waiting for approval looks like one still working.
                if kind == .claude, hooks.checked, !hooks.claude {
                    note("If it stops to ask for approval, Lantern can't see that without hooks (Settings). Open agent to check.", color: Theme.muted)
                }
            case .needsInput where !agent.isAnswerable: needsYou(agent)
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

    private func needsYou(_ agent: Agent) -> some View {
        UnansweredQuestion(agent: agent, model: model, elsewhere: "Reply below.")
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
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 0) {
                Spacer(minLength: 28)
                VStack(alignment: .leading, spacing: 6) {
                    Text(turn.question).font(.system(size: 13)).foregroundStyle(Theme.ink)
                        .accessibilityHint("Your message")
                        .textSelection(.enabled)
                        .lineLimit(long && !expanded ? 4 : nil)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contextMenu {
                            Button("Copy message") {
                                NSPasteboard.general.clearContents()
                                NSPasteboard.general.setString(turn.question, forType: .string)
                            }
                        }
                    if long {
                        Button(expanded ? "Show less" : "Show more") { expanded.toggle() }
                            .buttonStyle(.plain).font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.muted)
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
                }
                .padding(12)
                .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Color.white.opacity(0.07)))
            }
            if let via = turn.via { ViaLabel(provider: via) }
            if turn.answer.isEmpty {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Thinking…").font(.system(size: 12)).foregroundStyle(Theme.muted)
                }
            } else {
                Text((try? AttributedString(markdown: turn.answer, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(turn.answer))
                    .font(.system(size: 13)).lineSpacing(3)
                    .accessibilityHint("Lantern's reply")
                    .foregroundStyle(turn.fix != nil ? Theme.amber : turn.failed ? Theme.red : Theme.ink.opacity(0.92))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contextMenu {
                        Button("Copy message") {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(turn.answer, forType: .string)
                        }
                    }
            }
            if let fix = turn.fix {
                FixButtons(fix: fix, from: "assistant", useOctober: fix == .useOctober && Assistant.shared.canAskOctober(turn.id)
                    ? { Assistant.shared.askOctober(turn.id) } : nil)
            }
            if let started = turn.started, turn.task == nil {
                Label(started, systemImage: "play.circle.fill").font(.system(size: 11.5, weight: .medium)).foregroundStyle(Theme.green)
            }
            if let note = turn.note { Text(note).font(.system(size: 11)).foregroundStyle(Theme.muted) }
            if !turn.answer.isEmpty, !turn.failed, turn.task == nil, turn.started == nil, Preferences.shared.showMe {
                ShowMeButton { Assistant.shared.showMe(turn.question) }.padding(.top, 2)
            }
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

/// Start free trial and Buy Lifetime, bought on Stripe for the signed-in account's email: under an
/// answer that hit a plan limit, on the account card and with your own key.
struct OfferButtons: View {
    let from: String

    var body: some View {
        HStack(spacing: 6) {
            button("Start free trial", .trial)
            button("Buy Lifetime", .lifetime)
        }
    }

    private func button(_ title: String, _ offer: OctoberAI.Offer) -> some View {
        Button {
            Analytics.shared.capture("upgrade_opened", ["from": from, "offer": offer.rawValue])
            NSWorkspace.shared.open(OctoberAI.offerURL(offer, email: OctoberAccount.shared.session?.email))
        } label: {
            HStack(spacing: 3) {
                Text(title)
                Image(systemName: "arrow.up.forward").font(.system(size: 9, weight: .semibold))
            }
        }
        .buttonStyle(SecondaryButtonStyle())
        .help("Opens Stripe checkout in your browser")
    }
}

/// What a failed answer offers: the trial and Lifetime, your key's settings, or (only before any
/// answer came back) asking October's AI instead.
struct FixButtons: View {
    let fix: OctoberAI.Fix
    let from: String
    /// Asks the same question again without your key; nil where that isn't possible.
    var useOctober: (() -> Void)?

    var body: some View {
        switch fix {
        case .upgrade: OfferButtons(from: from)
        case .editKey: Button("Edit key") { Self.openKeySettings() }.buttonStyle(SecondaryButtonStyle())
        case .openSettings: Button("Open Settings") { Self.openKeySettings() }.buttonStyle(SecondaryButtonStyle())
        case .useOctober:
            if let useOctober {
                Button("Ask October's AI instead", action: useOctober).buttonStyle(SecondaryButtonStyle())
                    .help("Asks again with October's AI, which uses your October allowance")
            }
        }
    }

    /// Settings › Connections, where your own key is.
    static func openKeySettings() {
        (NSApp.delegate as? AppDelegate)?.showSettings(tab: 2)
    }
}

/// "via OpenAI": your own key answered.
struct ViaLabel: View {
    let provider: String
    var body: some View {
        Text("via \(provider)").font(.system(size: 10.5)).foregroundStyle(Theme.muted)
            .help("Your own \(provider) key answered; \(provider) bills you for it.")
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
                VStack(alignment: .leading, spacing: 10) {
                    TextField(placeholder, text: $assistant.text, axis: .vertical)
                        .textFieldStyle(.plain).font(.system(size: large ? 15 : 13)).lineLimit(large ? 2...6 : 1...6)
                        .focused($focused)
                        .onSubmit { assistant.submit() }
                    toolbar
                }
                }
                .padding(.leading, large ? 14 : 11).padding(.trailing, 8).padding(.vertical, large ? 11 : 7)
                .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(Theme.faint))
                .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(
                    dropping ? Theme.amber : focused ? Theme.amber.opacity(0.5) : Theme.hairline, lineWidth: dropping ? 2 : 1))
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
                    Text("Ask anything about your agents, or tell Lantern what to do.")
                        .font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                    AccountCard(account: account, showsHeader: false)
                }
            }
        }
        .padding(.horizontal, 12).padding(.bottom, 12).padding(.top, 4)
        .onChange(of: model.composeFocusToken) { focused = true }
    }

    private var toolbar: some View {
        HStack(spacing: 6) {
            Button { assistant.pickImages() } label: {
                Image(systemName: "paperclip").font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Theme.muted).frame(width: 28, height: 28)
            }
            .buttonStyle(.plain).help("Add images (you can also paste or drop them here)")
            Button {
                if dictation.isActive { dictation.stop() } else { dictation.start(prefix: assistant.text, owner: "assistant") }
            } label: {
                Image(systemName: dictation.isRecording ? "mic.fill" : "mic").font(.system(size: 12, weight: .medium))
                    .foregroundStyle(dictation.isRecording ? Theme.red : Theme.muted).frame(width: 28, height: 28)
            }
            .buttonStyle(.plain).help(dictation.isRecording ? "Stop listening" : "Speak")
            AIChoiceMenu()
            Spacer(minLength: 8)
            Button { assistant.submit() } label: {
                Image(systemName: "arrow.up").font(.system(size: 11, weight: .bold)).foregroundStyle(.black.opacity(0.85))
                    .frame(width: 28, height: 28)
                    .background(Circle().fill(canSubmit ? Theme.amber : Theme.amber.opacity(0.35)))
            }
            .buttonStyle(.plain).disabled(!canSubmit).help("Ask (Return)")
        }
    }

    private var replying: Bool { assistant.activeAgent != nil && !assistant.askLanternInstead }

    private var placeholder: String {
        if dictation.isRecording { return "Listening…" }
        if assistant.checking, let agent = assistant.activeAgent { return "Checking which conversation @\(agent.handle) is in…" }
        if replying, let agent = assistant.activeAgent {
            return assistant.inNewConversation ? "Message @\(agent.handle)'s new conversation…" : "Reply to @\(agent.handle)…"
        }
        return assistant.turns.isEmpty ? "Ask Lantern, or tell it what to do" : "Ask a follow-up"
    }

    private var canSubmit: Bool {
        (!assistant.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !assistant.attachments.isEmpty)
            && (replying || !assistant.asking) && !assistant.checking
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
                                // Under a minute is "now" (the relative format says "in 0s" for a chat just saved).
                                Group {
                                    if Date().timeIntervalSince(c.updated) < 60 {
                                        Text("now")
                                    } else {
                                        Text(c.updated, format: .relative(presentation: .numeric, unitsStyle: .narrow))
                                    }
                                }
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

/// Pro AI or Lite AI for answers and October agents. A free account sees Pro AI locked, with the
/// way to get it.
struct AIChoiceMenu: View {
    @ObservedObject private var prefs = Preferences.shared
    @ObservedObject private var account = OctoberAccount.shared

    var body: some View {
        let allowed = OctoberTier(plan: account.plan) == .pro
        let tier = OctoberTier.chosen(plan: account.plan, choice: prefs.octoberAI)
        Menu {
            if allowed {
                Button { prefs.octoberAI = "pro" } label: { choice(.pro, tier) }
            } else {
                Button { open(.trial) } label: { Label("Pro AI · start a free trial", systemImage: "lock") }
                Button { open(.lifetime) } label: { Label("Pro AI · buy Lifetime", systemImage: "lock") }
            }
            Button { prefs.octoberAI = "lite" } label: { choice(.lite, tier) }
        } label: {
            HStack(spacing: 3) {
                Text(tier.label)
                Image(systemName: "chevron.down").font(.system(size: 7.5, weight: .bold))
            }
            .font(.system(size: 11, weight: .medium)).foregroundStyle(tier == .pro ? Theme.green : Theme.muted)
            .padding(.horizontal, 8).frame(height: 24)
            .background(Capsule().fill(Theme.faint))
        }
        .menuStyle(.button).buttonStyle(.plain).menuIndicator(.hidden).fixedSize()
        .help(tier.explanation)
        .accessibilityLabel("AI: \(tier.label)")
    }

    private func choice(_ option: OctoberTier, _ current: OctoberTier) -> some View {
        Label(option.menuTitle, systemImage: option == current ? "checkmark" : "")
    }

    private func open(_ offer: OctoberAI.Offer) {
        Analytics.shared.capture("upgrade_opened", ["from": "ai_menu", "offer": offer.rawValue])
        NSWorkspace.shared.open(OctoberAI.offerURL(offer, email: account.session?.email))
    }
}
