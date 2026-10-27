import AppKit
import UserNotifications

/// macOS notifications when an agent needs you. Clicking one opens that agent's conversation.
@MainActor
final class Notifier: NSObject, ObservableObject, UNUserNotificationCenterDelegate {
    static let shared = Notifier()
    var onOpen: ((String) -> Void)?

    /// Notifications need a real app bundle (a bundle identifier).
    private var center: UNUserNotificationCenter? {
        Bundle.main.bundleIdentifier == nil ? nil : UNUserNotificationCenter.current()
    }

    func setUp() {
        center?.delegate = self
        Task { await refreshStatus() }
        // Permission can change in System Settings while Lantern runs.
        NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in
            Task { @MainActor in await Notifier.shared.refreshStatus() }
        }
    }

    func requestPermission() async -> Bool {
        guard let center else { return false }
        let granted = (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
        await refreshStatus()
        return granted
    }

    /// Whether macOS lets Lantern notify: notDetermined, denied, authorized (or provisional).
    @Published private(set) var status: UNAuthorizationStatus = .notDetermined

    func refreshStatus() async {
        guard let center else { return }
        status = await center.notificationSettings().authorizationStatus
    }

    static func openSystemSettings() {
        NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension")!)
    }

    func post(for agent: Agent) {
        let prefs = Preferences.shared
        guard prefs.notificationsEnabled, let center else { return }
        if agent.state == .waiting && !prefs.notifyOnFinish { return }

        let content = UNMutableNotificationContent()
        content.title = agent.state == .needsInput ? "@\(agent.handle) needs you" : "@\(agent.handle) finished"
        content.subtitle = [agent.project, agent.title].compactMap { $0 }.joined(separator: " · ")
        let body = agent.question ?? agent.lastMessage ?? "It's your turn."
        content.body = String(body.prefix(240))
        content.sound = agent.state == .needsInput ? .default : nil
        content.userInfo = ["agentId": agent.id]
        content.threadIdentifier = agent.id
        center.add(UNNotificationRequest(identifier: agent.turnKey, content: content, trigger: nil))
    }

    /// A card waiting for you: a question or a step only you can do. One notification per card.
    func post(card: Card, agent: Agent) {
        guard Preferences.shared.notificationsEnabled, let center else { return }
        let content = UNMutableNotificationContent()
        content.title = card.kind == "handOff" ? "@\(agent.handle) needs you to do something" : "@\(agent.handle) has a question"
        content.subtitle = [agent.project, agent.title].compactMap { $0 }.joined(separator: " · ")
        content.body = String((card.kind == "handOff" ? card.step : card.question)?.prefix(240) ?? "")
        content.sound = .default
        content.userInfo = ["agentId": agent.id]
        content.threadIdentifier = agent.id
        center.add(UNNotificationRequest(identifier: card.id, content: content, trigger: nil))
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse
    ) async {
        let id = response.notification.request.content.userInfo["agentId"] as? String
        await MainActor.run { if let id { onOpen?(id) } }
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }
}
