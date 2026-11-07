import SwiftUI

/// The conversation with one agent, read from its session file, inside the panel.
struct ChatView: View {
    let agent: Agent
    @ObservedObject var model: AppModel
    @ObservedObject private var metrics = PanelMetrics.shared
    /// Whether the newest message is in view; new messages scroll into view only then, so reading
    /// further up isn't interrupted.
    @State private var atBottom = true
    /// The full height of what waits for you above the conversation.
    @State private var pinnedHeight: CGFloat = 0

    /// How many messages the engine sends at most (the newest ones).
    static let historyLimit = 120

    private var launch: CardLaunch? { model.launch(forAgent: agent.id) }
    private var waitingCards: Bool { !(launch?.waiting.isEmpty ?? true) }
    /// A question Lantern can't answer, shown with its full request: those aren't always in the
    /// conversation's text.
    private var unanswered: Bool {
        agent.state == .needsInput && !agent.isAnswerable && (agent.question != nil || agent.questionDetail != nil)
    }
    private var hasPinned: Bool { agent.isAnswerable || unanswered || waitingCards }

    var body: some View {
        VStack(spacing: 0) {
            header
            // What waits for you (a terminal question, Lantern's own question or hand-off) stays in
            // view above the conversation, however long that is: opening from Waiting or a
            // notification shows it, and a draft answer stays put while you read further up.
            if hasPinned { pinned }
            Rectangle().fill(Theme.hairline).frame(height: 1)
            // One scroll for the conversation and the task's steps and result after it: two capped
            // areas, one above the other, cut the result off mid-text.
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8) {
                        content
                        if let launch {
                            LaunchCards(launch: launch, model: model, part: .rest).padding(.top, 4)
                        }
                        Color.clear.frame(height: 1).id("bottom")
                            .onAppear { atBottom = true }
                            .onDisappear { atBottom = false }
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                }
                // Fits the conversation, up to the panel's room: a short chat stays short.
                .frame(maxHeight: Self.scrollHeight(panel: metrics.maxHeight,
                                                    pinned: hasPinned ? min(pinnedHeight, Self.pinnedLimit(panel: metrics.maxHeight)) : 0))
                .fixedSize(horizontal: false, vertical: true)
                .onAppear { proxy.scrollTo("bottom", anchor: .bottom) }
                .onChange(of: model.chatMessages) {
                    if atBottom { proxy.scrollTo("bottom", anchor: .bottom) }
                }
                // A new step or the result changes only the cards; a question appearing, growing
                // or going changes the room the conversation has.
                .onChange(of: launch) {
                    if atBottom { proxy.scrollTo("bottom", anchor: .bottom) }
                }
                .onChange(of: hasPinned ? pinnedHeight : 0) {
                    if atBottom { proxy.scrollTo("bottom", anchor: .bottom) }
                }
            }
        }
    }

    /// What waits for you, scrolling on its own past `pinnedLimit` (a long plan, many choices).
    /// Not lazy, so a half-typed answer isn't dropped.
    private var pinned: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                if agent.isAnswerable {
                    QuestionCard(agent: agent, model: model, keys: true)
                } else if unanswered {
                    UnansweredQuestion(agent: agent, model: model, elsewhere: agent.canReply ? "Reply below." : nil)
                }
                if let launch, waitingCards {
                    LaunchCards(launch: launch, model: model, keys: !agent.isAnswerable, part: .waiting)
                }
            }
            .padding(.horizontal, 12).padding(.bottom, 10)
            .background(GeometryReader { g in
                Color.clear.onAppear { pinnedHeight = g.size.height }.onChange(of: g.size.height) { _, h in pinnedHeight = h }
            })
        }
        .frame(maxHeight: Self.pinnedLimit(panel: metrics.maxHeight))
        .fixedSize(horizontal: false, vertical: true)
    }

    /// Room the panel keeps for the header, the composer (up to five lines) and a toast.
    static let reserved: CGFloat = 230

    /// The most what waits for you takes above the conversation: up to 320, less on a short
    /// panel so the conversation keeps its 160.
    static func pinnedLimit(panel: CGFloat) -> CGFloat {
        max(100, min(320, panel - reserved - 160))
    }

    /// The most the conversation's scroll takes: the panel's room less the header, the composer,
    /// a toast and what's pinned above it (its shown height). The panel is never taller than the
    /// screen allows, so content beyond its room would be cut off (and its buttons would answer
    /// clicks somewhere else) rather than scroll.
    static func scrollHeight(panel: CGFloat, pinned: CGFloat) -> CGFloat {
        min(800, max(160, panel - reserved - pinned))
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
            AgentStateChip(agent: agent)
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
            if model.chatLoading {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Loading the conversation…").font(.system(size: 12)).foregroundStyle(Theme.muted)
                }
                .frame(maxWidth: .infinity).padding(.vertical, 24)
            } else {
                note("No messages yet.")
            }
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
            // An entry's id where it has one: updated (a tool call filled in), it's the same row.
            let base = m.id.map { "id|\($0)" } ?? "\(m.role.rawValue)|\(m.at ?? "")|\(m.text.hashValue)"
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
    var inAssistant = false
    @State private var expanded = false
    /// The message's full height, measured, to offer More only when something is cut off.
    @State private var fullHeight: CGFloat = 0

    /// Your first message without what Lantern added for the agent (the recipe line, October Bus,
    /// the folder and full-path note, the screenshot path, the toolkit list): the agent still gets
    /// all of it.
    static func withoutLanternNotes(_ text: String) -> String {
        var text = text
        if text.hasPrefix(Assistant.recipeLead), let end = text.range(of: "\n\n") {
            text = String(text[end.upperBound...])
        }
        let markers = ["\n\nYou're connected to October Bus", "\n\n(Started from October Lantern",
                       "\n\nFor context, a screenshot of my screen", "\n\nWhat this Mac already has",
                       "\n\nWhat this computer already has"]
        let cut = markers.compactMap { text.range(of: $0)?.lowerBound }.min()
        return cut.map { String(text[..<$0]) } ?? text
    }

    /// A file or web page an agent's message names, offered as a chip under it.
    enum Target: Hashable {
        case file(String)
        case link(String)
    }

    /// The links (http, https) and existing files (full or ~/ paths, also in `backticks`) a
    /// message names, in order, at most four (Rust `chat::targets`).
    static func targets(in text: String, home: String = NSHomeDirectory(), exists: (String) -> Bool) -> [Target] {
        let spans = text.components(separatedBy: "`").enumerated().filter { $0.offset % 2 == 1 }
            .map { $0.element.trimmingCharacters(in: .whitespaces) }
        let separators = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: "`()<>[]\"'"))
        var found: [Target] = []
        for raw in spans + text.components(separatedBy: separators) {
            var word = Substring(raw)
            while let last = word.last, ".,;:!?*".contains(last) { word = word.dropLast() }
            let target: Target?
            if word.hasPrefix("http://") || word.hasPrefix("https://") {
                target = word.count > 8 ? .link(String(word)) : nil
            } else {
                let path = word.hasPrefix("~/") ? home + "/" + word.dropFirst(2) : String(word)
                target = path.hasPrefix("/") && path.count > 1 && !path.contains("\n") && exists(path) ? .file(path) : nil
            }
            if let target, !found.contains(target) {
                found.append(target)
                if found.count == 4 { break }
            }
        }
        return found
    }

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
                .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(inAssistant ? Color.white.opacity(0.07) : Theme.amber.opacity(0.22)))
                .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(inAssistant ? .clear : Theme.amber.opacity(0.3)))
            }
        case .agent:
            VStack(alignment: .leading, spacing: 4) {
                collapsible.frame(maxWidth: .infinity, alignment: .leading)
                HStack(spacing: 12) {
                    moreButton
                    // The whole reply, even while it's shown collapsed.
                    CopyButton(text: message.text)
                }
                let found = Self.targets(in: message.text) { FileManager.default.fileExists(atPath: $0) }
                if !found.isEmpty {
                    HStack(spacing: 10) {
                        ForEach(found, id: \.self) { TargetChip(target: $0) }
                    }
                }
            }
            .padding(.horizontal, inAssistant ? 0 : 11).padding(.vertical, inAssistant ? 2 : 8)
            .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(inAssistant ? .clear : Theme.faint))
            .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(inAssistant ? .clear : Theme.hairline))
            .padding(.trailing, inAssistant ? 0 : 24)
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
        MarkdownBlocks(text: message.role == .user ? Self.withoutLanternNotes(message.text) : message.text,
                       ink: message.role == .user ? Theme.ink : Theme.ink.opacity(0.92))
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

/// A session starting in Lantern, shown as its conversation from the moment you start it: your
/// message, then "Starting…" until the real conversation takes over (or why it couldn't start).
struct StartingChatView: View {
    let preview: LaunchPreview
    @ObservedObject var model: AppModel

    private var folderName: String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        if preview.folder == home || preview.folder == home + "/" { return "Home folder" }
        return (preview.folder as NSString).lastPathComponent
    }

    private var status: String {
        if preview.kind == .october, model.harness?.status == "installing" { return "Setting up October Harness…" }
        return "Starting \(preview.kind.displayName)…"
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Rectangle().fill(Theme.hairline).frame(height: 1)
            VStack(alignment: .leading, spacing: 8) {
                let text = Bubble.withoutLanternNotes(preview.prompt).trimmingCharacters(in: .whitespacesAndNewlines)
                if !text.isEmpty { Bubble(message: ChatMessage(role: .user, text: text, at: nil)) }
                if let error = preview.error {
                    failure(error)
                } else {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.mini)
                        Text(status).font(.system(size: 11)).foregroundStyle(Theme.muted)
                    }
                    .padding(.leading, 4)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            if preview.error == nil { composer }
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Button { model.closePreview() } label: {
                Image(systemName: "chevron.left").font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.muted)
                    .frame(width: 22, height: 22)
            }
            .buttonStyle(.plain)
            .keyboardShortcut(.cancelAction)
            .help("Back (Esc)")
            .accessibilityLabel("Back")
            Group {
                if let img = Assets.harness(preview.kind) {
                    Image(nsImage: img).resizable().interpolation(.high).aspectRatio(contentMode: .fit)
                        .clipShape(RoundedRectangle(cornerRadius: 24 * 0.22, style: .continuous))
                } else {
                    RoundedRectangle(cornerRadius: 24 * 0.22, style: .continuous).fill(Theme.faint)
                        .overlay(Text(preview.kind.displayName.prefix(1)).font(.system(size: 11, weight: .bold, design: .rounded)))
                }
            }
            .frame(width: 24, height: 24)
            VStack(alignment: .leading, spacing: 1) {
                Text(preview.kind.displayName).font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                Text(folderName).font(.system(size: 11)).foregroundStyle(Theme.muted).lineLimit(1)
            }
            Spacer(minLength: 6)
        }
        .padding(.horizontal, 12)
        .padding(.top, 12)
        .padding(.bottom, 10)
    }

    private func failure(_ error: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Couldn't start it: \(error)")
                .font(.system(size: 12)).foregroundStyle(Theme.amber)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            HStack(spacing: 8) {
                SmallButton(title: "Try in Terminal", symbol: "terminal") { model.retryPreviewInTerminal() }
                SmallButton(title: "Close", symbol: "xmark") { model.closePreview() }
            }
        }
        .padding(.leading, 4)
    }

    /// Where the composer will be; it takes messages once the session is up.
    private var composer: some View {
        Text("You can reply once it's started.")
            .font(.system(size: 12)).foregroundStyle(Theme.muted.opacity(0.7))
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 12).padding(.vertical, 9)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Theme.faint))
            .padding(.horizontal, 12).padding(.bottom, 12)
    }
}

/// A link or file from an agent's message: links open; files and folders are shown in Finder.
struct TargetChip: View {
    let target: Bubble.Target

    var body: some View {
        switch target {
        case .link(let link):
            Button("↗ " + link.replacingOccurrences(of: "https://", with: "").replacingOccurrences(of: "http://", with: "")) {
                if let url = URL(string: link) { NSWorkspace.shared.open(url) }
            }
            .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.amber)
            .lineLimit(1).truncationMode(.middle).help(link)
        case .file(let path):
            let url = URL(fileURLWithPath: path)
            Button((url.hasDirectoryPath ? "📁 " : "📄 ") + url.lastPathComponent) {
                NSWorkspace.shared.activateFileViewerSelecting([url])
            }
            .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.amber)
            .lineLimit(1).truncationMode(.middle).help("Show \(path) in Finder")
        }
    }
}
