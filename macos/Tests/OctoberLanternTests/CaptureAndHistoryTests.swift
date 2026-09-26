@testable import OctoberLantern
import LanternCore
import AppKit
import XCTest

@MainActor
final class CaptureAndHistoryTests: XCTestCase {
    private func image() -> CGImage {
        CGContext(data: nil, width: 2, height: 2, bitsPerComponent: 8, bytesPerRow: 8,
                  space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!.makeImage()!
    }

    func testReselectionImmediatelyInvalidatesThePreviousPayload() {
        let point = PointAsk()
        let first = point.beginCapture()
        point.completeCapture(.success(image()), attempt: first)
        XCTAssertTrue(point.canSendSelection)
        let second = point.beginCapture()
        XCTAssertFalse(point.canSendSelection)
        XCTAssertNil(point.image)
        point.completeCapture(.success(image()), attempt: first)
        XCTAssertNil(point.image, "An old completion must not revive the old selection")
        XCTAssertFalse(point.canSendSelection, "Still capturing the new selection")
        XCTAssertTrue(point.capturing)
        point.completeCapture(.failure(NSError(domain: "test", code: 1)), attempt: second)
        XCTAssertNil(point.image, "Failure must not fall back to the previous screenshot")
        XCTAssertTrue(point.canSendSelection, "Without a screenshot, Ask and Send still go with words alone")
        XCTAssertFalse(point.capturing)
    }

    func testCancelDiscardsACaptureThatFinishesLater() {
        let point = PointAsk()
        let attempt = point.beginCapture()
        point.cancel()
        point.completeCapture(.success(image()), attempt: attempt)
        XCTAssertNil(point.image)
        XCTAssertFalse(point.capturing)
    }

    func testHistoryPollingAndLateResultsAreBoundedByRequestIdentity() {
        var requests = HistoryRequests()
        let first = requests.begin(for: "agent")!
        XCTAssertNil(requests.begin(for: "agent"), "Polling must not accumulate reads")
        requests.cancelAll()
        let reopened = requests.begin(for: "agent")!
        XCTAssertNotEqual(first, reopened)
        XCTAssertFalse(requests.finish(first, for: "agent"), "An old reply must not populate a reopened chat")
        XCTAssertNil(requests.begin(for: "agent"), "The old reply must not release the new request")
        XCTAssertTrue(requests.finish(reopened, for: "agent"))
        XCTAssertNotNil(requests.begin(for: "agent"))
    }

    func testLaunchTimeoutKeepsThePendingRequestUntilTheEngineAnswers() {
        let model = AppModel()
        model.track("launch-test", .launch, sent: true)
        model.expire("launch-test")
        XCTAssertNotNil(model.pending["launch-test"], "Timeout must not permit another launch while this one may still start")
        XCTAssertTrue(model.toast?.contains("Waiting for the engine") == true)
        model.launch(kind: AgentKind(rawValue: "claude"), folder: "/unused", prompt: "retry", background: true)
        XCTAssertEqual(model.pending.count, 1, "A retry must not create a second launch request")
    }
    func testRejectedLongReplyPreservesTheWholeDraftAndQueuedReplyIsLabelled() throws {
        let agent = try JSONDecoder().decode(Agent.self, from: Data("""
            {"id":"a","kind":"claude","handle":"claude-1","pid":1,"state":"waiting","canReply":true,
             "route":{"via":"october","canvasId":"c","nodeId":"n"},"stateSource":"hook"}
            """.utf8))
        let model = AppModel()
        model.update([agent])
        model.targetId = agent.id
        let full = String(repeating: "x", count: 8000) + "keep these final instructions"
        model.draft = full
        let ticket = try XCTUnwrap(model.drafts.ticket())
        model.track("reply", .reply(ticket), sent: true)
        model.replyFinished("reply", ok: false, uncertain: false, message: "Shorten the message; nothing was sent.")
        XCTAssertEqual(model.draft, full)
        model.track("queued", .direct(agentId: agent.id), sent: true)
        model.replyFinished("queued", ok: true, uncertain: false, message: "Queued in October")
        XCTAssertTrue(model.toast?.hasPrefix("Queued in October") == true)
        XCTAssertFalse(model.toast?.hasPrefix("Sent") == true)
    }

}
