import AppKit
import SwiftUI

/// An agent's question, answered right here: a permission (Yes / Always / No), Claude's own
/// questions (their options, numbered as in the terminal) or its plan (start / keep planning).
/// With `keys`, pressing a number picks too, unless you're typing something in a text box.
struct QuestionCard: View {
    let agent: Agent
    @ObservedObject var model: AppModel
    var keys = false
    /// The options ticked so far, per question, when one click doesn't answer it all.
    @State private var picks: [Int: Set<Int>] = [:]

    private var sent: AgentAnswer? { agent.promptId.flatMap { model.answered[$0] } }
    private var questions: [Choices.Question] { agent.choices?.questions ?? [] }
    /// One click answers: a permission, a plan, or one single-choice question.
    private var oneClick: Bool {
        agent.questionKind != "choice" || (questions.count == 1 && !questions[0].multiSelect)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            switch agent.questionKind {
            case "choice": choiceQuestions
            case "plan": plan
            default: permission
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Theme.amber.opacity(0.10)))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Theme.amber.opacity(0.22)))
        .background(NumberKeys(enabled: keys && sent == nil && oneClick, press: pressNumber))
        .onChange(of: agent.promptId) { picks = [:] }
    }

    // MARK: Kinds

    private var permission: some View {
        VStack(alignment: .leading, spacing: 6) {
            heading(agent.question ?? "Permission to continue?")
            if let detail = agent.questionDetail, detail != agent.question {
                RequestDetail(detail: detail)
            }
            ForEach(Array(permissionRows.enumerated()), id: \.offset) { i, row in
                OptionRow(number: i + 1, label: row.label, detail: row.detail, chosen: sent == row.answer, faded: sent != nil) {
                    model.answer(agent, row.answer)
                }
            }
        }
    }

    private var permissionRows: [(label: String, detail: String?, answer: AgentAnswer)] {
        var rows: [(String, String?, AgentAnswer)] = [("Yes", nil, .yes)]
        if let always = agent.choices?.always { rows.append((always, nil, .always)) }
        rows.append(("No", "Then tell @\(agent.handle) what to do instead", .no))
        return rows
    }

    private var plan: some View {
        VStack(alignment: .leading, spacing: 6) {
            heading(agent.question ?? "Its plan is ready. Start on it?")
            if let text = agent.questionDetail, !text.isEmpty {
                ScrollView {
                    Text(markdown(text))
                        .font(.system(size: 12)).foregroundStyle(Theme.ink.opacity(0.9))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 180)
                .fixedSize(horizontal: false, vertical: true)
                .padding(8)
                .background(RoundedRectangle(cornerRadius: 8).fill(Color.black.opacity(0.18)))
            }
            OptionRow(number: 1, label: "Yes, start on it", detail: nil, chosen: sent == .yes, faded: sent != nil) {
                model.answer(agent, .yes)
            }
            OptionRow(number: 2, label: "No, keep planning", detail: "Then tell @\(agent.handle) what to change", chosen: sent == .no, faded: sent != nil) {
                model.answer(agent, .no)
            }
        }
    }

    private var choiceQuestions: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(questions.enumerated()), id: \.offset) { qi, q in
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 6) {
                        if questions.count > 1, let header = q.header {
                            Text(header).font(.system(size: 10, weight: .semibold)).foregroundStyle(Theme.muted)
                                .padding(.horizontal, 6).padding(.vertical, 2)
                                .background(Capsule().fill(Theme.faint))
                        }
                        if q.multiSelect {
                            Text("Pick any").font(.system(size: 10, weight: .medium)).foregroundStyle(Theme.muted)
                        }
                    }
                    heading(q.question, first: qi == 0)
                    ForEach(Array(q.options.enumerated()), id: \.offset) { oi, option in
                        OptionRow(
                            number: oi + 1, label: option.label, detail: option.description,
                            chosen: chosen(qi).contains(oi), faded: sent != nil, tick: q.multiSelect
                        ) { pick(qi, oi) }
                    }
                }
            }
            if !oneClick, sent == nil {
                // Looks disabled until every question has an answer, but still takes the click, so
                // it doesn't open the chat behind the card.
                Button("Send answers") { sendPicks() }
                    .buttonStyle(AmberButtonStyle())
                    .opacity(complete ? 1 : 0.5)
            }
            if sent == nil {
                Button("Don't answer") { model.answer(agent, .no) }
                    .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
                    .help("Close the question in the terminal; @\(agent.handle) is told you didn't answer")
            }
        }
    }

    private func heading(_ text: String, first: Bool = true) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(text)
                .font(.system(size: 13, weight: .medium)).foregroundStyle(Theme.ink)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 4)
            if first, sent != nil {
                Label("Sent", systemImage: "checkmark")
                    .font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.green)
            }
        }
    }

    // MARK: Picking

    private func chosen(_ question: Int) -> Set<Int> {
        if case .picks(let sentPicks) = sent, question < sentPicks.count { return Set(sentPicks[question]) }
        return picks[question] ?? []
    }

    private var complete: Bool {
        questions.indices.allSatisfy { !(picks[$0] ?? []).isEmpty }
    }

    private func pick(_ question: Int, _ option: Int) {
        guard sent == nil else { return }
        if oneClick {
            model.answer(agent, .picks([[option]]))
            return
        }
        if questions[question].multiSelect {
            picks[question, default: []].formSymmetricDifference([option])
        } else {
            picks[question] = [option]
        }
    }

    private func sendPicks() {
        guard complete else { return }
        model.answer(agent, .picks(questions.indices.map { (picks[$0] ?? []).sorted() }))
    }

    private func pressNumber(_ n: Int) -> Bool {
        let i = n - 1
        switch agent.questionKind {
        case "choice":
            guard let q = questions.first, q.options.indices.contains(i) else { return false }
            pick(0, i)
        case "plan":
            guard i < 2 else { return false }
            model.answer(agent, i == 0 ? .yes : .no)
        default:
            let rows = permissionRows
            guard rows.indices.contains(i) else { return false }
            model.answer(agent, rows[i].answer)
        }
        return true
    }
}

/// One option: its number (the key that picks it), its label and what it means.
struct OptionRow: View {
    let number: Int
    let label: String
    let detail: String?
    let chosen: Bool
    var faded = false
    var tick = false
    let action: () -> Void
    @State private var hover = false

    var body: some View {
        Button { if !faded { action() } } label: {
            HStack(alignment: .top, spacing: 9) {
                ZStack {
                    RoundedRectangle(cornerRadius: 5, style: .continuous)
                        .fill(chosen ? Theme.green : Color.white.opacity(0.10))
                    if chosen {
                        Image(systemName: "checkmark").font(.system(size: 10, weight: .bold)).foregroundStyle(.black.opacity(0.8))
                    } else {
                        Text(tick ? "" : "\(number)").font(.system(size: 11, weight: .semibold, design: .rounded))
                            .foregroundStyle(Theme.ink.opacity(0.8))
                    }
                }
                .frame(width: 20, height: 20)
                .overlay(tick && !chosen ? RoundedRectangle(cornerRadius: 5).strokeBorder(Theme.stroke) : nil)
                VStack(alignment: .leading, spacing: 1) {
                    Text(label).font(.system(size: 12.5, weight: .medium)).foregroundStyle(Theme.ink)
                        .fixedSize(horizontal: false, vertical: true)
                    if let detail, !detail.isEmpty {
                        Text(detail).font(.system(size: 11)).foregroundStyle(Theme.muted)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .padding(.top, 1)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 8).padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .fill(chosen ? Theme.green.opacity(0.18) : hover && !faded ? Color.white.opacity(0.09) : Color.white.opacity(0.05))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .strokeBorder(chosen ? Theme.green.opacity(0.75) : Theme.hairline)
            )
            .opacity(faded && !chosen ? 0.45 : 1)
            .contentShape(Rectangle())
        }
        // Answered, it still takes clicks (so they don't open the chat behind) and does nothing.
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .accessibilityLabel(tick ? label : "\(number). \(label)")
    }
}

/// Only listens for keys: clicks go to the card's options in front of it (an AppKit view behind
/// SwiftUI content would otherwise take them).
private final class ClickThrough: NSView {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// Number keys for the card in this window, except while there's text being typed in a box.
private struct NumberKeys: NSViewRepresentable {
    let enabled: Bool
    let press: (Int) -> Bool

    func makeNSView(context: Context) -> NSView {
        let view = ClickThrough()
        context.coordinator.view = view
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.press = enabled ? press : nil
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.stop()
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    @MainActor final class Coordinator {
        weak var view: NSView?
        var press: ((Int) -> Bool)? {
            didSet { press == nil ? stop() : start() }
        }
        private var monitor: Any?

        func start() {
            guard monitor == nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                let handled = MainActor.assumeIsolated { self?.handle(event) ?? false }
                return handled ? nil : event
            }
        }

        func stop() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
        }

        private func handle(_ event: NSEvent) -> Bool {
            guard let press, let window = view?.window, event.window === window,
                  event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
                  // Typing a reply: numbers are text. An empty box still answers, as the terminal does.
                  (window.firstResponder as? NSText)?.string.isEmpty ?? true,
                  let chars = event.charactersIgnoringModifiers, let n = Int(chars), (1...9).contains(n)
            else { return false }
            return press(n)
        }
    }
}

extension QuestionCard {
    /// Sample questions for `--snapshot-questions`: a permission with "always", Claude's questions
    /// (one, two with a pick-many, and one already answered, p5) and a plan.
    static func samples() -> [(String, Agent)] {
        func agent(_ id: String, _ kind: String, question: String, detail: String? = nil, choices: String) -> Agent {
            let json = """
            {"id":"claude:\(id):1","kind":"claude","handle":"claude-\(id)","pid":1,"state":"needs_input","canReply":true,
             "stateSource":"hook","questionKind":"\(kind)","question":\(String(reflecting: question)),"promptId":"p\(id)",
             "questionDetail":\(detail.map { String(reflecting: $0) } ?? "null"),"choices":\(choices),
             "route":{"via":"tmux"}}
            """
            return try! JSONDecoder().decode(Agent.self, from: Data(json.utf8))
        }
        return [
            ("permission", agent("1", "permission", question: "Permission to run Bash · npm install stripe", detail: "npm install stripe",
                                 choices: #"{"always":"Always allow npm install:*"}"#)),
            ("choice", agent("2", "choice", question: "Which layout?", choices: #"""
                {"questions":[{"question":"Which layout?","header":"Layout","multiSelect":false,"options":[
                {"label":"Three tiers","description":"Free, Pro and Max side by side"},{"label":"One plan"},{"label":"A table"}]}]}
                """#)),
            ("choices", agent("3", "choice", question: "Ship to all three screens? (+1 more)", choices: #"""
                {"questions":[{"question":"Ship to all three screens?","header":"Screens","multiSelect":false,"options":[
                {"label":"Yes"},{"label":"Just one"},{"label":"Not yet"}]},
                {"question":"Which checks should run first?","header":"Checks","multiSelect":true,"options":[
                {"label":"Unit tests"},{"label":"Screenshots"},{"label":"Accessibility"}]}]}
                """#)),
            ("choice-sent", agent("5", "choice", question: "Which layout?", choices: #"""
                {"questions":[{"question":"Which layout?","multiSelect":false,"options":[{"label":"Three tiers"},{"label":"One plan"},{"label":"A table"}]}]}
                """#)),
            ("plan", agent("4", "plan", question: "Its plan is ready. Start on it?",
                           detail: "1. Add a `pricing` page with three tiers\n2. Link it from the header\n3. Add screenshot tests", choices: "null")),
        ]
    }
}
