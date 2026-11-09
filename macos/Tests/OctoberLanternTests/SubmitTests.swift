@testable import OctoberLantern
import XCTest

/// Sending to the agent of Ask's latest task: the message waits on a read of the agent's
/// conversation (to know where it goes), and only what was submitted goes, only to that agent,
/// only while it's still that chat's. Driven through `submit`, with reads answered late.
@MainActor
final class SubmitTests: XCTestCase {
    private var reads: [(agentId: String, done: (HistoryRead?) -> Void)] = []
    private var replies: [(agentId: String, text: String)] = []
    private var routes: [(agentId: String, text: String)] = []
    private let x = URL(fileURLWithPath: "/tmp/x.png")
    private let y = URL(fileURLWithPath: "/tmp/y.png")
    /// Held here: Ask holds its model weakly.
    private var model: AppModel!

    private func agent(_ id: String) -> Agent {
        let json = """
        {"id":"\(id)","kind":"claude","handle":"\(id)","pid":1,"state":"waiting","canReply":true,
         "route":{"via":"tmux"},"stateSource":"hook"}
        """
        return try! JSONDecoder().decode(Agent.self, from: Data(json.utf8))
    }

    private func make() -> Assistant {
        model = AppModel()
        model.update([agent("a"), agent("b")])
        return Assistant(testing: model, seams: .init(
            readNow: { [unowned self] id, done in reads.append((id, done)) },
            reply: { [unowned self] text, agent in replies.append((agent.id, text)) },
            route: { [unowned self] text, agent, _, report in routes.append((agent.id, text)); report(true, false, "Sent") }
        ))
    }

    /// A chat whose task is on `agentId`, placed in session `source`.
    private func chat(on agentId: String, source: String = "A") -> Assistant.Conversation {
        var link = Assistant.TaskLink(kind: "claude", folder: "/tmp", prompt: "Make a poster", agentId: agentId)
        link.startedAt = Date().addingTimeInterval(-60)
        link.start = ChatMessage(role: .user, text: "Make a poster", at: nil, id: "s")
        link.source = source
        let turn = Assistant.Turn(question: "Make a poster", answer: "On it.", task: link)
        return Assistant.Conversation(title: "Make a poster", updated: Date(), turns: [turn])
    }

    private func read(_ source: String?, _ messages: [ChatMessage] = []) -> HistoryRead {
        HistoryRead(messages: messages, source: source)
    }

    private func at(_ offset: TimeInterval) -> String {
        ISO8601DateFormatter().string(from: Date().addingTimeInterval(offset))
    }

    func testWhatGoesIsWhatWasSubmittedNotWhatsInTheBoxLater() {
        let ask = make()
        ask.open(chat(on: "a"))
        ask.text = "first"
        ask.attach([x])
        ask.submit()
        XCTAssertEqual(reads.map(\.agentId), ["a"])
        XCTAssertTrue(ask.checking)
        XCTAssertEqual(ask.text, "", "taken out of the box when submitted")
        XCTAssertTrue(ask.attachments.isEmpty)
        // Typed and added while the read is out: not part of it.
        ask.text = "second"
        ask.attach([y])
        reads[0].done(read("A"))
        XCTAssertEqual(replies.map(\.agentId), ["a"])
        XCTAssertEqual(replies.first?.text, "first" + Assistant.imageLines([x]))
        XCTAssertEqual(ask.text, "second")
        XCTAssertEqual(ask.attachments, [y])
        XCTAssertFalse(ask.checking)
    }

    func testANewChatWhileCheckingSendsNothingAndKeepsTheMessageWithItsChat() {
        let ask = make()
        let a = chat(on: "a")
        ask.open(a)
        ask.text = "first"
        ask.submit()
        ask.clear()
        XCTAssertFalse(ask.checking)
        reads[0].done(read("A"))
        XCTAssertTrue(replies.isEmpty && routes.isEmpty, "a read for a cancelled send sends nothing")
        XCTAssertEqual(ask.text, "", "a new chat doesn't get it")
        ask.open(a)
        XCTAssertEqual(ask.text, "first", "back in the box of its own chat")
        XCTAssertNotNil(ask.attachProblem)
    }

    func testOpeningAnotherAgentsChatWhileCheckingSendsToNeither() {
        let ask = make()
        let (a, b) = (chat(on: "a"), chat(on: "b"))
        ask.open(a)
        ask.text = "for a"
        ask.submit()
        ask.open(b)
        XCTAssertEqual(ask.activeAgent?.id, "b")
        XCTAssertEqual(ask.text, "")
        reads[0].done(read("A"))
        XCTAssertTrue(replies.isEmpty && routes.isEmpty)
        // B's own message goes to B, with B's own read.
        ask.text = "for b"
        ask.submit()
        XCTAssertEqual(reads.last?.agentId, "b")
        reads.last?.done(read("A"))
        XCTAssertEqual(replies.map(\.agentId), ["b"])
        XCTAssertEqual(replies.map(\.text), ["for b"])
        ask.open(a)
        XCTAssertEqual(ask.text, "for a")
    }

    func testOnlyOneSendWaitsAndALateDuplicateAnswerDoesNothing() {
        let ask = make()
        ask.open(chat(on: "a"))
        ask.text = "first"
        ask.submit()
        ask.text = "second"
        ask.submit()
        XCTAssertEqual(reads.count, 1, "a second send waits for the first")
        XCTAssertEqual(ask.text, "second", "and stays in the box")
        reads[0].done(read("A"))
        reads[0].done(read("A"))
        XCTAssertEqual(replies.map(\.text), ["first"])
    }

    func testWithoutAReadNothingGoesAndTheMessageComesBack() {
        let ask = make()
        ask.open(chat(on: "a"))
        ask.text = "first"
        ask.attach([x])
        ask.submit()
        ask.text = "more"
        reads[0].done(nil)
        XCTAssertTrue(replies.isEmpty && routes.isEmpty)
        XCTAssertEqual(ask.text, "first\n\nmore", "back before what was typed since")
        XCTAssertEqual(ask.attachments, [x])
        XCTAssertNotNil(ask.attachProblem)
        XCTAssertFalse(ask.checking)
    }

    func testANewConversationGetsItsOwnTaskWithThatReadAsItsBaseline() {
        let ask = make()
        ask.open(chat(on: "a"))
        ask.text = "next step"
        ask.submit()
        ask.text = "typed meanwhile"
        reads[0].done(read("B", [ChatMessage(role: .user, text: "old", at: nil, id: "b0")]))
        XCTAssertTrue(replies.isEmpty)
        XCTAssertEqual(routes.map(\.agentId), ["a"])
        XCTAssertEqual(routes.map(\.text), ["next step"])
        let task = ask.turns.last?.task
        XCTAssertEqual(ask.turns.count, 2)
        XCTAssertEqual(task?.prompt, "next step")
        XCTAssertEqual(task?.baselineSource, "B")
        XCTAssertEqual(task?.baseline, ["b0"])
        XCTAssertEqual(task?.delivery, "Sent")
        XCTAssertEqual(ask.text, "typed meanwhile")
    }

    /// Last poll A, process now in B, the read before sending still says A: the reply is typed
    /// into B. The task keeps A, and says its message isn't there rather than wait for an answer
    /// that won't show.
    func testAReplyThatWentIntoAnotherConversationIsSaidToBeMissing() throws {
        let ask = make()
        ask.open(chat(on: "a"))
        ask.text = "and a title"
        ask.submit()
        reads[0].done(read("A"))
        XCTAssertEqual(replies.map(\.text), ["and a title"])
        let link = try XCTUnwrap(ask.turns.last?.task)
        XCTAssertEqual(link.replies?.map(\.text), ["and a title"])

        let inB = read("B", [ChatMessage(role: .user, text: "and a title", at: at(1), id: "b1"),
                             ChatMessage(role: .agent, text: "Titled.", at: at(2), id: "b2")])
        let shown = Assistant.taskMessages(link, live: inB)
        XCTAssertEqual(Assistant.missingReply(link, shown: shown, live: inB, working: true)?.text, "and a title",
                       "moved on: it's missing even while the agent works")
        XCTAssertEqual(Assistant.keeping(inB, in: link), link, "A's conversation is kept as it was")

        // Still in A, not there yet: not missing while it could show up.
        let stillA = read("A", [ChatMessage(role: .user, text: "Make a poster", at: nil, id: "s")])
        XCTAssertNil(Assistant.missingReply(link, shown: Assistant.taskMessages(link, live: stillA), live: stillA, working: false))
        XCTAssertNotNil(Assistant.missingReply(link, shown: Assistant.taskMessages(link, live: stillA), live: stillA, working: false,
                                               now: Date().addingTimeInterval(60)), "idle half a minute later")
    }

    /// The reply went in where the task is; after a `/clear` (and after it leaves what's kept) it
    /// isn't said to be missing.
    func testAReplyFoundOnceStaysFound() throws {
        let ask = make()
        ask.open(chat(on: "a"))
        ask.text = "and a title"
        ask.submit()
        reads[0].done(read("A"))
        var link = try XCTUnwrap(ask.turns.last?.task)
        let inA = read("A", [ChatMessage(role: .user, text: "Make a poster", at: nil, id: "s"),
                             ChatMessage(role: .user, text: "and a title", at: at(1), id: "a1"),
                             ChatMessage(role: .agent, text: "Titled.", at: at(2), id: "a2")])
        link = Assistant.keeping(inA, in: link)
        XCTAssertEqual(link.replies?.last?.seen, true)
        XCTAssertEqual(Assistant.taskMessages(link, live: nil).map(\.text), ["and a title", "Titled."])
        link.transcript = []
        let cleared = read("C")
        XCTAssertNil(Assistant.missingReply(link, shown: link.transcript, live: cleared, working: false,
                                            now: Date().addingTimeInterval(60)))
    }

    /// Saved replies (and chats from before them) read back.
    func testRepliesAreSavedWithTheTask() throws {
        var link = Assistant.TaskLink(kind: "claude", folder: "/tmp", prompt: "p")
        link.replies = [Assistant.Reply(text: "r", at: Date(timeIntervalSince1970: 100), seen: true)]
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        XCTAssertEqual(try decoder.decode(Assistant.TaskLink.self, from: encoder.encode(link)), link)
        let old = #"{"kind":"claude","folder":"/tmp","prompt":"p","inTerminal":false,"transcript":[]}"#
        XCTAssertNil(try decoder.decode(Assistant.TaskLink.self, from: Data(old.utf8)).replies)
    }
}
