@testable import OctoberLantern
import AppKit
import SwiftUI
import Vision
import XCTest

@MainActor
final class PhoneTests: XCTestCase {
    private func state(code: String? = nil, finishing: Bool = false, outcome: String? = nil, error: String? = nil) throws -> PhoneModel.State {
        let pairing: [String: Any] = ["intentId": "test-intent", "bind": "test-phone", "qr": "https://october.dev/pair#synthetic-test-only",
            "expiresAt": 9999999999999.0, "code": code as Any? ?? NSNull(), "label": "Test iPhone", "platform": "ios", "finishing": finishing]
        return try JSONDecoder().decode(PhoneModel.State.self, from: JSONSerialization.data(withJSONObject: [
            "status": "connected", "devices": [], "pairing": outcome == nil ? pairing as Any : NSNull(), "busy": false,
            "outcome": outcome as Any? ?? NSNull(), "errorCode": error as Any? ?? NSNull(),
            "installUrl": "https://example.com/test-install"
        ]))
    }
    func testOnlyANewConfirmationBringsThePanelForward() throws {
        let model = PhoneModel()
        XCTAssertFalse(model.receive(try state()))
        XCTAssertTrue(model.receive(try state(code: "123456")))
        XCTAssertFalse(model.receive(try state(code: "123456")))
        XCTAssertFalse(model.receive(try state(code: "123456", finishing: true)))
        XCTAssertTrue(model.receive(try state(code: "654321")))
        model.reset()
        XCTAssertNil(model.state)
        XCTAssertFalse(model.busy)
    }
    func testSuccessOnlyComesFromFinalizedPairing() throws {
        let model = PhoneModel(); model.setup = true
        model.receive(try state(code: "123456", finishing: true))
        XCTAssertTrue(model.setup)
        model.receive(try state(outcome: "paired"))
        XCTAssertFalse(model.setup)
    }
    func testExpiryDoesNotGiveAnExtraMinuteAndRecoveriesAreExplicit() throws {
        let p = try XCTUnwrap(state().pairing)
        let deadline = Date(timeIntervalSince1970: p.expiresAt / 1000)
        XCTAssertEqual(p.remaining(at: deadline.addingTimeInterval(-1)), 1)
        XCTAssertEqual(p.remaining(at: deadline), 0)
        XCTAssertEqual(p.remaining(at: deadline.addingTimeInterval(1)), 0)
        for code in ["PAIRING_EXPIRED", "PAIRING_USED", "PAIRING_DISCONNECTED"] {
            XCTAssertEqual(PhoneModel.recovery(for: code, outcome: nil), "New code")
        }
        XCTAssertEqual(PhoneModel.recovery(for: "plan_required", outcome: nil), "Upgrade")
        XCTAssertEqual(PhoneModel.recovery(for: "plan_authority_unavailable", outcome: nil), "Retry")
        XCTAssertEqual(PhoneModel.recovery(for: nil, outcome: "denied"), "Start again")
    }
    func testQRIsReadableAndContainsTheFullHTTPSURL() throws {
        let link = "https://october.dev/pair#" + Data(String(repeating: "example-test-payload", count: 25).utf8).base64EncodedString()
        let image = try XCTUnwrap(PhoneCard.qrImage(link))
        let cg = try XCTUnwrap(image.cgImage(forProposedRect: nil, context: nil, hints: nil))
        let request = VNDetectBarcodesRequest(); request.symbologies = [.qr]
        try VNImageRequestHandler(cgImage: cg).perform([request])
        XCTAssertEqual(request.results?.first?.payloadStringValue, link)
    }
    func testPhoneViewsRenderAtTheRealCardWidthWithoutAnEngine() throws {
        let model = PhoneModel()
        let account = OctoberAccount.detached(signedIn: true)
        for (name, state) in [("install", try state()), ("scan", try state()), ("confirm", try state(code: "123456")),
                               ("finishing", try state(code: "123456", finishing: true)), ("done", try state(outcome: "paired")),
                               ("denied", try state(outcome: "denied"))] {
            model.reset(); model.receive(state); model.setup = name == "install"
            if name == "install" {
                let empty = try JSONDecoder().decode(PhoneModel.State.self, from: Data("""
                {"status":"offline","devices":[],"busy":false,"installUrl":"https://example.com/test-install"}
                """.utf8))
                model.receive(empty)
            }
            let renderer = ImageRenderer(content: PhoneCard(model: model, account: account).frame(width: 348).background(Color.black).environment(\.colorScheme, .dark))
            renderer.scale = 2
            let image = try XCTUnwrap(renderer.nsImage, name)
            XCTAssertEqual(image.size.width, 348, accuracy: 1)
            XCTAssertLessThan(image.size.height, 900, name)
            if let path = ProcessInfo.processInfo.environment["LANTERN_PHONE_SNAPSHOTS"] {
                let dir = URL(fileURLWithPath: path); try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
                let bitmap = try XCTUnwrap(image.tiffRepresentation.flatMap(NSBitmapImageRep.init(data:)))
                try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: dir.appendingPathComponent("\(name).png"))
            }
        }
    }
}
