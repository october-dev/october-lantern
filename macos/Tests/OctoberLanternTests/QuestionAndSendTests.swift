import XCTest
@testable import OctoberLantern

@MainActor
final class QuestionAndSendTests: XCTestCase {
    private func agent(_ json: String) throws -> Agent {
        try JSONDecoder().decode(Agent.self, from: Data(json.utf8))
    }

    func testOnlyQuestionsLanternCanPressKeysForGetTheCard() throws {
        let base = #""id":"claude:1:1","kind":"claude","handle":"claude-1","pid":1,"canReply":true,"stateSource":"hook","state":"needs_input""#
        let choice = try agent("{\(base),\"questionKind\":\"choice\",\"promptId\":\"t-1\",\"route\":{\"via\":\"tmux\"},"
            + #""choices":{"questions":[{"question":"Which?","multiSelect":false,"options":[{"label":"A"},{"label":"B"}]}]}}"#)
        XCTAssertTrue(choice.isAnswerable)
        XCTAssertEqual(choice.choices?.questions.first?.options.map(\.label), ["A", "B"])
        // No options to show, or a terminal Lantern can't press keys in: the plain question.
        XCTAssertFalse(try agent("{\(base),\"questionKind\":\"choice\",\"promptId\":\"t-1\",\"route\":{\"via\":\"tmux\"}}").isAnswerable)
        XCTAssertFalse(try agent("{\(base),\"questionKind\":\"plan\",\"promptId\":\"t-1\",\"route\":{\"via\":\"terminal\"}}").isAnswerable)
        XCTAssertTrue(try agent("{\(base),\"questionKind\":\"plan\",\"promptId\":\"t-1\",\"route\":{\"via\":\"cmux\"}}").isAnswerable)
        XCTAssertFalse(try agent("{\(base),\"questionKind\":\"other\",\"promptId\":\"t-1\",\"route\":{\"via\":\"tmux\"}}").isAnswerable)
    }

    func testAnswersAreSentAsTheEnginesAnswer() throws {
        func json(_ answer: AgentAnswer) throws -> String {
            String(decoding: try JSONSerialization.data(withJSONObject: answer.json, options: .sortedKeys), as: UTF8.self)
        }
        XCTAssertEqual(try json(.yes), #"{"kind":"yes"}"#)
        XCTAssertEqual(try json(.always), #"{"kind":"always"}"#)
        XCTAssertEqual(try json(.picks([[1], [0, 2]])), #"{"kind":"picks","picks":[[1],[0,2]]}"#)
    }

    func testAnAgentItWasSentToShowsOnlyWhatCameAfter() {
        let earlier = ChatMessage(role: .user, text: "Build the pricing page", at: nil)
        let done = ChatMessage(role: .agent, text: "Done.", at: nil)
        let nudge = ChatMessage(role: .user, text: "I sent you a message on October Bus. Check your inbox (check_inbox) and do what it asks.", at: nil)
        let after = ChatMessage(role: .agent, text: "Adding the tests now.", at: nil)
        let sent = Date(timeIntervalSince1970: 1_800_000_000)
        let iso = { (offset: Double) in ISO8601DateFormatter().string(from: sent.addingTimeInterval(offset)) }
        let nudgeNow = ChatMessage(role: .user, text: nudge.text, at: iso(1))
        XCTAssertEqual(Assistant.after(sent: "Also add tests", since: sent, in: [earlier, done, nudgeNow, after]), [after])
        let typed = ChatMessage(role: .user, text: "Also add tests", at: nil)
        XCTAssertEqual(Assistant.after(sent: "Also add tests", since: sent, in: [earlier, done, typed, after]), [after])
        XCTAssertEqual(Assistant.after(sent: "Also add tests", since: sent, in: [earlier, done]), [], "not there yet")
    }

    /// Every send's nudge has the same words: an older one (or one without a time) isn't this send's.
    func testAnOlderNudgeDoesntPassForANewSend() {
        let sent = Date(timeIntervalSince1970: 1_800_000_000)
        let old = ChatMessage(role: .user, text: "I sent you a message on October Bus. Check your inbox (check_inbox) and do what it asks.",
                              at: ISO8601DateFormatter().string(from: sent.addingTimeInterval(-600)))
        let oldAnswer = ChatMessage(role: .agent, text: "I finished OLD TASK.", at: nil)
        XCTAssertEqual(Assistant.after(sent: "New task", since: sent, in: [old, oldAnswer]), [])
        let untimed = ChatMessage(role: .user, text: old.text, at: nil)
        XCTAssertEqual(Assistant.after(sent: "New task", since: sent, in: [untimed, oldAnswer]), [])
    }
}
