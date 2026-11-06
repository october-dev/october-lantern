import XCTest
@testable import OctoberLantern

/// Ask Lantern's request stays within what `/api/lantern/ask` accepts.
final class AskRequestTests: XCTestCase {
    private func failure(_ status: Int, _ error: [String: Any]) -> OctoberAI.Failure {
        OctoberAI.failure(status: status, data: try! JSONSerialization.data(withJSONObject: ["error": error]))
    }

    func testPlanLimitsOfferTheTrialAndLifetime() {
        XCTAssertEqual(failure(429, ["code": "free_limit", "message": "Out"]).fix, .upgrade)
        XCTAssertEqual(failure(402, ["code": "credit_exhausted", "upgradeUrl": "https://lantern.october.dev/#pricing"]).fix, .upgrade)
        XCTAssertEqual(failure(403, ["code": "plan_required"]).fix, .upgrade)
        XCTAssertNil(failure(429, ["code": "rate_limited"]).fix, "slow down isn't a plan problem")
        XCTAssertNil(failure(401, [:]).fix)
        XCTAssertEqual(failure(429, ["code": "free_limit", "message": "Out"]).errorDescription, "Out")
        XCTAssertFalse(failure(429, ["code": "free_limit"]).errorDescription!.contains("5"), "the number is the server's")

    }

    func testOffersAreBoughtAsAGuestWithTheSignedInEmail() {
        XCTAssertEqual(OctoberAI.offerURL(.trial, email: nil).absoluteString, "https://lantern.october.dev/buy?offer=trial&ref=app")
        XCTAssertEqual(OctoberAI.offerURL(.lifetime, email: nil).absoluteString, "https://lantern.october.dev/buy?offer=lifetime&ref=app")
        let url = OctoberAI.offerURL(.trial, email: "a+b@x.com")
        XCTAssertEqual(url.absoluteString, "https://lantern.october.dev/buy?offer=trial&ref=app&email=a%2Bb@x.com")
        XCTAssertEqual(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.last, URLQueryItem(name: "email", value: "a+b@x.com"))
    }

    func testAnOlderUpgradeLinkStillReadsAsTheOffers() throws {
        let saved = Data(#"{"upgrade":{"_0":"https:\/\/www.october.dev\/lantern"}}"#.utf8)
        XCTAssertEqual(try JSONDecoder().decode(OctoberAI.Fix.self, from: saved), .upgrade)
    }

    func testOwnKeyFailuresOfferTheRightFix() {
        XCTAssertEqual(failure(402, ["code": "byok_requires_plan", "message": "Needs Lifetime"]).fix, .upgrade)
        XCTAssertEqual(failure(400, ["code": "byok_key_invalid", "message": "Bad key"]).fix, .editKey)
        XCTAssertEqual(failure(400, ["code": "byok_key_invalid", "message": "Bad key"]).errorDescription, "Bad key")
        XCTAssertEqual(failure(400, ["code": "byok_model_unavailable"]).fix, .openSettings)
        XCTAssertEqual(failure(429, ["code": "byok_provider_limited", "message": "OpenAI says slow down"]).fix, .useOctober)
        XCTAssertEqual(failure(502, ["code": "byok_provider_failed"]).fix, .useOctober)
        XCTAssertNotNil(failure(502, ["code": "byok_provider_failed"]).errorDescription)
    }

    func testOwnKeyHeadersAreAllOrNothing() {
        let key = OwnKey.Saved(provider: .anthropic, key: "sk-ant-secret1234", model: nil)
        XCTAssertEqual(OwnKey.headers(key), ["X-Lantern-Byok-Provider": "anthropic", "X-Lantern-Byok-Key": "sk-ant-secret1234"])
        var withModel = key
        withModel.model = "claude-x"
        XCTAssertEqual(OwnKey.headers(withModel)["X-Lantern-Byok-Model"], "claude-x")
        XCTAssertEqual(OwnKey.headers(nil), [:])
        XCTAssertFalse("\(key)".contains("secret"), "the key never reaches logs")
        XCTAssertFalse(String(reflecting: key).contains("secret"))
        XCTAssertEqual(key.hint, "…1234")
        // A plan that says your own key isn't included: October's AI answers instead of failing.
        XCTAssertNil(OwnKey.answering(key, plan: lantern(byok: false)))
        XCTAssertEqual(OwnKey.answering(key, plan: lantern(byok: true)), key)
        XCTAssertEqual(OwnKey.answering(key, plan: nil), key, "an unknown plan leaves it to October")
    }

    private func lantern(byok: Bool, lifetime: Bool = false, trialEndsAt: Date? = nil, daily: Int = 20) -> OctoberAccount.Plan.Lantern {
        .init(lifetime: lifetime, trialEndsAt: trialEndsAt, byok: byok, dailyQuestions: daily)
    }

    func testPlanDecodesLanternRights() throws {
        let json = #"{"plan":"free","status":null,"lantern":{"lifetime":false,"trialEndsAt":"2026-11-01T17:00:00.123+00:00","byok":true,"dailyQuestions":20}}"#
        let plan = try JSONDecoder().decode(OctoberAccount.Plan.self, from: Data(json.utf8))
        XCTAssertEqual(plan.lantern?.byok, true)
        XCTAssertEqual(plan.lantern?.dailyQuestions, 20)
        XCTAssertEqual(plan.lantern?.trialEndsAt?.timeIntervalSince1970 ?? 0, 1_793_552_400.123, accuracy: 0.01)
        let old = try JSONDecoder().decode(OctoberAccount.Plan.self, from: Data(#"{"plan":"pro"}"#.utf8))
        XCTAssertNil(old.lantern)

        let before = Date(timeIntervalSince1970: 1_793_000_000)
        let trial = lantern(byok: true, trialEndsAt: plan.lantern?.trialEndsAt)
        XCTAssertEqual(AccountCard.lanternLines(trial, now: before).count, 3)
        XCTAssertTrue(AccountCard.lanternLines(trial, now: before)[0].hasPrefix("Pro trial · ends "))
        XCTAssertEqual(AccountCard.lanternLines(trial, now: before)[1], "Up to 20 questions a day on October's AI")
        XCTAssertEqual(AccountCard.lanternLines(trial, now: before)[2], "Agents use Pro AI, our smartest")
        XCTAssertEqual(AccountCard.lanternLines(trial, now: Date(timeIntervalSince1970: 1_800_000_000)),
                       ["Up to 20 questions a day on October's AI", "Agents use Pro AI, our smartest"],
                       "an ended trial isn't shown")
        XCTAssertEqual(AccountCard.lanternLines(lantern(byok: true, lifetime: true))[0], "Lantern Lifetime")
        XCTAssertTrue(AccountCard.offersUpgrade(try JSONDecoder().decode(OctoberAccount.Plan.self, from: Data(#"{"plan":"free","lantern":{"lifetime":false,"trialEndsAt":null,"byok":false,"dailyQuestions":5}}"#.utf8))))
        XCTAssertFalse(AccountCard.offersUpgrade(plan, now: before), "not during the trial")
        XCTAssertEqual(AccountCard.lanternLines(lantern(byok: false)).last, "Agents use Lite AI, free with your account")
    }

    func testAgentsRunOnProForAPaidLanternAndLiteOtherwise() throws {
        let paid = try JSONDecoder().decode(OctoberAccount.Plan.self, from: Data(#"{"plan":"free","lantern":{"lifetime":true,"byok":true,"dailyQuestions":20}}"#.utf8))
        let free = try JSONDecoder().decode(OctoberAccount.Plan.self, from: Data(#"{"plan":"free","lantern":{"lifetime":false,"byok":false,"dailyQuestions":5}}"#.utf8))
        XCTAssertEqual(OctoberTier(plan: paid), .pro)
        XCTAssertEqual(OctoberTier(plan: free), .lite)
        XCTAssertEqual(OctoberTier(plan: nil), .lite, "signed out or not loaded yet")
        XCTAssertEqual(OctoberTier.pro.label, "Pro")
        XCTAssertEqual(OctoberTier.lite.label, "Lite")
        for tier in [OctoberTier.pro, .lite] {
            XCTAssertFalse(tier.explanation.contains("Claude") || tier.explanation.contains("Opus") || tier.explanation.contains("Sonnet"), "no model names")
        }
    }

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

    func testAMessageOffersTheLinksAndExistingFilesItNames() {
        let text = "Rendered `~/Movies/Lantern Ads/october/out.mp4`. Preview at http://localhost:3000/, frames in /tmp/frames. Also /no/such/file and (https://october.dev). Not a path: a/b, http://"
        let exists = { (p: String) in ["/home/me/Movies/Lantern Ads/october/out.mp4", "/tmp/frames"].contains(p) }
        XCTAssertEqual(Bubble.targets(in: text, home: "/home/me", exists: exists), [
            .file("/home/me/Movies/Lantern Ads/october/out.mp4"), .link("http://localhost:3000/"), .file("/tmp/frames"),
            .link("https://october.dev"),
        ])
        XCTAssertEqual(Bubble.targets(in: "nothing here", exists: { _ in true }), [])
    }

    func testYourMessageHidesTheRecipeLine() {
        let sent = "Follow the recipe at https://lantern.october.dev/recipes/video-ad.md. Read it first (curl -fsSL https://lantern.october.dev/recipes/video-ad.md), then do this:\n\nmake me an ad\n\nYou're connected to October Bus as x"
        XCTAssertEqual(Bubble.withoutLanternNotes(sent), "make me an ad")
    }

    @MainActor func testARecipeStaysInFrontOfYourWords() {
        let recipe = "Follow the recipe at https://lantern.october.dev/recipes/video-ad.md. Read it first (curl -fsSL https://lantern.october.dev/recipes/video-ad.md), then do this:"
        XCTAssertEqual(Assistant.taskPrompt(question: "make me an ad for Bean Cafe", suggested: recipe + "\n\nMake a video ad for Bean Cafe", followUp: false),
                       recipe + "\n\nmake me an ad for Bean Cafe")
        XCTAssertEqual(Assistant.taskPrompt(question: "make me an ad", suggested: "Follow the recipe at https://example.com/x.md\n\nMake an ad", followUp: false),
                       "make me an ad")
    }
}
