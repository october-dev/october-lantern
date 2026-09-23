import SwiftUI

/// The panel beside the pill: the inbox or the agent list, with a composer at the bottom.
struct PanelView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var dictation: Dictation

    var body: some View {
        VStack(spacing: 0) {
            header
            ScrollView {
                VStack(spacing: 8) {
                    switch model.panel {
                    case .agents: agentList
                    default: inboxList
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 8)
            }
            .frame(maxHeight: 440)
            .fixedSize(horizontal: false, vertical: true)

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
            Composer(model: model, dictation: dictation)
        }
        .frame(width: 380)
        .background(ZStack { Glass(material: .hudWindow); Color.black.opacity(0.55) })
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 18, style: .continuous).stroke(Theme.stroke, lineWidth: 1))
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
            ForEach(model.inbox) { agent in
                InboxCard(agent: agent, selected: model.target?.id == agent.id, model: model)
            }
        }
    }

    @ViewBuilder private var agentList: some View {
        if model.agents.isEmpty {
            EmptyState(title: "No agents running", detail: "Lantern looks for Claude Code, Codex, OpenCode and Pi.")
        } else {
            ForEach(model.ranked) { agent in
                AgentRow(agent: agent, selected: model.target?.id == agent.id)
                    .onTapGesture { model.compose(to: agent) }
                    .contextMenu {
                        Button("Message @\(agent.handle)") { model.compose(to: agent) }
                        Button("Open in \(agent.host?.app ?? "terminal")") { model.open(agent) }
                    }
            }
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
                    StateChip(state: agent.state)
                    Text(timeAgo(agent.since)).font(.system(size: 10)).foregroundStyle(Theme.muted)
                }
            }

            if let title = agent.title {
                Text(title).font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.ink.opacity(0.75)).lineLimit(1)
            }

            if let question = agent.question {
                Text(question)
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(Theme.amber)
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

            HStack(spacing: 8) {
                SmallButton(title: "Reply", symbol: "arrowshape.turn.up.left") { model.compose(to: agent) }
                SmallButton(title: "Open", symbol: "arrow.up.forward.app") { model.open(agent) }
                Spacer()
                SmallButton(title: "Done", symbol: "checkmark") { model.dismiss(agent) }
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.faint))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(selected ? Theme.amber.opacity(0.5) : Color.clear, lineWidth: 1)
        )
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
            StateChip(state: agent.state)
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
                    Button("@\(agent.handle) · \(agent.project ?? "")") { model.targetId = agent.id }
                }
            } label: {
                HStack(spacing: 4) {
                    Text(model.target.map { "To @\($0.handle)" } ?? "No agent selected")
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
                    Image(systemName: dictation.isRecording ? "waveform" : "mic")
                        .foregroundStyle(dictation.isRecording ? Theme.red : Theme.muted)
                        .symbolEffect(.variableColor.iterative, isActive: dictation.isRecording)
                }
                .buttonStyle(.plain)
                Button { model.send() } label: {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 18))
                        .foregroundStyle(model.draft.isEmpty ? Theme.muted : Theme.amber)
                }
                .buttonStyle(.plain)
                .disabled(model.draft.isEmpty || model.target == nil)
            }
            .padding(.horizontal, 12).padding(.vertical, 9)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Theme.faint))
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(dictation.isRecording ? Theme.red.opacity(0.5) : Theme.stroke, lineWidth: 1)
            )
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 12)
        .padding(.top, 4)
        .onChange(of: model.composeFocusToken) { focused = true }
        .onAppear { focused = true }
    }
}
