@testable import OctoberLantern
import XCTest

/// Issue #10: replies select as one text, copy keeps their shape, and the agent chat fits the panel.
@MainActor
final class ChatLayoutTests: XCTestCase {
    private func plain(_ text: String) -> String {
        String(MarkdownBlocks.attributed(MarkdownBlocks.parse(text)).characters)
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

    func testTheConversationFitsThePanelWithRoomForTheRest() {
        for panel: CGFloat in [500, 700, 900, 1400] {
            for asking in [false, true] {
                let height = ChatView.scrollHeight(panel: panel, asking: asking)
                XCTAssertGreaterThanOrEqual(height, 160)
                XCTAssertLessThanOrEqual(height, 800)
                if panel >= 700 {
                    XCTAssertLessThanOrEqual(height + (asking ? 450 : 230), panel, "header, question and composer must still fit")
                }
            }
        }
        XCTAssertLessThan(ChatView.scrollHeight(panel: 900, asking: true), ChatView.scrollHeight(panel: 900, asking: false))
    }
}
