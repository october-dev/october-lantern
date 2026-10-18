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
    /// Hold to point at something on screen and ask an agent about it.
    @Published var pointAsk: PointAskPreset { didSet { d.set(pointAsk.rawValue, forKey: "pointAsk") } }
    /// New sessions start with a screenshot of the screen you're on (New Session can turn it off
    /// for one session).
    @Published var screenshotNewSessions: Bool { didSet { d.set(screenshotNewSessions, forKey: "screenshotNewSessions") } }
    /// Also give the toolkit list to plain new sessions (Tasks always get it).
    @Published var toolkitInSessions: Bool { didSet { d.set(toolkitInSessions, forKey: "toolkitInSessions") } }
    /// Connect agents Lantern starts to October Bus, so they can message each other.
    @Published var busForSessions: Bool { didSet { d.set(busForSessions, forKey: "busForSessions") } }
    /// Let agents Lantern starts use this Mac's apps through Cua Driver (engine `computer`).
    @Published var computerUse: Bool { didSet { d.set(computerUse, forKey: "computerUse") } }
    /// Agents Lantern starts ask before each action instead of starting in their own
    /// auto-approve mode (engine `askFirst`). Off: tasks often run where nobody sees a question.
    @Published var askFirst: Bool { didSet { d.set(askFirst, forKey: "askFirst") } }
    /// The agent new sessions and assistant tasks start with (an AgentKind raw value), or empty to
    /// choose: your own signed-in plan first, then October. See `AppModel.defaultAgentKind`.
    @Published var defaultAgent: String { didSet { d.set(defaultAgent, forKey: "defaultAgent") } }

    private init() {
        d.register(defaults: ["notificationsEnabled": true, "notifyOnFinish": true])
        notificationsEnabled = d.bool(forKey: "notificationsEnabled")
        notifyOnFinish = d.bool(forKey: "notifyOnFinish")
        mutedKinds = Set(d.stringArray(forKey: "mutedKinds") ?? [])
        hotkey = HotKeyPreset(rawValue: d.string(forKey: "hotkey") ?? "") ?? .controlOptionSpace
        edge = d.string(forKey: "pillEdge") ?? "right"
        pointAsk = PointAskPreset(rawValue: d.string(forKey: "pointAsk") ?? "") ?? .controlOptionP
        screenshotNewSessions = d.bool(forKey: "screenshotNewSessions")
        toolkitInSessions = d.bool(forKey: "toolkitInSessions")
        d.register(defaults: ["busForSessions": true, "computerUse": true])
        busForSessions = d.bool(forKey: "busForSessions")
        computerUse = d.bool(forKey: "computerUse")
        askFirst = d.bool(forKey: "askFirst")
        defaultAgent = d.string(forKey: "defaultAgent") ?? ""
    }

    func counts(_ kind: AgentKind) -> Bool { !mutedKinds.contains(kind.rawValue) }
}
