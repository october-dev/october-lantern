import SwiftUI

/// The conversation with one agent, read from its session file, inside the panel.
struct ChatView: View {
    let agent: Agent
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            header
            if agent.isPermissionPrompt {
                VStack(alignment: .leading, spacing: 8) {
                    Text(agent.question ?? "").font(.system(size: 12.5, weight: .medium)).foregroundStyle(Theme.amber)
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
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                }
                .frame(height: 440)
                .onAppear { proxy.scrollTo("bottom", anchor: .bottom) }
                .onChange(of: model.chatMessages) { proxy.scrollTo("bottom", anchor: .bottom) }
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
            .help("Back")
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
        if !model.chatSupported {
            note("Lantern can't read \(agent.kind.displayName) conversations yet. Click Open to see it in its terminal.")
        } else if model.chatMessages.isEmpty {
            note(model.chatLoading ? "Loading…" : "No messages yet.")
        } else {
            ForEach(Array(model.chatMessages.enumerated()), id: \.offset) { _, message in
                Bubble(message: message)
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

    private func note(_ text: String) -> some View {
        Text(text).font(.system(size: 12)).foregroundStyle(Theme.muted)
            .frame(maxWidth: .infinity).padding(.vertical, 40).multilineTextAlignment(.center)
    }
}

struct Bubble: View {
    let message: ChatMessage
    @State private var expanded = false

    var body: some View {
        switch message.role {
        case .user:
            HStack {
                Spacer(minLength: 40)
                Text(markdown(message.text))
                    .font(.system(size: 12.5))
                    .foregroundStyle(Theme.ink)
                    .textSelection(.enabled)
                    .lineLimit(expanded ? nil : 12)
                    .padding(.horizontal, 11).padding(.vertical, 8)
                    .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.amber.opacity(0.22)))
                    .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Theme.amber.opacity(0.3)))
                    .onTapGesture { expanded.toggle() }
            }
        case .agent:
            VStack(alignment: .leading, spacing: 4) {
                Text(markdown(message.text))
                    .font(.system(size: 12.5))
                    .foregroundStyle(Theme.ink.opacity(0.92))
                    .textSelection(.enabled)
                    .lineLimit(expanded ? nil : 16)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if message.text.count > 900 {
                    Button(expanded ? "Less" : "More") { expanded.toggle() }
                        .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
                }
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
        }
    }
}
