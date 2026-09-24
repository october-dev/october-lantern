import LanternCore
import XCTest

final class DraftsTests: XCTestCase {
    func testOpeningAnotherChatDoesNotCarryTheDraft() {
        var d = Drafts()
        d.address("A")
        d.type("private A context", fallback: nil)
        d.address("B")
        XCTAssertEqual(d.text, "")
        XCTAssertEqual(d.recipient, "B")
        d.address("A")
        XCTAssertEqual(d.text, "private A context")
    }

    func testAckForOneAgentKeepsSameTextDraftForAnother() {
        var d = Drafts()
        d.address("A")
        d.type("yes", fallback: nil)
        let sentToA = d.ticket()!
        d.address("B")
        d.type("yes", fallback: nil)
        d.sent(sentToA)
        XCTAssertEqual(d.text(for: "A"), "")
        XCTAssertEqual(d.text(for: "B"), "yes")
    }

    func testAckKeepsARetypedDraftWithTheSameText() {
        var d = Drafts()
        d.address("A")
        d.type("run the tests", fallback: nil)
        let old = d.ticket()!
        d.type("something else", fallback: nil)
        d.type("run the tests", fallback: nil)
        d.sent(old)
        XCTAssertEqual(d.text, "run the tests")
    }

    func testAckClearsTheUnchangedDraft() {
        var d = Drafts()
        d.address("A")
        d.type("  go  ", fallback: nil)
        let t = d.ticket()!
        XCTAssertEqual(t.text, "go")
        d.sent(t)
        XCTAssertEqual(d.text, "")
    }

    func testFirstKeystrokeLocksTheFallbackRecipient() {
        var d = Drafts()
        d.type("h", fallback: "A")
        XCTAssertEqual(d.recipient, "A")
        d.type("hi", fallback: "B")
        XCTAssertEqual(d.recipient, "A")
        XCTAssertEqual(d.text(for: "A"), "hi")
    }

    func testNoTicketWithoutRecipientOrText() {
        var d = Drafts()
        d.type("hello", fallback: nil)
        XCTAssertNil(d.ticket())
        d.address("A")
        d.type("   ", fallback: nil)
        XCTAssertNil(d.ticket())
    }

    func testReaddressMovesTextOnlyToAnEmptyDraft() {
        var d = Drafts()
        d.address("A")
        d.type("for someone", fallback: nil)
        d.readdress("B")
        XCTAssertEqual(d.text(for: "B"), "for someone")
        XCTAssertEqual(d.text(for: "A"), "")
        d.address("C")
        d.type("C's own", fallback: nil)
        d.address("B")
        d.readdress("C")
        XCTAssertEqual(d.text, "C's own")
        XCTAssertEqual(d.text(for: "B"), "for someone")
    }

    func testPruneKeepsTheAddressedDraft() {
        var d = Drafts()
        d.address("gone")
        d.type("keep me", fallback: nil)
        d.address("other")
        d.type("drop me", fallback: nil)
        d.address("gone")
        d.prune(keeping: [])
        XCTAssertEqual(d.text(for: "gone"), "keep me")
        XCTAssertEqual(d.text(for: "other"), "")
    }
}

final class LifecycleTests: XCTestCase {
    func testStopDuringBackoffPreventsRelaunch() {
        var s = Supervisor()
        let g = s.start()
        let delay = s.exited(g, ranFor: 1)
        XCTAssertNotNil(delay)
        s.stop()
        XCTAssertNil(s.relaunch(after: g))
    }

    func testStaleExitIsIgnored() {
        var s = Supervisor()
        let old = s.start()
        let new = s.start()
        XCTAssertNil(s.exited(old, ranFor: 1))
        XCTAssertTrue(s.isCurrent(new))
    }

    func testBackoffGrowsAndIsCapped() {
        var s = Supervisor()
        var g = s.start()
        var delays: [TimeInterval] = []
        for _ in 0..<7 {
            delays.append(s.exited(g, ranFor: 1)!)
            g = s.relaunch(after: g)!
        }
        XCTAssertEqual(delays, [2, 4, 8, 16, 30, 30, 30])
        XCTAssertEqual(s.exited(g, ranFor: 60), 2)
    }

    func testCancelledAttemptIsStale() {
        var a = Attempts()
        let first = a.begin()
        a.cancel()
        XCTAssertFalse(a.isCurrent(first))
        let second = a.begin()
        XCTAssertFalse(a.isCurrent(first))
        XCTAssertTrue(a.isCurrent(second))
    }
}
