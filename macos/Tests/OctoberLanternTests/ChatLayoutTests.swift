@testable import OctoberLantern
import XCTest

/// Issue #10: replies select as one text, copy keeps their shape, and the agent chat fits the panel.
@MainActor
final class ChatLayoutTests: XCTestCase {
    private func plain(_ text: String) -> String {
        String(MarkdownBlocks.attributed(MarkdownBlocks.parse(text)).characters)
    }

    func testATableReadsAsAHeaderThenARowPerLine() {
        let reply = """
        **Where the space goes**

        | What | Size |
        |---|---|
        | macOS itself | 30 GB |
        | `estate` copies | 22 GB |

        Want me to move it?
        """
        XCTAssertEqual(MarkdownBlocks.parse(reply), [
            .paragraph("**Where the space goes**"),
            .paragraph("What · Size"),
            .item(marker: "•", text: "macOS itself · 30 GB"),
            .item(marker: "•", text: "`estate` copies · 22 GB"),
            .paragraph("Want me to move it?"),
        ])
        // A lone row of pipes, no separator under it, is still a row; "a | b" without the outer pipes is text.
        XCTAssertEqual(MarkdownBlocks.parse("| a | b |\n\na | b"), [.item(marker: "•", text: "a · b"), .paragraph("a | b")])
    }

    func testAReplyIsOneTextWithItsBlocksInOrder() {
        let reply = """
        Here's one ready to paste:

        ---

        ## 🍁 we're hiring 3 dev interns!!

        here's the deal:
        - contribute to our **open-source** repos
        - get your contributions in [before 11 Nov](https://github.com/october-dev)

        1. one
        2. two

        ```
        func f() {
            return 1
        }
        ```
        clock's ticking ⏳
        """
        XCTAssertEqual(plain(reply), """
        Here's one ready to paste:

        ---

        🍁 we're hiring 3 dev interns!!

        here's the deal:

        • contribute to our open-source repos
        • get your contributions in before 11 Nov
        1. one
        2. two

        func f() {
            return 1
        }

        clock's ticking ⏳
        """)
    }

    func testLinksStayLinks() {
        let text = MarkdownBlocks.attributed(MarkdownBlocks.parse("see [the repo](https://github.com/october-dev)"))
        XCTAssertEqual(text.runs.compactMap(\.link), [URL(string: "https://github.com/october-dev")!])
    }

    /// What waits for you is bounded, and with it at its most the conversation, header and
    /// composer still fit the panel (issue #10).
    func testTheConversationFitsThePanelWithRoomForTheRest() {
        for panel: CGFloat in [500, 600, 700, 900, 1400] {
            let limit = ChatView.pinnedLimit(panel: panel)
            XCTAssertGreaterThanOrEqual(limit, 100)
            XCTAssertLessThanOrEqual(limit, 320)
            for pinned: CGFloat in [0, limit / 2, limit] {
                let height = ChatView.scrollHeight(panel: panel, pinned: pinned)
                XCTAssertGreaterThanOrEqual(height, 160)
                XCTAssertLessThanOrEqual(height, 800)
                XCTAssertLessThanOrEqual(ChatView.reserved + pinned + height, panel, "header, question and composer must still fit")
            }
        }
        XCTAssertLessThan(ChatView.scrollHeight(panel: 900, pinned: 200), ChatView.scrollHeight(panel: 900, pinned: 0))
    }

    /// Every code block keeps its Copy: past the row's width they go onto the next line.
    func testCopyButtonsWrapRatherThanOverflow() {
        let sizes = Array(repeating: CGSize(width: 100, height: 16), count: 5)
        let frames = FlowRow.frames(sizes, width: 330, spacing: 12, lineSpacing: 4)
        XCTAssertEqual(frames.map(\.minY), [0, 0, 0, 20, 20])
        XCTAssertLessThanOrEqual(frames.map(\.maxX).max()!, 330)
        XCTAssertEqual(FlowRow.frames([CGSize(width: 400, height: 16)], width: 330, spacing: 12, lineSpacing: 4).first?.minX, 0)
    }
}
