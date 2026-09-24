import SwiftUI

/// The conversation with one agent, read from its session file, inside the panel.
struct ChatView: View {
    let agent: Agent
    @ObservedObject var model: AppModel
    @ObservedObject private var metrics = PanelMetrics.shared
    /// Whether the newest message is in view; new messages scroll into view only then, so reading
    /// further up isn't interrupted.
    @State private var atBottom = true

    /// How many messages the engine sends at most (the newest ones).
    static let historyLimit = 120

    var body: some View {
        VStack(spacing: 0) {
            header
            if agent.isPermissionPrompt {
                VStack(alignment: .leading, spacing: 8) {
                    Text(agent.question ?? "").font(.system(size: 12.5, weight: .medium)).foregroundStyle(Theme.amber)
                        .textSelection(.enabled)
                    if let detail = agent.questionDetail, detail != agent.question {
                        RequestDetail(detail: detail)
                    }
                    PermissionButtons(agent: agent, model: model)
                }
                .padding(.horizontal, 12).padding(.bottom, 10)
            }
            Rectangle().fill(Theme.hairline).frame(height: 1)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8) {
                        content
                        Color.clear.frame(height: 1).id("bottom")
                            .onAppear { atBottom = true }
                            .onDisappear { atBottom = false }
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                }
                .frame(height: min(440, max(180, metrics.maxHeight - 240)))
                .onAppear { proxy.scrollTo("bottom", anchor: .bottom) }
                .onChange(of: model.chatMessages) {
                    if atBottom { proxy.scrollTo("bottom", anchor: .bottom) }
                }
            }
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Button { model.closeChat() } label: {
                Image(systemName: "chevron.left").font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.muted)
                    .frame(width: 22, height: 22)
            }
            .buttonStyle(.plain)
            .keyboardShortcut(.cancelAction)
            .help("Back (Esc)")
            .accessibilityLabel("Back")
            AgentBadge(agent: agent, size: 24)
            VStack(alignment: .leading, spacing: 1) {
                Text("@\(agent.handle)").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                Text(agent.title ?? [agent.project, agent.location].compactMap { $0 }.joined(separator: " · "))
                    .font(.system(size: 11)).foregroundStyle(Theme.muted).lineLimit(1)
            }
            Spacer(minLength: 6)
            StateChip(state: agent.state)
            SmallButton(title: "Open", symbol: "arrow.up.forward.app") { model.open(agent) }
        }
        .padding(.horizontal, 12)
        .padding(.top, 12)
        .padding(.bottom, 10)
    }

    @ViewBuilder private var content: some View {
        if agent.sessionMatch == "ambiguous" {
            note("Another \(agent.kind.displayName) session works in this folder, so Lantern can't tell which conversation is this one. Turn on hooks (Claude Code) or start it with --resume <id> for an exact match.")
        } else if !model.chatSupported {
            note("Lantern can't read \(agent.kind.displayName) conversations yet. Click Open to see it in its terminal.")
        } else if model.chatMessages.isEmpty {
            note(model.chatLoading ? "Loading…" : "No messages yet.")
        } else {
            if model.chatMessages.count >= Self.historyLimit {
                HStack(spacing: 6) {
                    Text("Recent messages only. The whole conversation is in its terminal.")
                        .font(.system(size: 11)).foregroundStyle(Theme.muted)
                    Button("Open") { model.open(agent) }
                        .buttonStyle(.plain).font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.amber)
                }
                .frame(maxWidth: .infinity)
                .padding(.bottom, 4)
            }
            ForEach(Self.keyed(model.chatMessages), id: \.key) { item in
                Bubble(message: item.message)
            }
            if agent.state == .working {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.mini)
                    Text("Working…").font(.system(size: 11)).foregroundStyle(Theme.muted)
                }
                .padding(.leading, 4)
            }
        }
    }

    /// Identities that stay with a message when older ones drop out of the window: its role, time
    /// and text, numbered when the same message repeats.
    static func keyed(_ messages: [ChatMessage]) -> [(key: String, message: ChatMessage)] {
        var seen: [String: Int] = [:]
        return messages.map { m in
            let base = "\(m.role.rawValue)|\(m.at ?? "")|\(m.text.hashValue)"
            let n = seen[base, default: 0]
            seen[base] = n + 1
            return (n == 0 ? base : "\(base)#\(n)", m)
        }
    }

    private func note(_ text: String) -> some View {
        Text(text).font(.system(size: 12)).foregroundStyle(Theme.muted)
            .frame(maxWidth: .infinity).padding(.vertical, 40).multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
    }
}

struct Bubble: View {
    let message: ChatMessage
    @State private var expanded = false
    /// The message's full height, measured, to offer More only when something is cut off.
    @State private var fullHeight: CGFloat = 0

    private var collapsedHeight: CGFloat { message.role == .user ? 200 : 280 }
    private var clipped: Bool { fullHeight > collapsedHeight + 1 }

    var body: some View {
        switch message.role {
        case .user:
            HStack {
                Spacer(minLength: 40)
                VStack(alignment: .trailing, spacing: 4) {
                    collapsible
                    moreButton
                }
                .padding(.horizontal, 11).padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.amber.opacity(0.22)))
                .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Theme.amber.opacity(0.3)))
            }
        case .agent:
            VStack(alignment: .leading, spacing: 4) {
                collapsible.frame(maxWidth: .infinity, alignment: .leading)
                moreButton
            }
            .padding(.horizontal, 11).padding(.vertical, 8)
            .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.faint))
            .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Theme.hairline))
            .padding(.trailing, 24)
        case .tool:
            HStack(spacing: 6) {
                Image(systemName: "chevron.right.2").font(.system(size: 8, weight: .bold))
                Text(message.text).font(.system(size: 11, design: .monospaced)).lineLimit(1).truncationMode(.tail)
            }
            .foregroundStyle(Theme.muted.opacity(0.8))
            .padding(.leading, 6)
            .help(message.text)
        }
    }

    private var collapsible: some View {
        MarkdownBlocks(text: message.text, ink: message.role == .user ? Theme.ink : Theme.ink.opacity(0.92))
            .fixedSize(horizontal: false, vertical: true)
            .background(GeometryReader { g in
                Color.clear.onAppear { fullHeight = g.size.height }.onChange(of: g.size.height) { _, h in fullHeight = h }
            })
            .frame(maxHeight: expanded ? nil : collapsedHeight, alignment: .top)
            .clipped()
    }

    @ViewBuilder private var moreButton: some View {
        if clipped {
            Button(expanded ? "Less" : "More") { expanded.toggle() }
                .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
        }
    }
}
