import XCTest
@testable import OctoberLantern

/// Ask Lantern's request stays within what `/api/lantern/ask` accepts.
final class AskRequestTests: XCTestCase {
    func testHistoryIsTheLastFiveWholeTurns() {
        var turns = (1...7).map { (question: "q\($0)", answer: "a\($0)") }
        turns.insert((question: "cancelled", answer: ""), at: 6)
        let history = OctoberAI.history(turns)
        XCTAssertEqual(history.count, 10)
        XCTAssertEqual(history.first?["content"], "q3")
        XCTAssertEqual(history.last?["content"], "a7")
        XCTAssertFalse(history.contains { $0["content"] == "cancelled" })
    }

    func testCapsCountUTF16WithoutSplittingACharacter() {
        XCTAssertEqual(OctoberAI.capped("abc", 5), "abc")
        // "😀" is two UTF-16 units: three fit in 7, the fourth would split.
        XCTAssertEqual(OctoberAI.capped("😀😀😀😀", 7), "😀😀😀")
        XCTAssertEqual(OctoberAI.capped(String(repeating: "x", count: 9000), 8000).utf16.count, 8000)
        let long = (1...2).map { _ in (question: "q", answer: String(repeating: "é", count: 9000)) }
        XCTAssertTrue(OctoberAI.history(long).allSatisfy { ($0["content"] ?? "").utf16.count <= 8000 })
    }
}

/// A task from Ask Lantern goes to the agent in your words.
final class AskTaskPromptTests: XCTestCase {
    @MainActor func testFirstRequestIsSentAsTyped() {
        XCTAssertEqual(Assistant.taskPrompt(question: " Find me my thesis document ", suggested: "Find the user's thesis document", followUp: false),
                       "Find me my thesis document")
    }

    @MainActor func testFollowUpUsesTheSelfContainedVersion() {
        XCTAssertEqual(Assistant.taskPrompt(question: "yes, do that", suggested: "Fix the Wi-Fi on this Mac", followUp: true),
                       "Fix the Wi-Fi on this Mac")
    }
}
