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

    enum Mode: String, CaseIterable, Identifiable {
        case ask, work
        var id: String { rawValue }
        var label: String { self == .ask ? "Ask" : "Do" }
    }

    struct Turn: Identifiable {
        let id = UUID()
        let question: String
        var answer = ""
        var failed = false
        var note: String?
    }

    @Published var mode: Mode = .ask
    @Published var text = ""
    @Published private(set) var turns: [Turn] = []
    @Published private(set) var asking = false
    let dictation = Dictation()
    private var task: Task<Void, Never>?

    private init() {
        dictation.onText = { [weak self] text, _ in self?.text = text }
    }

    func submit() {
        switch mode {
        case .ask: ask()
        case .work: work()
        }
    }

    func clear() {
        task?.cancel()
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

    /// Opens New Session with the task as its message and a project guessed from the words.
    private func work() {
        let request = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let model, !request.isEmpty else { return }
        dictation.stop()
        text = ""
        model.startNew(prompt: request, folder: Self.guessFolder(request, recent: model.recentFolders))
        Analytics.shared.capture("assistant_task", [:])
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
struct AssistantView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var assistant = Assistant.shared
    @ObservedObject var dictation = Assistant.shared.dictation
    @ObservedObject var account = OctoberAccount.shared
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            header
            if !model.inbox.isEmpty { waitingStrip }
            if assistant.mode == .ask && !account.signedIn {
                signIn
            } else if assistant.mode == .ask && !assistant.turns.isEmpty {
                conversation
            } else {
                suggestions
            }
            input
        }
        .padding(.horizontal, 16).padding(.top, 14).padding(.bottom, 16)
        .onAppear { focused = true }
        .onChange(of: assistant.mode) { focused = true }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Text("LANTERN").font(.system(size: 11, weight: .semibold)).tracking(0.8).foregroundStyle(Theme.ink)
            HStack(spacing: 2) {
                ForEach(Assistant.Mode.allCases) { m in
                    Button { assistant.mode = m } label: {
                        Text(m.label).font(.system(size: 11.5, weight: .semibold))
                            .foregroundStyle(assistant.mode == m ? Color.black.opacity(0.85) : Theme.muted)
                            .padding(.horizontal, 10).padding(.vertical, 3)
                            .background(Capsule().fill(assistant.mode == m ? Theme.amber : .clear))
                    }
                    .buttonStyle(.plain)
                    .help(m == .ask ? "Ask anything; October's AI answers here" : "Describe a task; Lantern sets up an agent for it")
                }
            }
            .padding(2)
            .background(Capsule().fill(Theme.faint))
            Spacer()
            if assistant.mode == .ask && !assistant.turns.isEmpty {
                Button { assistant.clear() } label: {
                    Image(systemName: "square.and.pencil").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.muted)
                        .frame(width: 22, height: 22)
                }
                .buttonStyle(.plain).help("New chat")
            }
            Button { model.panel = nil } label: {
                Image(systemName: "xmark").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.muted)
                    .frame(width: 22, height: 22)
            }
            .buttonStyle(.plain).keyboardShortcut(.cancelAction).help("Close").accessibilityLabel("Close")
        }
    }

    private var waitingStrip: some View {
        Button { model.panel = .inbox } label: {
            HStack(spacing: 8) {
                Circle().fill(Theme.amber).frame(width: 7, height: 7)
                Text(model.inbox.count == 1 ? "1 agent is waiting on you" : "\(model.inbox.count) agents are waiting on you")
                    .font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.ink)
                Spacer()
                Image(systemName: "chevron.right").font(.system(size: 10, weight: .semibold)).foregroundStyle(Theme.muted)
            }
            .padding(.horizontal, 10).padding(.vertical, 7)
            .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.amber.opacity(0.12)))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Theme.amber.opacity(0.3)))
        }
        .buttonStyle(.plain)
    }

    private var signIn: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Sign in to October to ask. Answers come from October's AI and can see what your agents are doing.")
                .font(.system(size: 12)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
            Button("Sign in") { model.panel = .october }.buttonStyle(AmberButtonStyle())
        }
        .padding(.vertical, 4)
    }

    private var suggestions: some View {
        let ideas: [String] = assistant.mode == .ask
            ? ["What are my agents doing?", "Which agent needs me first, and why?", "Explain the last error one of my agents hit"]
            : model.recentFolders.prefix(3).map { "Fix the failing tests in \(URL(fileURLWithPath: $0).lastPathComponent)" }
        return VStack(alignment: .leading, spacing: 6) {
            Text(assistant.mode == .ask ? "Ask anything. Lantern includes what your agents are doing." : "Describe a task. Lantern sets up a session for it, and you check it before it starts.")
                .font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
            ForEach(ideas, id: \.self) { idea in
                Button { assistant.text = idea; focused = true } label: {
                    Text(idea).font(.system(size: 12)).foregroundStyle(Theme.ink.opacity(0.85)).lineLimit(1)
                        .padding(.horizontal, 10).padding(.vertical, 6)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 9, style: .continuous).fill(Theme.faint))
                }
                .buttonStyle(.plain)
            }
        }
    }

    private var conversation: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(assistant.turns) { turn in
                        VStack(alignment: .leading, spacing: 6) {
                            Text(turn.question).font(.system(size: 12.5, weight: .semibold)).foregroundStyle(Theme.ink)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            if turn.answer.isEmpty {
                                HStack(spacing: 6) {
                                    ProgressView().controlSize(.small)
                                    Text("Thinking…").font(.system(size: 12)).foregroundStyle(Theme.muted)
                                }
                            } else {
                                Text(markdown(turn.answer)).font(.system(size: 12.5)).lineSpacing(2)
                                    .foregroundStyle(turn.failed ? Theme.red : Theme.ink.opacity(0.92))
                                    .textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            if let note = turn.note { Text(note).font(.system(size: 11)).foregroundStyle(Theme.muted) }
                        }
                        .id(turn.id)
                    }
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

    private var input: some View {
        HStack(alignment: .bottom, spacing: 6) {
            TextField(placeholder, text: $assistant.text, axis: .vertical)
                .textFieldStyle(.plain).font(.system(size: 13)).lineLimit(1...6)
                .focused($focused)
                .onSubmit { assistant.submit() }
                .disabled(assistant.mode == .ask && !account.signedIn)
            Button {
                if dictation.isActive { dictation.stop() } else { dictation.start(prefix: assistant.text, owner: "assistant") }
            } label: {
                Image(systemName: dictation.isRecording ? "mic.fill" : "mic").font(.system(size: 12, weight: .medium))
                    .foregroundStyle(dictation.isRecording ? Theme.red : Theme.muted).frame(width: 22, height: 22)
            }
            .buttonStyle(.plain).help(dictation.isRecording ? "Stop listening" : "Speak")
            Button { assistant.submit() } label: {
                Image(systemName: assistant.mode == .ask ? "arrow.up" : "arrow.right")
                    .font(.system(size: 11, weight: .bold)).foregroundStyle(.black.opacity(0.85))
                    .frame(width: 22, height: 22)
                    .background(Circle().fill(canSubmit ? Theme.amber : Theme.amber.opacity(0.35)))
            }
            .buttonStyle(.plain).disabled(!canSubmit)
            .help(assistant.mode == .ask ? "Ask (Return)" : "Set up the session (Return)")
        }
        .padding(.leading, 11).padding(.trailing, 6).padding(.vertical, 7)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Color.black.opacity(0.3)))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(focused ? Theme.amber.opacity(0.5) : Theme.stroke))
    }

    private var canSubmit: Bool {
        let has = !assistant.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return assistant.mode == .ask ? has && !assistant.asking && account.signedIn : has
    }

    private var placeholder: String {
        if dictation.isRecording { return "Listening…" }
        switch assistant.mode {
        case .ask: return assistant.turns.isEmpty ? "Ask anything" : "Ask a follow-up"
        case .work: return "What should an agent do?"
        }
    }

    private func markdown(_ s: String) -> AttributedString {
        (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(s)
    }
}
