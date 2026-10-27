import AppKit
import SwiftUI

/// One session Lantern started with its card tools (engine `cards`): what it asks of you, its
/// progress and its result. Kept after the agent exits.
struct CardLaunch: Decodable, Identifiable, Hashable {
    let id: String
    let kind: AgentKind
    let cwd: String
    let title: String
    /// The agent running it, once Lantern has seen it start (it stays after the agent exits).
    let agentId: String?
    /// The agent has Lantern's tools (its MCP server started).
    let tools: Bool
    let createdAt: Double
    let endedAt: Double?
    let status: String?
    let steps: [Step]
    let cards: [Card]

    struct Step: Decodable, Hashable {
        let id: String
        let title: String
        let done: Bool
    }

    var waiting: [Card] { cards.filter(\.isWaiting) }
    var ended: Date? { endedAt.map { Date(timeIntervalSince1970: $0 / 1000) } }
}

/// A question, a step handed to you, or a result (engine `cards::Op`).
struct Card: Decodable, Identifiable, Hashable {
    let id: String
    /// ask | handOff | result
    let kind: String
    /// waiting | answered | delivered | unconfirmed | expired | stopped | cancelled | shown
    let state: String
    let revision: Int
    let createdAt: Double
    let resolvedAt: Double?
    let answer: CardAnswer?
    // ask
    let question: String?
    let details: String?
    let options: [String]?
    // handOff
    let reason: String?
    let step: String?
    let why: String?
    let instructions: [String]?
    let url: String?
    // result
    let summary: String?
    let outcome: String?
    let files: [File]?

    struct File: Decodable, Hashable {
        let path: String
        let name: String
        let folder: Bool
    }

    var isWaiting: Bool { state == "waiting" }
}

/// Your answer to a card: one of its options, your own words, or whether you did the step.
struct CardAnswer: Codable, Hashable {
    var choice: String?
    var text: String?
    var done: Bool?

    var json: [String: Any] {
        var v: [String: Any] = [:]
        if let choice { v["choice"] = choice }
        if let text { v["text"] = text }
        if let done { v["done"] = done }
        return v
    }
}

/// A session's cards: its status and steps, what it's waiting for you on, and its result.
struct LaunchCards: View {
    let launch: CardLaunch
    @ObservedObject var model: AppModel
    /// Number keys pick an option of the waiting question.
    var keys = false

    /// Waiting cards, then the latest result, then the last couple of closed questions.
    private var shown: [Card] {
        let result = launch.cards.last { $0.kind == "result" }
        let closed = launch.cards.filter { $0.kind != "result" && !$0.isWaiting }.suffix(2)
        return Array(closed) + launch.waiting + (result.map { [$0] } ?? [])
    }

    var body: some View {
        if launch.status != nil || !launch.steps.isEmpty || !shown.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                progress
                ForEach(shown) { card in
                    switch card.kind {
                    case "result": ResultCard(card: card)
                    case "handOff": HandOffCard(card: card, model: model)
                    default: AskCard(card: card, model: model, keys: keys && card.id == launch.waiting.first?.id)
                    }
                }
            }
        }
    }

    @ViewBuilder private var progress: some View {
        if launch.status != nil || !launch.steps.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                if let status = launch.status, launch.endedAt == nil {
                    Label(status, systemImage: "ellipsis.circle")
                        .font(.system(size: 12)).foregroundStyle(Theme.ink.opacity(0.85))
                }
                ForEach(launch.steps, id: \.id) { step in
                    Label(step.title, systemImage: step.done ? "checkmark.circle.fill" : "circle")
                        .font(.system(size: 12))
                        .foregroundStyle(step.done ? Theme.green : Theme.muted)
                }
            }
        }
    }
}

/// Where a closed or answered card stands, in words.
struct CardStateLabel: View {
    let card: Card
    let sending: Bool

    var body: some View {
        switch card.state {
        case "waiting" where sending:
            Text("Sending…").font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
        case "answered":
            Text("Giving it to the agent…").font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
        case "delivered":
            Label("Sent", systemImage: "checkmark").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.green)
        case "unconfirmed":
            Label("Not sure the agent got it", systemImage: "questionmark.circle")
                .font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.amber)
                .help("Your answer was saved, but the agent stopped waiting before it took it")
        case "expired":
            Text("Closed: nobody answered in time").font(.system(size: 11)).foregroundStyle(Theme.muted)
        case "stopped":
            Text("Closed: the agent stopped waiting").font(.system(size: 11)).foregroundStyle(Theme.muted)
        case "cancelled":
            Text("Closed by the agent").font(.system(size: 11)).foregroundStyle(Theme.muted)
        default:
            EmptyView()
        }
    }
}

private func cardBox(_ content: some View, waiting: Bool) -> some View {
    content
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(waiting ? Theme.amber.opacity(0.10) : Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(waiting ? Theme.amber.opacity(0.22) : Theme.hairline))
}

/// A question from the agent: its options, or your own words.
struct AskCard: View {
    let card: Card
    @ObservedObject var model: AppModel
    var keys = false
    @State private var words = ""

    private var open: Bool { card.isWaiting && !model.cardSending.contains(card.id) }

    var body: some View {
        cardBox(
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(card.question ?? "")
                        .font(.system(size: 13, weight: .medium)).foregroundStyle(Theme.ink)
                        .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 4)
                    CardStateLabel(card: card, sending: model.cardSending.contains(card.id))
                }
                if let details = card.details { RequestDetail(detail: details) }
                ForEach(Array((card.options ?? []).enumerated()), id: \.offset) { i, option in
                    OptionRow(number: i + 1, label: option, detail: nil, chosen: card.answer?.choice == option, faded: !open) {
                        model.answerCard(card, CardAnswer(choice: option))
                    }
                }
                if let text = card.answer?.text {
                    Text("You answered: \(text)").font(.system(size: 12)).foregroundStyle(Theme.ink.opacity(0.8)).textSelection(.enabled)
                } else if card.isWaiting {
                    HStack(spacing: 6) {
                        TextField(card.options?.isEmpty == false ? "Or answer in your own words" : "Your answer", text: $words)
                            .textFieldStyle(.roundedBorder).font(.system(size: 12))
                            .onSubmit(sendWords)
                        Button("Send", action: sendWords).buttonStyle(AmberButtonStyle())
                            .disabled(!open || words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
                if let error = model.cardErrors[card.id] {
                    Text(error).font(.system(size: 11)).foregroundStyle(Theme.red)
                }
            }, waiting: card.isWaiting
        )
        .background(NumberKeys(enabled: keys && open && !(card.options ?? []).isEmpty, press: press))
    }

    private func sendWords() {
        let text = words.trimmingCharacters(in: .whitespacesAndNewlines)
        guard open, !text.isEmpty else { return }
        model.answerCard(card, CardAnswer(text: text))
    }

    private func press(_ n: Int) -> Bool {
        guard let options = card.options, options.indices.contains(n - 1) else { return false }
        model.answerCard(card, CardAnswer(choice: options[n - 1]))
        return true
    }
}

/// A step only you can do: what, why (in Lantern's words, then the agent's), where, and whether
/// you did it.
struct HandOffCard: View {
    let card: Card
    @ObservedObject var model: AppModel
    @State private var note = ""

    private var open: Bool { card.isWaiting && !model.cardSending.contains(card.id) }

    /// Lantern's own words for why it's yours to do; the agent's reason is shown as its own.
    private var rule: String {
        switch card.reason {
        case "signIn": "Sign in yourself, in your own browser. Never send your password to the agent."
        case "verification": "Only you can do this check. Never send codes to the agent."
        case "payment": "Only you can approve a payment."
        case "identity": "Only you can prove who you are."
        case "physical": "This needs you in person."
        default: "Only you can do this step."
        }
    }

    private var site: String? { card.url.flatMap { URL(string: $0)?.host() } }

    var body: some View {
        cardBox(
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text("Your turn: \(card.step ?? "")")
                        .font(.system(size: 13, weight: .medium)).foregroundStyle(Theme.ink)
                        .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 4)
                    CardStateLabel(card: card, sending: model.cardSending.contains(card.id))
                }
                Text(rule).font(.system(size: 11)).foregroundStyle(Theme.muted)
                if let why = card.why {
                    Text("The agent says: \(why)").font(.system(size: 12)).foregroundStyle(Theme.ink.opacity(0.8)).textSelection(.enabled)
                }
                ForEach(Array((card.instructions ?? []).enumerated()), id: \.offset) { i, step in
                    Text("\(i + 1). \(step)").font(.system(size: 12)).foregroundStyle(Theme.ink.opacity(0.85)).textSelection(.enabled)
                }
                if let url = card.url.flatMap(URL.init(string:)), let site {
                    Button("Open \(site)") { NSWorkspace.shared.open(url) }
                        .buttonStyle(SecondaryButtonStyle())
                        .help(url.absoluteString)
                }
                if let done = card.answer?.done {
                    Text(done ? "You said you did it." : "You said you can't right now.")
                        .font(.system(size: 12)).foregroundStyle(Theme.ink.opacity(0.8))
                } else if card.isWaiting {
                    TextField("Add a note (optional)", text: $note).textFieldStyle(.roundedBorder).font(.system(size: 12))
                    HStack(spacing: 8) {
                        Button("I've done it") { reply(true) }.buttonStyle(AmberButtonStyle()).disabled(!open)
                        Button("I can't right now") { reply(false) }.buttonStyle(SecondaryButtonStyle()).disabled(!open)
                    }
                }
                if let error = model.cardErrors[card.id] {
                    Text(error).font(.system(size: 11)).foregroundStyle(Theme.red)
                }
            }, waiting: card.isWaiting
        )
    }

    private func reply(_ done: Bool) {
        let text = note.trimmingCharacters(in: .whitespacesAndNewlines)
        model.answerCard(card, CardAnswer(text: text.isEmpty ? nil : text, done: done))
    }
}

/// What the agent reports it did, with its files.
struct ResultCard: View {
    let card: Card

    private var outcome: (String, Color) {
        switch card.outcome {
        case "partial": ("Partly done", Theme.amber)
        case "failed": ("Didn't work", Theme.red)
        default: ("Says it's done", Theme.green)
        }
    }

    var body: some View {
        cardBox(
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    Text(outcome.0).font(.system(size: 11, weight: .semibold)).foregroundStyle(outcome.1)
                    Text("· reported by the agent").font(.system(size: 11)).foregroundStyle(Theme.muted)
                    Spacer()
                    Button {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(card.summary ?? "", forType: .string)
                    } label: { Image(systemName: "doc.on.doc") }
                        .buttonStyle(.plain).foregroundStyle(Theme.muted).help("Copy the summary")
                }
                Text(markdown(card.summary ?? ""))
                    .font(.system(size: 12)).foregroundStyle(Theme.ink.opacity(0.9))
                    .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                ForEach(card.files ?? [], id: \.path) { file in FileRow(file: file) }
            }, waiting: false
        )
    }
}

/// A file or folder from a result. Documents open; folders, apps and scripts are shown in Finder
/// instead of being run.
private struct FileRow: View {
    let file: Card.File

    private var exists: Bool { FileManager.default.fileExists(atPath: file.path) }
    private var runnable: Bool {
        let url = URL(fileURLWithPath: file.path)
        return ["app", "command", "sh", "tool", "pkg", "scpt", "workflow", "terminal"].contains(url.pathExtension.lowercased())
            || FileManager.default.isExecutableFile(atPath: file.path)
    }

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: file.folder ? "folder" : "doc").foregroundStyle(Theme.muted)
            Text(file.name).font(.system(size: 12)).foregroundStyle(Theme.ink).lineLimit(1).truncationMode(.middle).help(file.path)
            Spacer()
            if !exists {
                Text("Moved or deleted").font(.system(size: 11)).foregroundStyle(Theme.muted)
            } else {
                if !file.folder, !runnable {
                    Button("Open") { NSWorkspace.shared.open(URL(fileURLWithPath: file.path)) }.buttonStyle(.plain)
                        .font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.amber)
                }
                Button("Show") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: file.path)]) }.buttonStyle(.plain)
                    .font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.amber)
            }
        }
    }
}
