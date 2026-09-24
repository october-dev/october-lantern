import Foundation

/// Anonymous usage counts, sent to PostHog (US Cloud): how many people use Lantern each day and
/// which features they use. Never content: no messages, prompts, commands, folder or project
/// names, file paths or agent titles. Every property sent is listed at its call site.
///
/// Installs are identified by a random id. Someone signed in to October is linked to their
/// October account (id and email), so usage can be tied to an account.
///
/// On by default, off in Settings; nothing is sent (or kept) while off. Builds without a PostHog
/// key (`PostHogKey` in Info.plist, added by scripts/build-app.sh) send nothing at all.
@MainActor
final class Analytics {
    static let shared = Analytics()

    private static let endpoint = URL(string: "https://us.i.posthog.com/batch/")!
    private let key = Bundle.main.object(forInfoDictionaryKey: "PostHogKey") as? String
    private let d = UserDefaults.standard
    private var queue: [[String: Any]] = []
    private var flushTask: Task<Void, Never>?
    private var sending = false

    private init() {
        d.register(defaults: ["analyticsEnabled": true])
        // Events not sent before the last quit.
        if let data = d.data(forKey: "analyticsQueue"),
           let saved = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] {
            queue = saved
            d.removeObject(forKey: "analyticsQueue")
            scheduleFlush(after: 30)
        }
    }

    /// Keeps unsent events for the next launch (called when the app quits).
    func persist() {
        guard enabled, !queue.isEmpty, let data = try? JSONSerialization.data(withJSONObject: queue) else { return }
        d.set(data, forKey: "analyticsQueue")
    }

    var enabled: Bool {
        get { d.bool(forKey: "analyticsEnabled") }
        set {
            d.set(newValue, forKey: "analyticsEnabled")
            if !newValue { queue.removeAll() }
        }
    }

    /// Whether this build can send anything (a release build with a key).
    var available: Bool { !(key ?? "").isEmpty }

    /// A random id for this install, made on first use.
    private var installId: String {
        if let id = d.string(forKey: "analyticsInstallId") { return id }
        let id = UUID().uuidString.lowercased()
        d.set(id, forKey: "analyticsInstallId")
        return id
    }

    /// The October account id while signed in, otherwise the install id.
    private var distinctId: String { d.string(forKey: "analyticsUserId") ?? installId }

    func capture(_ event: String, _ properties: [String: Any] = [:]) {
        guard available, enabled else { return }
        var props = properties
        props["distinct_id"] = distinctId
        props["$lib"] = "lantern-mac"
        props["app_version"] = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
        props["os_version"] = ProcessInfo.processInfo.operatingSystemVersionString
        #if arch(arm64)
            props["arch"] = "arm64"
        #else
            props["arch"] = "x86_64"
        #endif
        // PostHog derives an approximate location (country, city) from the IP address it sees.
        queue.append(["event": event, "properties": props, "timestamp": ISO8601DateFormatter().string(from: Date())])
        if queue.count > 200 { queue.removeFirst(queue.count - 200) }
        scheduleFlush(after: queue.count >= 20 ? 1 : 60)
    }

    /// Links this install's usage to an October account (after sign-in or on launch while signed in).
    func identify(userId: String, email: String?) {
        guard available, enabled, d.string(forKey: "analyticsUserId") != userId else { return }
        let anonymous = distinctId
        d.set(userId, forKey: "analyticsUserId")
        var set: [String: Any] = [:]
        if let email { set["email"] = email }
        capture("$identify", ["$anon_distinct_id": anonymous, "$set": set])
    }

    /// Signed out: later usage is anonymous again.
    func forgetUser() {
        d.removeObject(forKey: "analyticsUserId")
    }

    /// Once per calendar day while the app runs: what DAU and MAU are counted from. `kinds` is how
    /// many agents of each harness are running (e.g. ["claude": 3, "codex": 1]).
    func dailyActive(kinds: [String: Int]) {
        let today = Date().formatted(.iso8601.year().month().day())
        guard available, enabled, d.string(forKey: "analyticsLastActive") != today else { return }
        d.set(today, forKey: "analyticsLastActive")
        var props: [String: Any] = ["agents_running": kinds.values.reduce(0, +)]
        for (kind, n) in kinds { props["agents_\(kind)"] = n }
        capture("lantern_active", props)
    }

    private func scheduleFlush(after seconds: Double) {
        guard flushTask == nil else { return }
        flushTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(seconds))
            self?.flushTask = nil
            await self?.flush()
        }
    }

    /// Sends what's queued. Failed sends stay queued for the next try.
    func flush() async {
        guard available, enabled, !queue.isEmpty, !sending, let key else { return }
        let batch = queue
        sending = true
        defer { sending = false }
        var req = URLRequest(url: Self.endpoint)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.timeoutInterval = 15
        req.httpBody = try? JSONSerialization.data(withJSONObject: ["api_key": key, "batch": batch])
        guard let (_, response) = try? await URLSession.shared.data(for: req),
              (response as? HTTPURLResponse)?.statusCode ?? 0 < 300 else {
            scheduleFlush(after: 300)
            return
        }
        queue.removeFirst(min(batch.count, queue.count))
    }
}
