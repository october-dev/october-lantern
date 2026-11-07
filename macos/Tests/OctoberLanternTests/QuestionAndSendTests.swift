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

    // MARK: A task's conversation, reconciled read by read (issue #10, #24 audits)

    private let sent = Date(timeIntervalSince1970: 1_800_000_000)
    private func iso(_ offset: Double) -> String { ISO8601DateFormatter().string(from: sent.addingTimeInterval(offset)) }
    private func step(_ n: Int) -> ChatMessage { ChatMessage(role: .tool, text: "Bash: step \(n)", at: iso(Double(n) + 2), id: "s\(n)") }
    private func msg(_ id: String, _ role: ChatMessage.Role, _ text: String, _ offset: Double?) -> ChatMessage {
        ChatMessage(role: role, text: text, at: offset.map(iso), id: id)
    }
    private func read(_ messages: [ChatMessage], _ source: String? = "A", complete: Bool = false) -> HistoryRead {
        HistoryRead(messages: messages, source: source, complete: complete)
    }
    private func routed(_ prompt: String = "Also add tests", at offset: Double = 0) -> Assistant.TaskLink {
        var link = Assistant.TaskLink(kind: "claude", folder: "/tmp", prompt: prompt, agentId: "claude:1:1")
        link.sent = true
        link.sentAt = sent.addingTimeInterval(offset)
        return link
    }
    private func started(_ prompt: String = "Make a poster") -> Assistant.TaskLink {
        var link = Assistant.TaskLink(kind: "claude", folder: "/tmp", prompt: prompt, agentId: "claude:1:1")
        link.startedAt = sent
        return link
    }
    private func reopened(_ link: Assistant.TaskLink) -> Assistant.TaskLink {
        try! JSONDecoder().decode(Assistant.TaskLink.self, from: JSONEncoder().encode(link))
    }
    private func texts(_ messages: [ChatMessage]) -> [String] { messages.filter { $0.role != .tool }.map(\.text) }

    /// The send, 59 steps and the answer: what's kept is only what came after the send, so the
    /// answer is still there once the agent exits or a newer task takes over.
    func testASavedRoutedTaskKeepsItsAnswerPastSixtyEntries() {
        let before = msg("b", .agent, "Earlier work.", -60)
        let typed = msg("u", .user, "Also add tests", 1)
        let answer = msg("a", .agent, "Tests added.", 70)
        let link = Assistant.keeping(read([before, typed] + (1...59).map(step) + [answer]), in: routed())
        XCTAssertEqual(link.start, typed)
        XCTAssertFalse(link.transcript.contains(before), "what came before the send isn't this task's")
        let saved = Assistant.taskMessages(reopened(link), live: nil)
        XCTAssertEqual(texts(saved), ["Tests added."])
        XCTAssertEqual(saved.filter { $0.role == .tool }.count, 59)
    }

    /// Live, the send drops out of the engine's 120 once 119 steps follow it: the answer still
    /// shows, and still does once it has itself left the window, with what follows once each.
    func testALiveRoutedTaskKeepsItsAnswerOnceTheSendLeavesTheWindow() {
        let typed = msg("u", .user, "Also add tests", 1)
        var link = Assistant.keeping(read([typed] + (1...10).map(step)), in: routed())
        let answer = msg("a", .agent, "Tests added.", 200)
        let full = Array(([typed] + (1...119).map(step) + [answer]).suffix(120))
        XCTAssertFalse(full.contains(typed), "the send has left the window")
        XCTAssertEqual(Assistant.taskMessages(link, live: read(full)).last, answer)
        link = Assistant.keeping(read(full), in: link)
        let followUp = msg("f", .user, "And the docs", 400)
        let later = Array((full + (120...260).map(step) + [followUp, msg("d", .agent, "Docs done.", 500)]).suffix(120))
        XCTAssertFalse(later.contains(answer), "the answer has left the window too")
        XCTAssertEqual(texts(Assistant.taskMessages(link, live: read(later))), ["Tests added.", "And the docs", "Docs done."],
                       "a window that moved on with nothing in common keeps the older answers, in order, once each")
    }

    /// Until the send shows up nothing counts, and a stale message never does.
    func testARoutedTaskShowsNothingBeforeItsSendAndNoStaleMessages() {
        let old = msg("o", .agent, "Old answer.", -600)
        let unseen = Assistant.keeping(read([old] + (1...5).map(step)), in: routed())
        XCTAssertNil(unseen.start)
        XCTAssertTrue(unseen.transcript.isEmpty, "no unfiltered history on a failed match")
        XCTAssertTrue(Assistant.taskMessages(unseen, live: read([old])).isEmpty)
        // Once its send has left a window that moved on, older messages still don't count.
        var placed = Assistant.keeping(read([msg("u", .user, "Also add tests", 1)]), in: routed())
        placed = Assistant.keeping(read([old, step(1)]), in: placed)
        XCTAssertEqual(placed.transcript, [step(1)])
    }

    /// A newer task on the same agent starts from its own send; the earlier card keeps its own.
    /// A later copy of the earlier send's words isn't its boundary.
    func testANewTaskOnTheSameAgentStartsFromItsOwnSend() {
        let one = msg("1", .user, "Also add tests", 1)
        let oneAnswer = msg("2", .agent, "Tests added.", 5)
        let two = msg("3", .user, "Now the docs", 10)
        let twoAnswer = msg("4", .agent, "Docs done.", 15)
        let earlier = Assistant.keeping(read([one, oneAnswer]), in: routed())
        XCTAssertEqual(Assistant.taskMessages(routed("Now the docs", at: 9), live: read([one, oneAnswer, two, twoAnswer])), [twoAnswer])
        XCTAssertEqual(Assistant.taskMessages(earlier, live: nil), [oneAnswer])
        let again = msg("5", .user, "Also add tests", 20)
        let later = Assistant.keeping(read([one, oneAnswer, again, msg("6", .agent, "Already done.", 25)]), in: earlier)
        XCTAssertEqual(texts(later.transcript), ["Tests added.", "Also add tests", "Already done."], "its start is still the first send")
    }

    /// A task Lantern started keeps its answer as the window moves on, without its prompt.
    func testAStartedTaskKeepsItsAnswersAsTheWindowMoves() {
        let prompt = msg("p", .user, "Make a poster", 0)
        let answer = msg("a", .agent, "Poster made.", 3)
        var link = Assistant.keeping(read([prompt, answer]), in: started())
        link = Assistant.keeping(read((1...120).map(step)), in: link)
        XCTAssertEqual(texts(Assistant.taskMessages(link, live: nil)), ["Poster made."])
        XCTAssertEqual(Assistant.taskMessages(link, live: nil).filter { $0.role == .tool }.count, 60)
    }

    /// The prompt is found by its first words: Lantern's recipe line before it, its notes and
    /// image paths after it, or the engine shortening it, don't hide it.
    func testAStartedTasksPromptIsFoundWithLanternsAdditions() {
        let prompt = String(repeating: "Make a poster for the spring fair with big letters. ", count: 200)
        let given = Assistant.recipeLead + " the poster recipe.\n\n" + String(prompt.prefix(7000))
            + "\n\nYou're connected to October Bus…\n\nFor context, a screenshot of my screen: /tmp/s.png"
        let link = Assistant.keeping(read([msg("p", .user, given, 1), msg("a", .agent, "On it.", 2)]), in: started(prompt))
        XCTAssertEqual(link.start?.id, "p")
        XCTAssertEqual(texts(link.transcript), ["On it."])
    }

    // MARK: Rewinds and empty reads

    /// The same session taken back and redone: `[U, A] → [U] → [U, D]` keeps only D, saved too.
    func testARewindTakesBackTheAnswerItRemoved() {
        let typed = msg("u", .user, "Also add tests", 1)
        let answer = msg("a", .agent, "Tests added.", 5)
        let redone = msg("d", .agent, "Tests added, differently.", 9)
        var link = Assistant.keeping(read([typed, answer]), in: routed())
        link = Assistant.keeping(read([typed]), in: link)
        XCTAssertTrue(Assistant.taskMessages(link, live: nil).isEmpty, "the taken-back answer is gone")
        link = Assistant.keeping(read([typed, redone]), in: link)
        XCTAssertEqual(Assistant.taskMessages(reopened(link), live: nil), [redone])
        // Started the same way.
        let prompt = msg("p", .user, "Make a poster", 0)
        var poster = Assistant.keeping(read([prompt, answer]), in: started())
        poster = Assistant.keeping(read([prompt, redone]), in: poster)
        XCTAssertEqual(poster.transcript, [redone])
    }

    /// Past the task's start: a rewind to before every kept entry the read still has replaces
    /// them; a complete read without the start means it was all taken back.
    func testARewindPastTheStartAndACompleteReadWithoutIt() {
        let typed = msg("u", .user, "Also add tests", 1)
        let b1 = msg("b1", .agent, "One.", 2), b2 = msg("b2", .agent, "Two.", 3), b3 = msg("b3", .agent, "Three.", 4)
        var link = Assistant.keeping(read([typed, b1, b2, b3]), in: routed())
        let d = msg("d", .agent, "Redone.", 9)
        let rewound = Assistant.keeping(read((1...5).map(step) + [b1, d]), in: link)
        XCTAssertEqual(texts(rewound.transcript), ["One.", "Redone."], "b2 and b3 were taken back")
        link = Assistant.keeping(read([msg("x", .user, "Something else", 20)], complete: true), in: link)
        XCTAssertTrue(link.transcript.isEmpty, "a whole session without its send: the task's part was taken back")
    }

    /// A real, empty conversation is a read; no read (busy, unreadable) changes nothing.
    func testAnEmptyReadIsntAFailedOne() {
        let typed = msg("u", .user, "Also add tests", 1)
        let answer = msg("a", .agent, "Tests added.", 5)
        let link = Assistant.keeping(read([typed, answer]), in: routed())
        XCTAssertEqual(Assistant.taskMessages(link, live: nil), [answer], "no read: what it kept")
        XCTAssertEqual(Assistant.keeping(read([], complete: true), in: link).transcript, [], "an empty session: taken back")
        XCTAssertEqual(Assistant.keeping(read([], nil), in: routed()), routed(), "no session yet: nothing to place")
    }

    // MARK: Entry ids

    /// Identical-looking steps without times are told apart by their ids: an unchanged read adds
    /// nothing, and a read with one more adds only that one.
    func testRepeatedLookingStepsAreNeitherDuplicatedNorMerged() {
        let same = { (id: String) in ChatMessage(role: .tool, text: "bash", at: nil, id: id) }
        let nine = (1...9).map { same("s\($0)") }
        XCTAssertEqual(Assistant.merged(nine, nine), nine, "an unchanged read is idempotent")
        XCTAssertEqual(Assistant.merged(nine, nine + [same("s10")]).count, 10)
        let answer = ChatMessage(role: .agent, text: "Done.", at: nil, id: "a1")
        let history = nine + [answer] + (10...18).map { same("s\($0)") }
        let merged = Assistant.merged(history, history + [ChatMessage(role: .agent, text: "Done again.", at: nil, id: "a2")])
        XCTAssertEqual(merged.count, 20)
        XCTAssertEqual(merged.filter { $0.id == "a1" }.count, 1, "the earlier answer shows once")
        // Saved before entries had ids: matched by what they say, still not added twice.
        let untimed = Array(repeating: ChatMessage(role: .tool, text: "bash", at: nil), count: 9)
        XCTAssertEqual(Assistant.merged(untimed, untimed).count, 9)
    }

    /// OpenCode saves a tool call with no input, then fills it in under the same id: the read
    /// after replaces it, with the answer before it kept once, and reads after that change nothing.
    func testAnUpdatedEntryReplacesItsEarlierVersion() {
        let typed = msg("u", .user, "Also add tests", 1)
        let answer = msg("a", .agent, "Looking.", 2)
        let pending = ChatMessage(role: .tool, text: "bash", at: iso(3), id: "t")
        let filled = ChatMessage(role: .tool, text: "bash · pwd", at: iso(3), id: "t")
        var link = Assistant.keeping(read([typed, answer, pending]), in: routed())
        link = Assistant.keeping(read([typed, answer, filled]), in: link)
        XCTAssertEqual(link.transcript, [answer, filled])
        XCTAssertEqual(Assistant.keeping(read([typed, answer, filled]), in: link), link, "a repeated read changes nothing")
        // Past its start, the update lands on the kept copy too.
        let older = msg("o", .agent, "Earlier.", 1.5)
        XCTAssertEqual(Assistant.merged([older, answer, pending], [answer, filled]), [older, answer, filled])
        XCTAssertEqual(ChatView.keyed(link.transcript).map(\.key), ["id|a", "id|t"], "rows are the entries, by id")
    }

    /// Two distinct calls that read the same stay two.
    func testDistinctLookAlikeEntriesStayTwo() {
        let one = ChatMessage(role: .tool, text: "read", at: nil, id: "t1")
        let two = ChatMessage(role: .tool, text: "read", at: nil, id: "t2")
        XCTAssertEqual(Assistant.merged([one], [one, two]), [one, two])
    }

    // MARK: Sessions

    /// The same process starting a new session (`/clear`, `/new`), empty at first: the task keeps
    /// its replies and takes nothing from it, a late read of its own still counts, and the
    /// follow-up you send goes there as a task of its own, its answer once under it.
    func testAFollowUpAfterANewSessionIsItsOwnTask() {
        let typed = msg("1", .user, "Also add tests", 1)
        let answer = msg("2", .agent, "Tests added.", 5)
        var first = Assistant.keeping(read([typed, answer]), in: routed())
        XCTAssertEqual(first.source, "A")

        let empty = read([], "B", complete: true)
        XCTAssertEqual(Assistant.keeping(empty, in: first), first, "the new, empty session leaves it alone")
        XCTAssertTrue(Assistant.movedOn(first, source: empty.source), "and it shows it moved on before B has anything")

        // A read from A that was in flight across the switch still lands.
        let late = msg("3", .agent, "Also the docs.", 8)
        first = Assistant.keeping(read([typed, answer, late]), in: first)
        XCTAssertEqual(first.transcript, [answer, late])

        // The follow-up, sent to B as a task of its own.
        var followUp = routed("And the changelog", at: 30)
        followUp.baseline = empty.messages.compactMap(\.id)
        followUp.baselineSource = empty.source
        let asked = msg("1", .user, "And the changelog", 31)
        let replied = msg("2", .agent, "Changelog updated.", 40)
        let b = read([asked, replied], "B")
        XCTAssertEqual(Assistant.keeping(b, in: first), first, "B's messages aren't the first task's")
        followUp = Assistant.keeping(b, in: followUp)
        XCTAssertEqual(followUp.source, "B")
        XCTAssertEqual(Assistant.taskMessages(followUp, live: b), [replied])
        // Closed and reopened: both as they were.
        XCTAssertEqual(Assistant.taskMessages(reopened(first), live: nil), [answer, late])
        XCTAssertEqual(Assistant.taskMessages(reopened(followUp), live: nil), [replied])

        // A task Lantern started stays with its session the same way.
        let prompt = msg("p", .user, "Make a poster", 0)
        let poster = Assistant.keeping(read([prompt, answer]), in: started())
        XCTAssertEqual(Assistant.keeping(b, in: poster), poster)
    }

    /// An earlier session's conversation (P) before the new task's own (N): nothing of P is shown
    /// or kept, even with the same words, and N is taken once it has the prompt.
    func testAStartedTaskTakesOnlyItsOwnSession() {
        let link = started()
        let p = read([msg("p1", .user, "Make a poster", -300), msg("p2", .agent, "Old poster.", -290)], "P")
        let first = Assistant.keeping(p, in: link)
        XCTAssertNil(first.start, "the same words from before it started aren't its prompt")
        XCTAssertTrue(Assistant.taskMessages(first, live: p).isEmpty)
        let n = read([msg("n1", .user, "Make a poster", 2), msg("n2", .agent, "New poster.", 9)], "N")
        let bound = Assistant.keeping(n, in: first)
        XCTAssertEqual(bound.source, "N")
        XCTAssertEqual(texts(bound.transcript), ["New poster."])
    }

    /// A send without times (some sessions don't record them): only a copy that's new since the
    /// session was last read counts, and without such a read, none does.
    func testAnUntimedSendCountsOnlyIfItsNewSinceTheSend() {
        let earlier = msg("e", .user, "Also add tests", nil)
        let earlierAnswer = msg("ea", .agent, "Old answer.", nil)
        var link = routed()
        link.baseline = ["e", "ea"]
        link.baselineSource = "A"
        let before = Assistant.keeping(read([earlier, earlierAnswer]), in: link)
        XCTAssertNil(before.start, "the copy it already had isn't this send")
        let now = msg("n", .user, "Also add tests", nil)
        let answer = msg("na", .agent, "New answer.", nil)
        let placed = Assistant.keeping(read([earlier, earlierAnswer, now, answer]), in: link)
        XCTAssertEqual(placed.start?.id, "n")
        XCTAssertEqual(placed.transcript, [answer])
        XCTAssertNil(Assistant.keeping(read([earlier, earlierAnswer, now, answer]), in: routed()).start,
                     "no earlier read: Lantern can't tell which copy is this one")
    }

    /// Ask first reads a task after its prompt left the window, in its own session or a newer
    /// one: it shows nothing as the answer and, a while after, says it can't place it.
    func testAFirstReadWithoutTheStartIsShownAsUnplaced() {
        let window = read((1...120).map(step))
        let link = Assistant.keeping(window, in: started())
        XCTAssertNil(link.start)
        XCTAssertTrue(Assistant.taskMessages(link, live: window).isEmpty)
        XCTAssertFalse(Assistant.unplaced(link, live: window, now: sent.addingTimeInterval(5)), "not yet: it may still show up")
        XCTAssertTrue(Assistant.unplaced(link, live: window, now: sent.addingTimeInterval(60)))
        XCTAssertTrue(Assistant.unplaced(link, live: read([msg("x", .user, "Other", 50)], "B"), now: sent.addingTimeInterval(60)))
        XCTAssertFalse(Assistant.unplaced(link, live: read([], "A"), now: sent.addingTimeInterval(60)), "an empty session isn't missing anything")
    }

    func testMergingAddsOnlyWhatsNew() {
        let a = (1...20).map(step)
        XCTAssertEqual(Assistant.merged(Array(a[0..<10]), Array(a[5..<20])), a)
        XCTAssertEqual(Assistant.merged(Array(a[0..<10]), Array(a[8..<20])), a)
        XCTAssertEqual(Assistant.merged([], Array(a[0..<3])), Array(a[0..<3]))
        XCTAssertEqual(Assistant.merged(Array(a[0..<3]), Array(a[0..<3])), Array(a[0..<3]))
    }
}
