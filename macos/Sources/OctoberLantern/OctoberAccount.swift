import AppKit
import CryptoKit
import Foundation
import Network
import Security

/// Signing in with an October account, the same way October Desktop does: Supabase Auth with
/// PKCE, the browser redirecting to a one-shot listener on http://localhost:54547/callback
/// (already on October's redirect allowlist), or email and password. The session lives in the
/// Keychain and is refreshed before it expires.
@MainActor
final class OctoberAccount: ObservableObject {
    static let shared = OctoberAccount()

    // October's public Supabase client values (row-level security protects the data).
    static let supabaseURL = URL(string: "https://latwxiqjgvluiddckvmj.supabase.co")!
    static let anonKey = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImxhdHd4aXFqZ3ZsdWlkZGNrdm1qIiwicm9sZSI6ImFub24iLCJpYXQiOjE3NTAzMDExMzEsImV4cCI6MjA2NTg3NzEzMX0.Aor5lE6ZSvv83Or_CxQNUdRzRUqit3fODkNSJpcDJ7E"
    static let planURL = URL(string: "https://www.october.dev/api/plan")!
    static let callbackPort: UInt16 = 54547

    enum Provider: String, CaseIterable, Identifiable {
        case google, github, apple
        var id: String { rawValue }
        var label: String {
            switch self {
            case .google: "Google"
            case .github: "GitHub"
            case .apple: "Apple"
            }
        }
    }

    struct Session: Codable {
        var accessToken: String
        var refreshToken: String
        var expiresAt: Date
        var userId: String
        var email: String?
    }

    struct Plan: Decodable {
        let plan: String
        let status: String?
        struct Features: Decodable {
            struct Flag: Decodable { let enabled: Bool }
            let mobile: Flag?
        }
        let features: Features?
    }

    @Published private(set) var session: Session?
    @Published private(set) var plan: Plan?
    @Published private(set) var signingIn = false
    @Published var error: String?

    var signedIn: Bool { session != nil }
    var mobileAllowed: Bool { plan?.features?.mobile?.enabled ?? false }

    private var callback: CallbackListener?
    private var refreshTask: Task<Void, Never>?

    private init() {
        session = Keychain.load()
        if session != nil {
            Task { await refreshIfNeeded(force: false); await loadPlan() }
        }
    }

    // MARK: Sign in

    func signIn(with provider: Provider) {
        guard !signingIn else { return }
        error = nil
        signingIn = true
        let verifier = Self.randomVerifier()
        let challenge = Data(SHA256.hash(data: Data(verifier.utf8))).base64URL
        var parts = URLComponents(url: Self.supabaseURL.appendingPathComponent("auth/v1/authorize"), resolvingAgainstBaseURL: false)!
        parts.queryItems = [
            URLQueryItem(name: "provider", value: provider.rawValue),
            URLQueryItem(name: "redirect_to", value: "http://localhost:\(Self.callbackPort)/callback"),
            URLQueryItem(name: "code_challenge", value: challenge),
            URLQueryItem(name: "code_challenge_method", value: "s256"),
        ]
        if provider == .google { parts.queryItems?.append(URLQueryItem(name: "prompt", value: "select_account")) }

        let listener = CallbackListener(port: Self.callbackPort)
        callback = listener
        listener.start { [weak self] result in
            Task { @MainActor in
                guard let self else { return }
                self.callback = nil
                switch result {
                case .success(let code): await self.exchange(code: code, verifier: verifier)
                case .failure(let message):
                    self.error = message
                    self.signingIn = false
                }
            }
        } onReady: { ok in
            Task { @MainActor in
                if ok {
                    NSWorkspace.shared.open(parts.url!)
                } else {
                    self.error = "Another app is signing in to October right now. Try again in a moment."
                    self.signingIn = false
                }
            }
        }
        // Give up after five minutes, like October Desktop.
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(300))
            guard let self, self.callback === listener else { return }
            listener.stop()
            self.callback = nil
            self.signingIn = false
            self.error = "Sign-in timed out."
        }
    }

    func cancelSignIn() {
        callback?.stop()
        callback = nil
        signingIn = false
    }

    func signIn(email: String, password: String) async {
        error = nil
        signingIn = true
        defer { signingIn = false }
        do {
            let s = try await token(grant: "password", body: ["email": email, "password": password])
            await adopt(s)
        } catch {
            self.error = Self.describe(error)
        }
    }

    func signOut() {
        let token = session?.accessToken
        session = nil
        plan = nil
        refreshTask?.cancel()
        Keychain.delete()
        PhoneModel.shared.stop()
        guard let token else { return }
        var req = URLRequest(url: Self.supabaseURL.appendingPathComponent("auth/v1/logout"))
        req.url = URL(string: req.url!.absoluteString + "?scope=local")
        req.httpMethod = "POST"
        req.setValue(Self.anonKey, forHTTPHeaderField: "apikey")
        req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        URLSession.shared.dataTask(with: req).resume()
    }

    // MARK: Tokens

    /// A current access token, refreshing first if it's about to expire.
    func accessToken() async -> String? {
        await refreshIfNeeded(force: false)
        return session?.accessToken
    }

    private func exchange(code: String, verifier: String) async {
        defer { signingIn = false }
        do {
            let s = try await token(grant: "pkce", body: ["auth_code": code, "code_verifier": verifier])
            await adopt(s)
        } catch {
            self.error = Self.describe(error)
        }
    }

    private func adopt(_ s: Session) async {
        session = s
        Keychain.save(s)
        PhoneModel.shared.start(accessToken: s.accessToken)
        scheduleRefresh()
        await loadPlan()
    }

    func refreshIfNeeded(force: Bool) async {
        guard let s = session else { return }
        guard force || s.expiresAt.timeIntervalSinceNow < 120 else {
            PhoneModel.shared.start(accessToken: s.accessToken)
            scheduleRefresh()
            return
        }
        do {
            let fresh = try await token(grant: "refresh_token", body: ["refresh_token": s.refreshToken])
            session = fresh
            Keychain.save(fresh)
            PhoneModel.shared.start(accessToken: fresh.accessToken)
            scheduleRefresh()
        } catch AuthError.rejected {
            // The refresh token is no longer valid: signed out elsewhere or expired.
            signOut()
            error = "You were signed out. Sign in again to reconnect."
        } catch {
            // Offline or October unreachable: keep the session and try again later.
            scheduleRefresh(in: 60)
        }
    }

    private func scheduleRefresh(in seconds: TimeInterval? = nil) {
        refreshTask?.cancel()
        guard let s = session else { return }
        let delay = seconds ?? max(30, s.expiresAt.timeIntervalSinceNow - 90)
        refreshTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(delay))
            guard !Task.isCancelled else { return }
            await self?.refreshIfNeeded(force: true)
        }
    }

    func loadPlan() async {
        guard let token = session?.accessToken else { return }
        var req = URLRequest(url: Self.planURL)
        req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        if let (data, resp) = try? await URLSession.shared.data(for: req),
           (resp as? HTTPURLResponse)?.statusCode == 200 {
            plan = try? JSONDecoder().decode(Plan.self, from: data)
        }
    }

    enum AuthError: Error {
        case rejected(String)
        case failed(String)
    }

    private func token(grant: String, body: [String: String]) async throws -> Session {
        var req = URLRequest(url: URL(string: Self.supabaseURL.absoluteString + "/auth/v1/token?grant_type=\(grant)")!)
        req.httpMethod = "POST"
        req.timeoutInterval = 15
        req.setValue(Self.anonKey, forHTTPHeaderField: "apikey")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, resp) = try await URLSession.shared.data(for: req)
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        let json = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
        guard status == 200 else {
            let message = (json["error_description"] ?? json["msg"] ?? json["message"]) as? String ?? "Sign-in failed (\(status))."
            throw (400...403).contains(status) ? AuthError.rejected(message) : AuthError.failed(message)
        }
        guard let access = json["access_token"] as? String, let refresh = json["refresh_token"] as? String,
              let expiresIn = json["expires_in"] as? Double, let user = json["user"] as? [String: Any],
              let id = user["id"] as? String else {
            throw AuthError.failed("October's sign-in answer was missing something.")
        }
        return Session(accessToken: access, refreshToken: refresh, expiresAt: Date().addingTimeInterval(expiresIn),
                       userId: id, email: user["email"] as? String)
    }

    private static func describe(_ error: Error) -> String {
        switch error {
        case AuthError.rejected(let m), AuthError.failed(let m): m
        default: "Couldn't reach October. Check your connection."
        }
    }

    private static func randomVerifier() -> String {
        var bytes = [UInt8](repeating: 0, count: 48)
        _ = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
        return Data(bytes).base64URL
    }
}

extension Data {
    var base64URL: String {
        base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

/// The session in the login Keychain, readable only by Lantern.
private enum Keychain {
    static let service = "dev.october.lantern.session"
    static let account = "october"

    static func save(_ s: OctoberAccount.Session) {
        guard let data = try? JSONEncoder().encode(s) else { return }
        delete()
        let item: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            kSecValueData as String: data,
        ]
        SecItemAdd(item as CFDictionary, nil)
    }

    static func load() -> OctoberAccount.Session? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return try? JSONDecoder().decode(OctoberAccount.Session.self, from: data)
    }

    static func delete() {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
    }
}

/// A one-shot HTTP listener on localhost for the browser's redirect back with `?code=`.
final class CallbackListener: @unchecked Sendable {
    enum Result { case success(String), failure(String) }
    private let port: UInt16
    private var listener: NWListener?
    private var done = false
    private let queue = DispatchQueue(label: "lantern.oauth")

    init(port: UInt16) { self.port = port }

    func start(completion: @escaping @Sendable (Result) -> Void, onReady: @escaping @Sendable (Bool) -> Void) {
        let params = NWParameters.tcp
        params.requiredLocalEndpoint = NWEndpoint.hostPort(host: "127.0.0.1", port: NWEndpoint.Port(rawValue: port)!)
        params.allowLocalEndpointReuse = false
        guard let l = try? NWListener(using: params) else { onReady(false); return }
        listener = l
        l.stateUpdateHandler = { state in
            switch state {
            case .ready: onReady(true)
            case .failed: onReady(false)
            default: break
            }
        }
        l.newConnectionHandler = { [weak self] conn in self?.handle(conn, completion: completion) }
        l.start(queue: queue)
    }

    func stop() {
        listener?.cancel()
        listener = nil
    }

    private func handle(_ conn: NWConnection, completion: @escaping @Sendable (Result) -> Void) {
        conn.start(queue: queue)
        conn.receive(minimumIncompleteLength: 1, maximumLength: 16384) { [weak self] data, _, _, _ in
            guard let self else { return }
            let request = data.flatMap { String(data: $0, encoding: .utf8) } ?? ""
            let target = request.split(separator: " ").dropFirst().first.map(String.init) ?? ""
            let parts = URLComponents(string: "http://localhost\(target)")
            guard parts?.path == "/callback" else {
                self.reply(conn, status: "404 Not Found", body: "Not found")
                return
            }
            let items = parts?.queryItems ?? []
            let value = { (name: String) in items.first { $0.name == name }?.value }
            if self.done {
                self.reply(conn, status: "409 Conflict", body: "Already handled")
                return
            }
            self.done = true
            if let code = value("code") {
                self.reply(conn, status: "200 OK", body: Self.page("You're signed in", "You can close this tab and go back to October Lantern."))
                completion(.success(code))
            } else {
                let message = value("error_description") ?? value("error") ?? "Sign-in was cancelled."
                self.reply(conn, status: "400 Bad Request", body: Self.page("Sign-in didn't finish", message))
                completion(.failure(message))
            }
            self.stop()
        }
    }

    private func reply(_ conn: NWConnection, status: String, body: String) {
        let response = "HTTP/1.1 \(status)\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: \(body.utf8.count)\r\nConnection: close\r\n\r\n\(body)"
        conn.send(content: Data(response.utf8), completion: .contentProcessed { _ in conn.cancel() })
    }

    private static func page(_ title: String, _ text: String) -> String {
        let esc = { (s: String) in s.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;") }
        return """
        <!doctype html><meta charset="utf-8"><title>October Lantern</title>
        <body style="font:16px -apple-system,sans-serif;background:#111;color:#eee;display:grid;place-items:center;height:100vh;margin:0">
        <div style="text-align:center"><h2>\(esc(title))</h2><p style="color:#aaa">\(esc(text))</p></div></body>
        """
    }
}
