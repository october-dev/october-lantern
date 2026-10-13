import SwiftUI

/// Lantern's assistant, opened by clicking the lantern. Two modes:
///
/// - **Ask**: a chat with October's AI (the same endpoint as Point & Ask, without a screenshot).
///   It knows what your agents are doing: each question carries a short summary of them.
/// - **Do**: describe a task and Lantern sets up a New Session for it (the message filled in, the
///   project guessed from your recent folders), for you to check and start.
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

    struct Turn: Identifiable {
        let id = UUID()
        let question: String
        var answer = ""
        var failed = false
        var note: String?
        /// "Started Claude Code in october-lantern", once a task from this answer launched.
        var started: String?
    }

    @Published var text = ""
    @Published private(set) var pending: PendingTask?
    private var countdown: Task<Void, Never>?
    @Published private(set) var turns: [Turn] = []
    @Published private(set) var asking = false
    let dictation = Dictation()
    private var task: Task<Void, Never>?

    private init() {
        dictation.onText = { [weak self] text, _ in self?.text = text }
    }

    func submit() { ask() }

    /// Back to the agent list; the chat is kept until a new question.
    func clear() {
        task?.cancel()
        cancelTask()
        turns = []
        asking = false
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
        // Waits for October Harness to finish installing when that's the agent.
        // In the background (tmux) when possible: the chat opens in Lantern, and Open shows the
        // terminal. Without tmux, a Terminal window.
        _ = model.launchTask(prompt: p.prompt, agent: p.kind, folder: p.folder, background: model.tmuxAvailable)
        if let i = turns.firstIndex(where: { $0.id == turn }) {
            turns[i].started = "Started \(p.kind.displayName) in \(URL(fileURLWithPath: p.folder).lastPathComponent)"
        }
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
                    Label("New question", systemImage: "chevron.left")
                        .font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.muted)
                }
                .buttonStyle(.plain)
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
            }
        }
        .padding(.horizontal, 16).padding(.top, 14).padding(.bottom, 6)
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
            if let started = turn.started {
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
                HStack(alignment: .bottom, spacing: 6) {
                    TextField(dictation.isRecording ? "Listening…" : assistant.turns.isEmpty ? "Ask Lantern, or tell it what to do" : "Ask a follow-up",
                              text: $assistant.text, axis: .vertical)
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
            } else {
                Button { model.panel = .october } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "sparkles").foregroundStyle(Theme.amber)
                        Text("Sign in to October to ask Lantern anything").foregroundStyle(Theme.muted)
                        Spacer()
                        Image(systemName: "chevron.right").foregroundStyle(Theme.muted)
                    }
                    .font(.system(size: 12, weight: .medium))
                    .padding(.horizontal, 11).padding(.vertical, 9)
                    .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Theme.faint))
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12).padding(.bottom, 12).padding(.top, 4)
        .onChange(of: model.composeFocusToken) { focused = true }
    }

    private var canSubmit: Bool {
        !assistant.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !assistant.asking
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
                    }
                }
                .padding(.horizontal, 16).padding(.top, 16).padding(.bottom, 8)
            }
            AskBox(model: model, large: true)
        }
        .onAppear { model.composeFocusToken += 1 }
    }
}
