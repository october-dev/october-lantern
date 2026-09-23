import AppKit
import UserNotifications

/// macOS notifications when an agent needs you. Clicking one opens that agent's conversation.
@MainActor
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    static let shared = Notifier()
    var onOpen: ((String) -> Void)?

    /// Notifications need a real app bundle (a bundle identifier).
    private var center: UNUserNotificationCenter? {
        Bundle.main.bundleIdentifier == nil ? nil : UNUserNotificationCenter.current()
    }

    func setUp() {
        center?.delegate = self
    }

    func requestPermission() async -> Bool {
        guard let center else { return false }
        return (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
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
