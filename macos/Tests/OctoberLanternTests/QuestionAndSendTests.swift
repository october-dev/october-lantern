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

    /// Typed in, a message arrives on one line (the engine turns line breaks into spaces), so a
    /// follow-up whose first lines were broken must still be found, or its answer never shows in
    /// Ask Lantern (issue #10).
    func testAFollowUpTypedOnOneLineStillFindsItsAnswer() {
        let sent = Date(timeIntervalSince1970: 1_800_000_000)
        let iso = { (offset: Double) in ISO8601DateFormatter().string(from: sent.addingTimeInterval(offset)) }
        let prompt = "Next:\nwrite a casual message\n\nto go with the posters."
        let earlier = ChatMessage(role: .agent, text: "I rewrote the posters.", at: iso(-60))
        let typed = ChatMessage(role: .user, text: "Next: write a casual message  to go with the posters.", at: iso(1))
        let answer = ChatMessage(role: .agent, text: "Here's a laid-back one.", at: iso(5))
        XCTAssertEqual(Assistant.after(sent: prompt, since: sent, in: [earlier, typed, answer]), [answer])
        let stale = ChatMessage(role: .user, text: typed.text, at: iso(-600))
        XCTAssertEqual(Assistant.after(sent: prompt, since: sent, in: [stale, earlier]), [], "an older copy isn't this send")
        XCTAssertEqual(Assistant.oneLine("  a\n\n b\tc  "), "a b c")
    }

    /// An approval or a choice is answered with keys: Ask Lantern sends you to its terminal for
    /// those rather than offering to type a reply into the prompt.
    func testQuestionsAnsweredWithKeysAreNotOfferedAReply() {
        XCTAssertTrue(Assistant.answeredInTerminal("permission"))
        XCTAssertTrue(Assistant.answeredInTerminal("plan"))
        XCTAssertTrue(Assistant.answeredInTerminal("choice"))
        XCTAssertTrue(Assistant.answeredInTerminal("other"), "a form or question the engine reads as other")
        XCTAssertFalse(Assistant.answeredInTerminal(nil))
    }

    // MARK: Replies stay with their task as the engine's window moves on (issue #10)

    private let sent = Date(timeIntervalSince1970: 1_800_000_000)
    private func iso(_ offset: Double) -> String { ISO8601DateFormatter().string(from: sent.addingTimeInterval(offset)) }
    private func step(_ n: Int) -> ChatMessage { ChatMessage(role: .tool, text: "Bash: step \(n)", at: iso(Double(n) + 2)) }
    private func routed(_ prompt: String = "Also add tests") -> Assistant.TaskLink {
        var link = Assistant.TaskLink(kind: "claude", folder: "/tmp", prompt: prompt, agentId: "claude:1:1")
        link.sent = true
        link.sentAt = sent
        return link
    }

    /// The send, 59 steps and the answer: what's saved is only what came after the send, so the
    /// answer is still there once the agent exits or a newer task takes over.
    func testASavedRoutedTaskKeepsItsAnswerPastSixtyEntries() {
        let before = ChatMessage(role: .agent, text: "Earlier work.", at: iso(-60))
        let typed = ChatMessage(role: .user, text: "Also add tests", at: iso(1))
        let answer = ChatMessage(role: .agent, text: "Tests added.", at: iso(70))
        let window = [before, typed] + (1...59).map(step) + [answer]
        let link = Assistant.keeping(window, in: routed())
        XCTAssertEqual(link.found, true)
        XCTAssertFalse(link.transcript.contains(before), "what came before the send isn't this task's")
        XCTAssertFalse(link.transcript.contains(typed))
        // Saved (the agent exited, or this is an earlier task now), and reopened from disk.
        let reopened = try! JSONDecoder().decode(Assistant.TaskLink.self, from: JSONEncoder().encode(link))
        XCTAssertEqual(Assistant.taskMessages(reopened, live: nil).filter { $0.role != .tool }, [answer])
        XCTAssertEqual(Assistant.taskMessages(reopened, live: nil).filter { $0.role == .tool }.count, 59)
    }

    /// Live, the send drops out of the engine's 120 once 119 steps follow it: the answer still
    /// shows, and still does once it has itself left the window.
    func testALiveRoutedTaskKeepsItsAnswerOnceTheSendLeavesTheWindow() {
        let typed = ChatMessage(role: .user, text: "Also add tests", at: iso(1))
        let first = [typed] + (1...10).map(step)
        var link = Assistant.keeping(first, in: routed())
        let answer = ChatMessage(role: .agent, text: "Tests added.", at: iso(200))
        let full = Array(([typed] + (1...119).map(step) + [answer]).suffix(120))
        XCTAssertFalse(full.contains(typed), "the send has left the window")
        XCTAssertEqual(Assistant.taskMessages(link, live: full).last, answer)
        link = Assistant.keeping(full, in: link)
        let followUp = ChatMessage(role: .user, text: "And the docs", at: iso(400))
        let later = Array((full + (120...260).map(step) + [followUp, ChatMessage(role: .agent, text: "Docs done.", at: iso(500))]).suffix(120))
        XCTAssertFalse(later.contains(answer), "the answer has left the window too")
        let shown = Assistant.taskMessages(link, live: later).filter { $0.role != .tool }.map(\.text)
        XCTAssertEqual(shown, ["Tests added.", "And the docs", "Docs done."], "kept in order, once each")
    }

    /// Until the send shows up nothing counts, and a stale message never does.
    func testARoutedTaskShowsNothingBeforeItsSendAndNoStaleMessages() {
        let old = ChatMessage(role: .agent, text: "Old answer.", at: iso(-600))
        let unseen = Assistant.keeping([old] + (1...5).map(step), in: routed())
        XCTAssertNil(unseen.found)
        XCTAssertTrue(unseen.transcript.isEmpty, "no unfiltered history on a failed match")
        var seen = routed()
        seen.found = true
        XCTAssertEqual(Assistant.replies(to: seen, in: [old, step(1)]), [step(1)])
    }

    /// A newer task on the same agent: the earlier card keeps what it had, and the newer one
    /// starts from its own send.
    func testANewTaskOnTheSameAgentStartsFromItsOwnSend() {
        let one = ChatMessage(role: .user, text: "Also add tests", at: iso(1))
        let oneAnswer = ChatMessage(role: .agent, text: "Tests added.", at: iso(5))
        let two = ChatMessage(role: .user, text: "Now the docs", at: iso(10))
        let twoAnswer = ChatMessage(role: .agent, text: "Docs done.", at: iso(15))
        let earlier = Assistant.keeping([one, oneAnswer], in: routed())
        var newer = routed("Now the docs")
        newer.sentAt = sent.addingTimeInterval(9)
        XCTAssertEqual(Assistant.taskMessages(newer, live: [one, oneAnswer, two, twoAnswer]), [twoAnswer])
        XCTAssertEqual(Assistant.taskMessages(earlier, live: nil), [oneAnswer])
    }

    /// A task Lantern started keeps answers as its window moves on too, without its first message.
    func testAStartedTaskKeepsItsAnswersAsTheWindowMoves() {
        var link = Assistant.TaskLink(kind: "claude", folder: "/tmp", prompt: "Make a poster", agentId: "claude:1:1")
        let prompt = ChatMessage(role: .user, text: "Make a poster", at: iso(0))
        let answer = ChatMessage(role: .agent, text: "Poster made.", at: iso(3))
        link = Assistant.keeping([prompt, answer], in: link)
        let moved = (1...120).map(step)
        link = Assistant.keeping(moved, in: link)
        XCTAssertEqual(Assistant.taskMessages(link, live: nil).filter { $0.role != .tool }, [answer])
        XCTAssertEqual(Assistant.taskMessages(link, live: nil).filter { $0.role == .tool }.count, 60)
    }

    func testMergingAddsOnlyWhatsNew() {
        let a = (1...20).map(step)
        XCTAssertEqual(Assistant.merged(Array(a[0..<10]), Array(a[5..<20])), a)
        XCTAssertEqual(Assistant.merged(Array(a[0..<10]), Array(a[8..<20])), a, "the window moved past most of the saved tail")
        XCTAssertEqual(Assistant.merged([], Array(a[0..<3])), Array(a[0..<3]))
        XCTAssertEqual(Assistant.merged(Array(a[0..<3]), Array(a[0..<3])), Array(a[0..<3]))
    }
}
