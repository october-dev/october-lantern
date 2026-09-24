import Foundation

/// User settings, stored in UserDefaults.
@MainActor
final class Preferences: ObservableObject {
    static let shared = Preferences()
    private let d = UserDefaults.standard

    @Published var notificationsEnabled: Bool { didSet { d.set(notificationsEnabled, forKey: "notificationsEnabled") } }
    /// Also notify when an agent finishes its turn, not only when it's blocked on you.
    @Published var notifyOnFinish: Bool { didSet { d.set(notifyOnFinish, forKey: "notifyOnFinish") } }
    /// Harnesses that don't count toward Waiting, the badge or notifications.
    @Published var mutedKinds: Set<String> { didSet { d.set(Array(mutedKinds), forKey: "mutedKinds") } }
    @Published var hotkey: HotKeyPreset { didSet { d.set(hotkey.rawValue, forKey: "hotkey") } }
    @Published var edge: String { didSet { d.set(edge, forKey: "pillEdge") } }
    /// New sessions start with a screenshot of the screen you're on (New Session can turn it off
    /// for one session).
    @Published var screenshotNewSessions: Bool { didSet { d.set(screenshotNewSessions, forKey: "screenshotNewSessions") } }
    /// Also give the toolkit list to plain new sessions (Tasks always get it).
    @Published var toolkitInSessions: Bool { didSet { d.set(toolkitInSessions, forKey: "toolkitInSessions") } }
    /// Connect agents Lantern starts to October Bus, so they can message each other.
    @Published var busForSessions: Bool { didSet { d.set(busForSessions, forKey: "busForSessions") } }

    private init() {
        d.register(defaults: ["notificationsEnabled": true, "notifyOnFinish": true])
        notificationsEnabled = d.bool(forKey: "notificationsEnabled")
        notifyOnFinish = d.bool(forKey: "notifyOnFinish")
        mutedKinds = Set(d.stringArray(forKey: "mutedKinds") ?? [])
        hotkey = HotKeyPreset(rawValue: d.string(forKey: "hotkey") ?? "") ?? .controlOptionSpace
        edge = d.string(forKey: "pillEdge") ?? "right"
        screenshotNewSessions = d.bool(forKey: "screenshotNewSessions")
        toolkitInSessions = d.bool(forKey: "toolkitInSessions")
        d.register(defaults: ["busForSessions": true])
        busForSessions = d.bool(forKey: "busForSessions")
    }

    func counts(_ kind: AgentKind) -> Bool { !mutedKinds.contains(kind.rawValue) }
}
