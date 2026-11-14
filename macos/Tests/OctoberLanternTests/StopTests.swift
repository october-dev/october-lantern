@testable import OctoberLantern
import XCTest

/// Stop ends Lantern's answer while it comes in, else sends the working agent Escape — except
/// Grok, which Escape doesn't stop: no Stop for a working Grok, and no "Stopped" claim.
@MainActor
final class StopTests: XCTestCase {
    /// Held here: Ask holds its model weakly.
    private var model: AppModel!

    private func agent(_ id: String, kind: String, state: String) -> Agent {
        let json = """
        {"id":"\(id)","kind":"\(kind)","handle":"\(id)","pid":1,"state":"\(state)","canReply":true,
         "route":{"via":"tmux"},"stateSource":"hook"}
        """
        return try! JSONDecoder().decode(Agent.self, from: Data(json.utf8))
    }

    private func make(_ agents: [Agent]) -> Assistant {
        model = AppModel()
        model.update(agents)
        return Assistant(testing: model, seams: .init(
            readNow: { _, _ in },
            reply: { _, _ in },
            route: { _, _, _, _ in }
        ))
    }

    /// A chat whose task is on `agentId`.
    private func chat(on agentId: String) -> Assistant.Conversation {
        var link = Assistant.TaskLink(kind: "claude", folder: "/tmp", prompt: "Make a poster", agentId: agentId)
        link.startedAt = Date().addingTimeInterval(-60)
        let turn = Assistant.Turn(question: "Make a poster", answer: "On it.", task: link)
        return Assistant.Conversation(title: "Make a poster", updated: Date(), turns: [turn])
    }

    func testWorkingGrokIsNotStoppableAndStopSendsNothing() {
        let ask = make([agent("grok:9:9", kind: "grok", state: "working")])
        ask.open(chat(on: "grok:9:9"))
        XCTAssertFalse(ask.stoppable, "no Stop for a working Grok")
        ask.stop()
        XCTAssertNil(model.toast, "no key sent, no Stopped toast")
    }

    func testWorkingClaudeStaysStoppableAndStopSendsEscape() {
        let ask = make([agent("claude:9:9", kind: "claude", state: "working")])
        ask.open(chat(on: "claude:9:9"))
        XCTAssertTrue(ask.stoppable)
        ask.stop()
        XCTAssertTrue(
            model.toast?.contains("Couldn't stop @claude:9:9") ?? false,
            "Escape was attempted (the engine is down in tests, so it fails): \(model.toast ?? "no toast")"
        )
    }
}
