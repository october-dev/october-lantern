import Foundation

/// One question to October's AI, for Point & Ask's Ask.
///
/// It goes to Lantern's endpoint on October (`POST /api/lantern/ask`), signed in with the October
/// account's access token: October picks the model by plan, bills paid plans' credit, and the
/// answer streams back. A refused mode or image fails explicitly; the app does not change the
/// request into a different capability or drop pictures.
///
/// Point & Ask sends the question, the selected screenshot and the identified window's app/title
/// shown on the card. It does not attach the foreground browser's URL or selected text.
///
/// With your own AI key saved (`OwnKey`), each request also carries it in `X-Lantern-Byok-*`
/// headers and your provider answers through the same endpoint; when the provider fails, the
/// person may ask October's AI instead (never automatically).
enum OctoberAI {

    struct Request {
        /// Earlier questions and answers in this card.
        var turns: [(question: String, answer: String)]
        var question: String
        /// The screenshot, as JPEG.
        var image: Data?
        /// Several pictures (Ask Lantern): what was attached, then earlier ones in the conversation.
        var images: [Data] = []
        /// The displayed context, as lines of text.
        var context: String
        /// "chat" for the assistant (its own prompt on the server); nil for Point & Ask.
        var mode: String? = nil
        /// Show me (mode "show"): the controls on screen and the guide's round, from the engine.
        var guidance: [String: Any]? = nil
        /// Off for "Ask October's AI instead": October answers even with your own key saved.
        var ownKey = true
    }

    enum Failure: LocalizedError {
        case signedOut, limited(String), unavailable(String), notDeployed
        /// Out of free questions or credit, or the plan doesn't include it (or your own key):
        /// `URL` is October's Lantern page, where the trial and Lifetime are.
        case plan(String, URL)
        /// 400: the server couldn't read the request.
        case invalid(String)
        /// Your provider didn't accept your key.
        case ownKey(String)
        /// Your provider couldn't use the model you chose.
        case ownModel(String)
        /// Your provider refused (its own limit or billing) or failed, before any answer.
        case provider(String)

        var errorDescription: String? {
            switch self {
            case .signedOut: "Sign in to October to ask."
            case .notDeployed: "October's AI isn't available right now."
            case .plan(let m, _), .limited(let m), .unavailable(let m), .invalid(let m), .ownKey(let m), .ownModel(let m), .provider(let m): m
            }
        }

        /// What the answer offers to do about it.
        var fix: Fix? {
            switch self {
            case .plan(_, let url): .upgrade(url)
            case .ownKey: .editKey
            case .ownModel: .openSettings
            case .provider: .useOctober
            default: nil
            }
        }
    }

    /// A button under a failed answer.
    enum Fix: Codable, Equatable {
        /// Start free trial / Buy Lifetime, on October's Lantern page.
        case upgrade(URL)
        /// Edit key / Open Settings: Settings › October, at your own key.
        case editKey, openSettings
        /// Ask October's AI instead: the same question again, without your key.
        case useOctober
    }

    static let lanternPage = URL(string: "https://www.october.dev/lantern")!

    /// The server's upgrade link when it's October's own, else October's Lantern page.
    static func upgradeURL(_ value: Any?) -> URL {
        guard let s = value as? String, let url = URL(string: s), url.scheme == "https",
              url.host == "www.october.dev" || url.host == "october.dev" else { return lanternPage }
        return url
    }

    enum Offer: String { case trial, lifetime }

    /// Where `offer` is bought: `base` (October's Lantern page) with exactly `offer=…&ref=app`.
    static func offerURL(_ offer: Offer, base: URL = lanternPage) -> URL {
        var parts = URLComponents(url: base, resolvingAgainstBaseURL: false)!
        parts.queryItems = [URLQueryItem(name: "offer", value: offer.rawValue), URLQueryItem(name: "ref", value: "app")]
        parts.fragment = nil
        return parts.url!
    }

    static func stream(_ request: Request) -> AsyncThrowingStream<Piece, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    guard let token = await OctoberAccount.shared.accessToken() else { throw Failure.signedOut }
                    let key = request.ownKey ? await MainActor.run { OwnKey.shared.answering } : nil
                    try await ask(request, token: token, key: key, into: continuation)
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    /// A piece of the answer, or a note about the account to show under it.
    enum Piece {
        case text(String)
        /// Your own key is answering: the provider's name ("OpenAI").
        case via(String)
        case note(String)
        /// Work the assistant decided to start (chat mode): a prompt for a coding agent, and the
        /// agent and project only when the user named them. `to` names a running agent (its
        /// handle) to send it to instead of starting one.
        case task(prompt: String, agent: String?, project: String?, to: String?)
        /// Show me's answer: where to point (October's `guidance`, or nil when it gave none) and the
        /// guide's goal id.
        case guide(guidance: [String: Any]?, goalId: String?)
    }

    /// "18 free questions left today" from a free account's `done` event.
    static func note(done event: [String: Any]) -> String? {
        guard let remaining = event["remaining"] as? [String: Any], remaining["kind"] as? String == "free",
              let left = remaining["questions"] as? Int, left <= 10 else { return nil }
        return left == 0 ? "That was today's last free question." : "\(left) free question\(left == 1 ? "" : "s") left today."
    }

    // MARK: Lantern's endpoint

    /// The server takes at most 10 history messages, each non-empty and at most 8,000 characters:
    /// the last five whole turns, leaving out any without an answer (one cancelled before it began).
    static func history(_ turns: [(question: String, answer: String)]) -> [[String: String]] {
        let whole = turns.filter {
            !$0.question.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && !$0.answer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
        return whole.suffix(5).flatMap {
            [["role": "user", "content": capped($0.question, 8000)], ["role": "assistant", "content": capped($0.answer, 8000)]]
        }
    }

    /// At most `limit` UTF-16 units (how the server counts, as JavaScript's `length`), without
    /// splitting a character.
    static func capped(_ text: String, _ limit: Int) -> String {
        guard text.utf16.count > limit else { return text }
        var out = ""
        var used = 0
        for c in text {
            let n = String(c).utf16.count
            if used + n > limit { break }
            out.append(c)
            used += n
        }
        return out
    }

    static let askURL = URL(string: "https://www.october.dev/api/lantern/ask")!

    /// `POST /api/lantern/ask`: October picks the model (by plan) and bills the answer; the answer
    /// streams back as `start`, `delta`… and `done` events.
    private static func ask(_ r: Request, token: String, key: OwnKey.Saved?, into continuation: AsyncThrowingStream<Piece, Error>.Continuation) async throws {
        var json: [String: Any] = [
            "question": capped(r.question, 4000),
            "history": history(r.turns),
            "client": ["app": "lantern", "version": Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "dev"],
        ]
        if !r.context.isEmpty { json["context"] = capped(r.context, 6000) }
        if let mode = r.mode { json["mode"] = mode }
        if let image = r.image { json["image"] = image.base64EncodedString() }
        if !r.images.isEmpty { json["images"] = r.images.map { $0.base64EncodedString() } }
        if let guidance = r.guidance { json["guidance"] = guidance }
        var req = URLRequest(url: askURL)
        req.httpMethod = "POST"
        req.timeoutInterval = 90
        req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        for (name, value) in OwnKey.headers(key) { req.setValue(value, forHTTPHeaderField: name) }
        req.httpBody = try JSONSerialization.data(withJSONObject: json)
        let (bytes, response) = try await URLSession.shared.bytes(for: req)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        if status == 404 || status == 405 { throw Failure.notDeployed }
        guard status == 200 else {
            var data = Data()
            for try await b in bytes {
                data.append(b)
                if data.count > 64_000 { break }
            }
            throw failure(status: status, data: data)
        }
        for try await line in bytes.lines {
            guard line.hasPrefix("data:"),
                  let data = line.dropFirst(5).trimmingCharacters(in: .whitespaces).data(using: .utf8),
                  let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { continue }
            switch event["type"] as? String {
            case "start":
                if event["tier"] as? String == "byok", let key { continuation.yield(.via(key.provider.label)) }
            case "delta": if let text = event["text"] as? String { continuation.yield(.text(text)) }
            case "done":
                if let problem = event["taskError"] as? String { throw Failure.unavailable(problem) }
                if let t = event["task"] as? [String: Any], let prompt = t["prompt"] as? String,
                   !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    continuation.yield(.task(
                        prompt: String(prompt.prefix(4000)), agent: t["agent"] as? String, project: t["project"] as? String,
                        to: t["to"] as? String
                    ))
                }
                if r.mode == "show" {
                    continuation.yield(.guide(guidance: event["guidance"] as? [String: Any], goalId: event["goalId"] as? String))
                }
                if let note = note(done: event) { continuation.yield(.note(note)) }
                return
            case "error":
                throw Failure.unavailable(event["message"] as? String ?? "The answer stopped partway. Try again.")
            default: continue
            }
        }
        throw Failure.unavailable("October's answer stopped before it finished. Try again.")
    }

    static func failure(status: Int, data: Data) -> Failure {
        let error = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? [String: Any]
        let code = error?["code"] as? String ?? ""
        let message = error?["message"] as? String
        let upgrade = upgradeURL(error?["upgradeUrl"])
        switch (status, code) {
        case (_, "byok_requires_plan"): return .plan(message ?? "Your own key needs Lantern Lifetime or the Pro trial.", upgrade)
        case (_, "byok_key_invalid"): return .ownKey(message ?? "Your AI provider didn't accept your key. Check it in Settings.")
        case (_, "byok_model_unavailable"): return .ownModel(message ?? "Your AI provider couldn't use that model. Check the model in Settings.")
        case (_, "byok_provider_limited"): return .provider(message ?? "Your AI provider refused the question: its rate limit or billing.")
        case (_, "byok_provider_failed"): return .provider(message ?? "Your AI provider didn't answer. Try again, or ask October's AI instead.")
        case (401, _): return .signedOut
        case (_, "plan_required"): return .plan(message ?? "This needs an October Pro or Max plan.", upgrade)
        case (402, _), (_, "credit_exhausted"): return .plan(message ?? "You've used this period's October AI credit.", upgrade)
        case (_, "free_limit"): return .plan(message ?? "You've used today's free questions. They reset at midnight UTC.", upgrade)
        case (429, _): return .limited(message ?? "Too many questions for now. Try again in a minute.")
        case (400, _): return .invalid(message ?? "October's AI couldn't read the request.")
        default: return .unavailable(message ?? "October's AI isn't available right now (\(status)). Try again soon.")
        }
    }

}
