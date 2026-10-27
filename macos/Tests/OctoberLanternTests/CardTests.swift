import XCTest
@testable import OctoberLantern

final class CardTests: XCTestCase {
    /// A `cards` event as the engine sends it (engine/src/cards, `snapshot`).
    func testTheEnginesCardsDecode() throws {
        let line = #"""
        {"type":"cards","launches":[{"id":"l-1","kind":"october","cwd":"/tmp","title":"Book the flight","agentId":"october:42:7",
        "tools":true,"createdAt":1793076316073,"endedAt":null,"status":"Comparing prices","revision":3,
        "steps":[{"id":"a","title":"Search","done":true}],
        "cards":[{"id":"c-1","kind":"ask","state":"waiting","revision":1,"createdAt":1,"resolvedAt":null,"answer":null,
        "question":"Which one?","details":null,"options":["Morning","Evening"]},
        {"id":"c-2","kind":"handOff","state":"delivered","revision":2,"createdAt":2,"resolvedAt":3,"answer":{"done":false,"text":"Not home"},
        "reason":"signIn","step":"Sign in to bank.example","why":null,"instructions":[],"url":"https://bank.example/login"},
        {"id":"c-3","kind":"result","state":"shown","revision":1,"createdAt":4,"resolvedAt":null,"answer":null,
        "summary":"Booked","outcome":"partial","files":[{"path":"/tmp/x.pdf","name":"x.pdf","folder":false}]}]}]}
        """#
        guard case .cards(let launches) = try JSONDecoder().decode(EngineMessage.self, from: Data(line.utf8)) else {
            return XCTFail("not a cards message")
        }
        let launch = try XCTUnwrap(launches.first)
        XCTAssertEqual(launch.waiting.map(\.id), ["c-1"])
        XCTAssertEqual(launch.cards[0].options, ["Morning", "Evening"])
        XCTAssertEqual(launch.cards[1].answer, CardAnswer(text: "Not home", done: false))
        XCTAssertEqual(launch.cards[2].files?.first?.name, "x.pdf")
        XCTAssertEqual(launch.steps.first?.done, true)
    }

    func testAnswersAreSentAsTheEngineTakesThem() throws {
        func json(_ answer: CardAnswer) throws -> String {
            String(decoding: try JSONSerialization.data(withJSONObject: answer.json, options: .sortedKeys), as: UTF8.self)
        }
        XCTAssertEqual(try json(CardAnswer(choice: "Evening")), #"{"choice":"Evening"}"#)
        XCTAssertEqual(try json(CardAnswer(text: "Later", done: false)), #"{"done":false,"text":"Later"}"#)
    }
}
