import Foundation

/// One question to October's AI, for Point & Ask's Ask.
///
/// It goes to October's inference gateway (`https://www.october.dev/v1`, OpenAI-compatible), signed
/// in with the October account's access token, and the answer streams back. Lantern picks a model
/// from the gateway's list that can see images (the screenshot goes with the first question); if
/// the model or plan only takes text, it asks again with the text alone.
///
/// What's sent: the question, the screenshot of the selected area, and the app, window title,
/// page address and selected text shown on the card. Nothing else.
enum OctoberAI {
    static let base = URL(string: "https://www.october.dev/v1")!

    struct Request {
        /// Earlier questions and answers in this card.
        var turns: [(question: String, answer: String)]
        var question: String
        /// The screenshot, as JPEG.
        var image: Data?
        /// The app, window, page and selection, as lines of text.
        var context: String
    }

    enum Failure: LocalizedError {
        case signedOut, plan(String), limited(String), unavailable(String)

        var errorDescription: String? {
            switch self {
            case .signedOut: "Sign in to October to ask."
            case .plan(let m), .limited(let m), .unavailable(let m): m
            }
        }
    }

    static let system = """
    You are October Lantern's Point & Ask on the user's Mac. The user selected part of their screen \
    (the attached screenshot) and asked about it. Answer directly and briefly: a short paragraph or a \
    few bullets, plain Markdown, no headings. If the question is about code or an error, say what is \
    wrong and how to fix it. If you can't see enough to answer, say what you'd need.
    """

    static func stream(_ request: Request) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    guard let token = await OctoberAccount.shared.accessToken() else { throw Failure.signedOut }
                    let model = try await pickModel(token: token, needsVision: request.image != nil)
                    do {
                        try await run(body(request, model: model.id, withImage: request.image != nil && model.vision), token: token, into: continuation)
                    } catch Failure.unavailable(let message) where request.image != nil && message.localizedCaseInsensitiveContains("text") {
                        // The gateway only takes text for this model or plan: ask without the picture.
                        try await run(body(request, model: model.id, withImage: false), token: token, into: continuation)
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    // MARK: The request

    private static func body(_ r: Request, model: String, withImage: Bool) -> Data {
        var messages: [[String: Any]] = [["role": "system", "content": system]]
        var first = r.turns.first?.question ?? r.question
        if !r.context.isEmpty { first += "\n\nWhere this is on my Mac:\n\(r.context)" }
        if !withImage, r.image != nil { first += "\n\n(A screenshot was taken but this model can't see images.)" }
        var firstContent: Any = first
        if withImage, let image = r.image {
            firstContent = [
                ["type": "text", "text": first],
                ["type": "image_url", "image_url": ["url": "data:image/jpeg;base64,\(image.base64EncodedString())"]],
            ]
        }
        messages.append(["role": "user", "content": firstContent])
        for (i, turn) in r.turns.enumerated() {
            messages.append(["role": "assistant", "content": turn.answer])
            let next = i + 1 < r.turns.count ? r.turns[i + 1].question : r.question
            messages.append(["role": "user", "content": next])
        }
        let json: [String: Any] = ["model": model, "messages": messages, "stream": true, "max_tokens": 1200]
        return (try? JSONSerialization.data(withJSONObject: json)) ?? Data()
    }

    /// Sends the request and yields the answer's text as it streams in (OpenAI's server-sent events).
    private static func run(_ body: Data, token: String, into continuation: AsyncThrowingStream<String, Error>.Continuation) async throws {
        var req = URLRequest(url: base.appendingPathComponent("chat/completions"))
        req.httpMethod = "POST"
        req.timeoutInterval = 90
        req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        req.httpBody = body
        let (bytes, response) = try await URLSession.shared.bytes(for: req)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard status == 200 else {
            var data = Data()
            for try await b in bytes {
                data.append(b)
                if data.count > 64_000 { break }
            }
            throw failure(status: status, data: data)
        }
        for try await line in bytes.lines {
            guard line.hasPrefix("data:") else { continue }
            let payload = line.dropFirst(5).trimmingCharacters(in: .whitespaces)
            if payload == "[DONE]" { break }
            guard let data = payload.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { continue }
            if let error = obj["error"] as? [String: Any] {
                throw Failure.unavailable(error["message"] as? String ?? "October's AI stopped answering.")
            }
            let choice = (obj["choices"] as? [[String: Any]])?.first
            if let piece = (choice?["delta"] as? [String: Any])?["content"] as? String, !piece.isEmpty {
                continuation.yield(piece)
            }
        }
    }

    static func failure(status: Int, data: Data) -> Failure {
        let error = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? [String: Any]
        let code = error?["code"] as? String ?? ""
        let message = error?["message"] as? String
        switch (status, code) {
        case (401, _): return .signedOut
        case (_, "plan_required"): return .plan(message ?? "This needs an October Pro or Max plan.")
        case (402, _), (_, "credit_exhausted"): return .plan("You've used this period's October AI credit.")
        case (429, _): return .limited("Too many questions for now. Try again in a minute.")
        case (400, _): return .unavailable(message ?? "October's AI couldn't read the request.")
        default: return .unavailable(message ?? "October's AI isn't available right now (\(status)). Try again soon.")
        }
    }

    // MARK: Choosing a model

    struct Model: Equatable {
        let id: String
        let vision: Bool
    }

    private static var cached: (model: Model, at: Date)?

    /// A model from the gateway's list: one that can see images when there's a screenshot. The
    /// `pointAskModel` default overrides it.
    private static func pickModel(token: String, needsVision: Bool) async throws -> Model {
        if let id = UserDefaults.standard.string(forKey: "pointAskModel"), !id.isEmpty { return Model(id: id, vision: true) }
        if let cached, Date().timeIntervalSince(cached.at) < 3600, cached.model.vision || !needsVision { return cached.model }
        var req = URLRequest(url: base.appendingPathComponent("models"))
        req.timeoutInterval = 15
        req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        let (data, response) = try await URLSession.shared.data(for: req)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard status == 200 else { throw failure(status: status, data: data) }
        let list = ((try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["data"] as? [[String: Any]]) ?? []
        let models = list.compactMap { m -> (id: String, vision: Bool?)? in
            guard let id = m["id"] as? String else { return nil }
            return (id, (m["capabilities"] as? [String: Any])?["vision"] as? Bool)
        }
        guard let chosen = choose(models) else {
            throw Failure.unavailable("October's AI has no models available right now.")
        }
        cached = (chosen, Date())
        return chosen
    }

    /// Prefers models that can see, then well-known capable families, then anything.
    static func choose(_ models: [(id: String, vision: Bool?)]) -> Model? {
        let preferred = ["gemini", "claude", "gpt-4", "gpt-5", "qwen2.5-vl", "qwen3-vl", "vl", "llama-4", "gemma-3", "mistral-small", "pixtral"]
        func rank(_ id: String) -> Int {
            let lower = id.lowercased()
            return preferred.firstIndex { lower.contains($0) } ?? preferred.count
        }
        let seeing = models.filter { $0.vision == true }.sorted { rank($0.id) < rank($1.id) }
        if let m = seeing.first { return Model(id: m.id, vision: true) }
        let any = models.sorted { rank($0.id) < rank($1.id) }
        return any.first.map { Model(id: $0.id, vision: false) }
    }
}
