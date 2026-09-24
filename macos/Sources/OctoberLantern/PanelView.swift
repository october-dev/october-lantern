import SwiftUI

/// The panel beside the pill: the inbox or the agent list, with a composer at the bottom.
struct PanelView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var dictation: Dictation

    var body: some View {
        VStack(spacing: 0) {
            EngineBanner(health: model.engineHealth)
            switch model.panel {
            case .newSession:
                PanelTitle(title: "New session", model: model)
                NewSessionView(model: model)
            case .october:
                PanelTitle(title: "October", model: model)
                OctoberView(model: model)
            default:
                if let agent = model.chatAgent {
                    ChatView(agent: agent, model: model)
                } else {
                header
                ScrollView {
                    VStack(spacing: 8) {
                        if model.panel == .agents { agentList } else { inboxList }
                    }
                    .padding(.horizontal, 12)
                    .padding(.bottom, 8)
                }
                .frame(maxHeight: max(240, PanelMetrics.shared.maxHeight - 200))
                .fixedSize(horizontal: false, vertical: true)
                }
            }

            if let toast = model.toast {
                Text(toast)
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.ink.opacity(0.9))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12).padding(.vertical, 8)
                    .background(RoundedRectangle(cornerRadius: 10).fill(Theme.faint))
                    .padding(.horizontal, 12).padding(.bottom, 6)
                    .transition(.opacity)
            }
            if model.panel?.isList ?? true {
                Composer(model: model, dictation: dictation)
            }
        }
        .frame(width: 380)
        .glassSurface(RoundedRectangle(cornerRadius: 22, style: .continuous))
        .environment(\.colorScheme, .dark)
        .animation(.easeOut(duration: 0.15), value: model.toast)
    }

    private var header: some View {
        HStack(spacing: 14) {
            TabLabel(title: "Waiting", count: model.inbox.count, selected: model.panel != .agents) { model.panel = .inbox }
            TabLabel(title: "Agents", count: model.agents.count, selected: model.panel == .agents) { model.panel = .agents }
            Spacer()
            Button { model.panel = nil } label: {
                Image(systemName: "xmark").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.muted)
                    .frame(width: 22, height: 22)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Close")
            .keyboardShortcut(.cancelAction)
        }
        .padding(.horizontal, 16)
        .padding(.top, 14)
        .padding(.bottom, 10)
    }

    @ViewBuilder private var inboxList: some View {
        if model.inbox.isEmpty {
            EmptyState(
                title: "Nothing waiting on you",
                detail: model.agents.isEmpty ? "No agents running." : "\(model.agents.count) agent\(model.agents.count == 1 ? "" : "s") running."
            )
        } else {
            ForEach(model.newInbox) { agent in
                InboxCard(agent: agent, selected: model.target?.id == agent.id, isNew: true, model: model)
                    .opensChat(agent, model: model)
            }
            if !model.earlierInbox.isEmpty {
                HStack {
                    Text("EARLIER").font(.system(size: 10.5, weight: .semibold)).tracking(0.8).foregroundStyle(Theme.muted)
                    Spacer()
                    Button("Clear all") { model.dismiss(model.earlierInbox) }
                        .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
                        .help("Dismiss all of these until they have something new")
                }
                .padding(.horizontal, 4).padding(.top, model.newInbox.isEmpty ? 0 : 6)
                ForEach(model.earlierInbox) { agent in
                    InboxCard(agent: agent, selected: model.target?.id == agent.id, model: model)
                        .opensChat(agent, model: model)
                }
            }
        }
    }

    @ViewBuilder private var agentList: some View {
        if model.agents.isEmpty {
            EmptyState(title: "No agents running", detail: "Start Claude Code, Codex, OpenCode, Pi, Gemini or another agent in any terminal.")
        } else {
            ForEach(model.ranked) { agent in
                Button { model.openChat(agent) } label: {
                    AgentRow(agent: agent, selected: model.target?.id == agent.id)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("@\(agent.handle), \(agent.state.label)\(agent.title.map { ", \($0)" } ?? "")")
                .accessibilityHint("Opens the conversation")
                    .contextMenu {
                        Button("Message @\(agent.handle)") { model.compose(to: agent) }
                        Button("Open in \(agent.host?.app ?? "terminal")") { model.open(agent) }
                    }
            }
        }
    }
}

/// A card that opens its agent's conversation when clicked. The card holds its own buttons, so it
/// can't itself be a Button; it's exposed to VoiceOver as one.
private struct OpensChat: ViewModifier {
    let agent: Agent
    let model: AppModel

    func body(content: Content) -> some View {
        content
            .contentShape(Rectangle())
            .onTapGesture { model.openChat(agent) }
            .accessibilityElement(children: .contain)
            .accessibilityAddTraits(.isButton)
            .accessibilityAction(named: "Open conversation") { model.openChat(agent) }
    }
}

extension View {
    fileprivate func opensChat(_ agent: Agent, model: AppModel) -> some View { modifier(OpensChat(agent: agent, model: model)) }
}

/// Shown while Lantern's engine isn't running normally.
struct EngineBanner: View {
    let health: EngineHealth

    var body: some View {
        let text: String? = switch health {
        case .ready: nil
        case .starting: "Starting…"
        case .restarting: "Lantern's engine stopped. Restarting…"
        case .failed(let message): message
        }
        if let text {
            Label(text, systemImage: health == .starting ? "hourglass" : "exclamationmark.triangle")
                .font(.system(size: 11.5, weight: .medium))
                .foregroundStyle(health == .starting ? Theme.muted : Theme.amber)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16).padding(.top, 12)
        }
    }
}

struct TabLabel: View {
    let title: String
    let count: Int
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Text(title.uppercased())
                    .font(.system(size: 11, weight: .semibold))
                    .tracking(0.8)
                if count > 0 {
                    Text("\(count)")
                        .font(.system(size: 10, weight: .bold, design: .rounded))
                        .padding(.horizontal, 6).padding(.vertical, 1)
                        .background(Capsule().fill(selected && title == "Waiting" ? Theme.amber.opacity(0.25) : Theme.faint))
                        .foregroundStyle(selected && title == "Waiting" ? Theme.amber : Theme.muted)
                }
            }
            .foregroundStyle(selected ? Theme.ink : Theme.muted)
        }
        .buttonStyle(.plain)
    }
}

struct EmptyState: View {
    let title: String
    let detail: String

    var body: some View {
        VStack(spacing: 4) {
            Text(title).font(.system(size: 13, weight: .medium)).foregroundStyle(Theme.ink.opacity(0.85))
            Text(detail).font(.system(size: 12)).foregroundStyle(Theme.muted)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 28)
    }
}

struct InboxCard: View {
    let agent: Agent
    let selected: Bool
    var isNew = false
    @ObservedObject var model: AppModel
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                AgentBadge(agent: agent, size: 28)
                VStack(alignment: .leading, spacing: 1) {
                    Text("@\(agent.handle)").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                    Text([agent.project, agent.location].compactMap { $0 }.joined(separator: " · "))
                        .font(.system(size: 11)).foregroundStyle(Theme.muted).lineLimit(1)
                }
                Spacer()
                VStack(alignment: .trailing, spacing: 2) {
                    HStack(spacing: 6) {
                        if isNew {
                            Text("NEW").font(.system(size: 9, weight: .bold)).tracking(0.5)
                                .foregroundStyle(.black.opacity(0.8))
                                .padding(.horizontal, 5).padding(.vertical, 1)
                                .background(Capsule().fill(Theme.amber))
                        }
                        AgentStateChip(agent: agent)
                    }
                    Text(timeAgo(agent.since)).font(.system(size: 10)).foregroundStyle(Theme.muted)
                }
            }

            if let title = agent.title {
                Text(title).font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.ink.opacity(0.75)).lineLimit(1)
            }

            if let question = agent.question {
                VStack(alignment: .leading, spacing: 6) {
                    Text(question)
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(Theme.amber)
                        .textSelection(.enabled)
                    if let detail = agent.questionDetail, detail != question {
                        RequestDetail(detail: detail)
                    }
                }
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 10).fill(Theme.amber.opacity(0.12)))
            }

            if let message = agent.lastMessage, !message.isEmpty {
                Text(markdown(message))
                    .font(.system(size: 12.5))
                    .foregroundStyle(Theme.ink.opacity(0.9))
                    .lineLimit(expanded ? 40 : 5)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if message.count > 280 {
                    Button(expanded ? "Less" : "More") { expanded.toggle() }
                        .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
                }
            }

            if agent.isPermissionPrompt {
                PermissionButtons(agent: agent, model: model)
            }

            HStack(spacing: 8) {
                SmallButton(title: "Reply", symbol: "arrowshape.turn.up.left") { model.compose(to: agent) }
                SmallButton(title: "Open", symbol: "arrow.up.forward.app") { model.open(agent) }
                Spacer()
                SmallButton(title: "Dismiss", symbol: "checkmark") { model.dismiss(agent) }
                    .help("Hide this until @\(agent.handle) has something new")
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Theme.hairline, lineWidth: 1))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(selected || isNew ? Theme.amber.opacity(selected ? 0.6 : 0.3) : Color.clear, lineWidth: 1)
        )
    }
}

/// The full request behind a permission prompt (every line of the command), collapsed by default.
struct RequestDetail: View {
    let detail: String
    @State private var shown = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button(shown ? "Hide full request" : "Show full request") { shown.toggle() }
                .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
            if shown {
                ScrollView {
                    Text(detail)
                        .font(.system(size: 11.5, design: .monospaced))
                        .foregroundStyle(Theme.ink.opacity(0.9))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 180)
            }
        }
    }
}

/// Allow / Deny for a Claude Code permission prompt, where Lantern can press keys in its terminal.
struct PermissionButtons: View {
    let agent: Agent
    @ObservedObject var model: AppModel

    var body: some View {
        if agent.route?.sendsKeys ?? false {
            HStack(spacing: 8) {
                Button { model.answerPermission(agent, allow: true) } label: {
                    Label("Allow", systemImage: "checkmark").font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(.black.opacity(0.85))
                        .frame(maxWidth: .infinity).padding(.vertical, 6)
                        .background(RoundedRectangle(cornerRadius: 9, style: .continuous).fill(Theme.amber))
                }
                .buttonStyle(.plain)
                .help("Allow once: presses 1 in the agent's terminal, if this is still the prompt on screen")
                Button { model.answerPermission(agent, allow: false) } label: {
                    Label("Deny", systemImage: "xmark").font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Theme.ink)
                        .frame(maxWidth: .infinity).padding(.vertical, 6)
                        .background(RoundedRectangle(cornerRadius: 9, style: .continuous).fill(Theme.faint))
                        .overlay(RoundedRectangle(cornerRadius: 9, style: .continuous).strokeBorder(Theme.hairline))
                }
                .buttonStyle(.plain)
                .help("Decline (presses Escape), then tell the agent what to do instead")
            }
        }
    }
}

struct AgentRow: View {
    let agent: Agent
    let selected: Bool
    @State private var hover = false

    var body: some View {
        HStack(spacing: 10) {
            AgentBadge(agent: agent, size: 26)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 6) {
                    Text("@\(agent.handle)").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                    if let title = agent.title {
                        Text(title).font(.system(size: 12)).foregroundStyle(Theme.muted).lineLimit(1)
                    }
                }
                Text([agent.shortCwd, agent.location].compactMap { $0 }.joined(separator: " · "))
                    .font(.system(size: 11)).foregroundStyle(Theme.muted).lineLimit(1).truncationMode(.head)
            }
            Spacer(minLength: 8)
            AgentStateChip(agent: agent)
        }
        .padding(.horizontal, 10).padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: 12).fill(hover || selected ? Theme.faint : Color.clear))
        .contentShape(Rectangle())
        .onHover { hover = $0 }
    }
}

struct SmallButton: View {
    let title: String
    let symbol: String
    let action: () -> Void
    @State private var hover = false

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: symbol)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(hover ? Theme.ink : Theme.muted)
                .padding(.horizontal, 8).padding(.vertical, 4)
                .background(Capsule().fill(hover ? Theme.faint : Color.clear))
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
    }
}

struct Composer: View {
    @ObservedObject var model: AppModel
    @ObservedObject var dictation: Dictation
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Menu {
                ForEach(model.agents) { agent in
                    // Inside a conversation, the recipient is the conversation: choosing another
                    // agent opens its conversation rather than sending there from this one.
                    Button("@\(agent.handle) · \(agent.project ?? "")") {
                        if model.chatAgentId != nil { model.openChat(agent) } else { model.readdress(to: agent) }
                    }
                }
            } label: {
                HStack(spacing: 4) {
                    Text(model.target.map { "To @\($0.handle)" } ?? (model.targetMissing ? "That agent has exited. Choose another" : "No agent selected"))
                    if let t = model.target, !t.canReply {
                        Text("· copies and opens \(t.host?.app ?? "terminal")").foregroundStyle(Theme.muted.opacity(0.7))
                    }
                }
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(Theme.muted)
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()

            HStack(alignment: .bottom, spacing: 8) {
                TextField(model.target.map { "Message @\($0.handle)…" } ?? "Message an agent…", text: $model.draft, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(.system(size: 13))
                    .lineLimit(1...5)
                    .focused($focused)
                    .onSubmit { model.send() }
                Button { model.toggleDictation() } label: {
                    Image(systemName: dictation.isAuthorizing ? "hourglass" : dictation.isRecording ? "waveform" : "mic")
                        .foregroundStyle(dictation.isActive ? Theme.red : Theme.muted)
                        .symbolEffect(.variableColor.iterative, isActive: dictation.isRecording)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(dictation.isActive ? "Stop dictation" : "Dictate")
                .help(dictation.isAuthorizing ? "Waiting for permission to use the microphone. Click to cancel." : "Dictate")
                Button { model.send() } label: {
                    Image(systemName: model.sending ? "ellipsis.circle" : "arrow.up.circle.fill")
                        .font(.system(size: 18))
                        .foregroundStyle(model.draft.isEmpty || model.sending ? Theme.muted : Theme.amber)
                }
                .buttonStyle(.plain)
                .disabled(model.draft.isEmpty || model.target == nil || model.sending)
                .help(model.sending ? "Sending…" : "Send")
                .accessibilityLabel("Send")
            }
            .padding(.horizontal, 12).padding(.vertical, 9)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Theme.faint))
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(dictation.isActive ? Theme.red.opacity(0.5) : Theme.stroke, lineWidth: 1)
            )
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 12)
        .padding(.top, 4)
        .onChange(of: model.composeFocusToken) { focused = true }
        .onAppear { focused = true }
    }
}

/// The agent's state, or "Recent" for an app session that isn't running any more.
struct AgentStateChip: View {
    let agent: Agent

    var body: some View {
        if agent.isLive {
            StateChip(state: agent.state)
        } else {
            Text("Recent").font(.system(size: 10.5, weight: .medium)).foregroundStyle(Theme.muted)
                .padding(.horizontal, 7).padding(.vertical, 2)
                .background(Capsule().fill(Theme.faint))
                .help("A \(agent.source ?? "app") session that isn't running now")
        }
    }
}

