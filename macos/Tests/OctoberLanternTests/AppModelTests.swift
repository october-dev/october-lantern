@testable import OctoberLantern
import XCTest

@MainActor
final class AppModelTests: XCTestCase {
    private func agent(_ id: String) -> Agent {
        let json = """
        {"id":"\(id)","kind":"claude","handle":"\(id)","pid":1,"state":"waiting","canReply":true,
         "route":{"via":"tmux"},"stateSource":"hook"}
        """
        return try! JSONDecoder().decode(Agent.self, from: Data(json.utf8))
    }

    /// An engine restart reports no agents for a moment; drafts for every conversation survive it.
    func testEngineRestartKeepsEveryDraft() {
        let m = AppModel()
        let (a, b) = (agent("a"), agent("b"))
        m.update([a, b])
        m.openChat(a)
        m.draft = "for a"
        m.openChat(b)
        m.draft = "for b"
        m.engineStopped("Lantern's engine stopped.")
        m.update([a, b])
        m.openChat(a)
        XCTAssertEqual(m.draft, "for a")
        m.openChat(b)
        XCTAssertEqual(m.draft, "for b")
    }

    /// No answer in time: not reported as failed and the draft stays; a late "sent" still clears it.
    func testTimedOutReplyStaysUncertainAndLateAnswerApplies() {
        let m = AppModel()
        let a = agent("a")
        m.update([a])
        m.openChat(a)
        m.draft = "deploy"
        let ticket = m.drafts.ticket()!
        m.track("r1", .reply(ticket), sent: true)
        XCTAssertTrue(m.sending)
        m.expire("r1")
        XCTAssertFalse(m.sending)
        XCTAssertEqual(m.draft, "deploy")
        XCTAssertTrue(m.toast?.contains("check the terminal") ?? false, m.toast ?? "")
        m.replyFinished("r1", ok: true, uncertain: false, message: nil)
        XCTAssertEqual(m.draft, "")
    }
}
