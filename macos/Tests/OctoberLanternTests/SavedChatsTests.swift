import XCTest
@testable import OctoberLantern

/// Ask Lantern's saved chats: one that can't be read is left out, not the whole list.
@MainActor
final class SavedChatsTests: XCTestCase {
    private let good = #"{"id":"6F9D4E36-2D40-4E3C-9B7A-1C1F0E0B5A11","title":"Find my thesis","updated":"2026-10-20T10:00:00Z","turns":[{"id":"0B0F5C1E-7E55-4A4F-9C6E-2B4F1E2A3C44","question":"Find my thesis","answer":"On it.","failed":false,"task":{"kind":"claude","folder":"/Users/me","prompt":"Find my thesis","inTerminal":false,"transcript":[{"role":"agent","text":"Found it."}]}}]}"#
    /// From before images and sign-in notes were saved: still reads.
    private let old = #"{"id":"1A2B3C4D-0000-4000-8000-000000000001","title":"Old","updated":"2026-10-14T09:00:00Z","turns":[{"id":"1A2B3C4D-0000-4000-8000-000000000002","question":"Old","answer":"a","failed":false}]}"#
    private let broken = #"{"id":"not-a-uuid","title":"Broken","updated":"yesterday","turns":[]}"#

    func testOneUnreadableChatIsLeftOutNotAll() throws {
        let data = Data("[\(good),\(broken),\(old)]".utf8)
        let read = try Assistant.decode(data).get()
        XCTAssertEqual(read.loaded.map(\.title), ["Find my thesis", "Old"])
        XCTAssertEqual(read.skipped, 1)
        XCTAssertNotNil(read.problem)
        XCTAssertEqual(read.loaded[0].turns[0].task?.transcript.first?.text, "Found it.")
    }

    func testAFileThatIsntAListFails() {
        guard case .failure = Assistant.decode(Data("{\"half".utf8)) else { return XCTFail("should fail") }
    }

    func testImagesAreSavedWithTheChat() throws {
        var turn = Assistant.Turn(question: "Make it look like this")
        turn.images = ["/tmp/a.png"]
        let conversation = Assistant.Conversation(title: "t", updated: Date(timeIntervalSince1970: 0), turns: [turn])
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let read = try Assistant.decode(try encoder.encode([conversation])).get()
        XCTAssertEqual(read.loaded.first?.turns.first?.images, ["/tmp/a.png"])
    }

    func testAgentsAreToldWhereTheImagesAre() {
        XCTAssertEqual(Assistant.imageLines([]), "")
        XCTAssertTrue(Assistant.imageLines([URL(fileURLWithPath: "/s/a.png")]).hasSuffix("/s/a.png"))
        let two = Assistant.imageLines([URL(fileURLWithPath: "/s/a.png"), URL(fileURLWithPath: "/s/b.png")])
        XCTAssertTrue(two.contains("- /s/a.png\n- /s/b.png"), two)
    }

    func testPastingAddsPicturesAndImageFilesButNotText() throws {
        let board = NSPasteboard(name: NSPasteboard.Name("lantern-test-\(UUID().uuidString)"))
        defer { board.releaseGlobally() }
        let picture = NSImage(size: NSSize(width: 20, height: 20), flipped: false) { r in NSColor.red.setFill(); r.fill(); return true }

        board.clearContents()
        board.writeObjects([picture])
        let pasted = try XCTUnwrap(ImageAttachments.fromPasteboard(board))
        XCTAssertEqual(pasted.count, 1)
        XCTAssertTrue(pasted[0].path.hasPrefix(ScreenCapture.folder.path))

        // Copied text that comes with a picture of itself (Excel, Word) stays text.
        board.clearContents()
        board.writeObjects([picture, "A1 B1" as NSString])
        XCTAssertNil(ImageAttachments.fromPasteboard(board))

        // An image file copied in Finder is added; another kind of file isn't.
        board.clearContents()
        board.writeObjects([pasted[0] as NSURL])
        XCTAssertEqual(ImageAttachments.fromPasteboard(board)?.count, 1)
        board.clearContents()
        board.writeObjects([URL(fileURLWithPath: "/etc/hosts") as NSURL])
        XCTAssertNil(ImageAttachments.fromPasteboard(board))

        for file in try FileManager.default.contentsOfDirectory(at: ScreenCapture.folder, includingPropertiesForKeys: nil)
        where file.lastPathComponent.hasPrefix("added-") && (try? file.resourceValues(forKeys: [.creationDateKey]).creationDate ?? .distantPast) ?? .distantPast > Date().addingTimeInterval(-60) {
            try? FileManager.default.removeItem(at: file)
        }
    }
}
