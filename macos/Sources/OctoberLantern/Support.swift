import AppKit
import ServiceManagement

/// Problem reports, crash reports and uninstalling.
@MainActor
enum Support {
    static let email = "hey@october.dev"
    static let website = URL(string: "https://lantern.october.dev")!

    static var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
    }

    /// What goes into a problem report: versions and a summary of the agents, never their
    /// conversations, folders or code.
    static func diagnostics(agents: [Agent]) -> String {
        let os = ProcessInfo.processInfo.operatingSystemVersionString
        var arch = "unknown"
        #if arch(arm64)
        arch = "Apple silicon"
        #elseif arch(x86_64)
        arch = "Intel"
        #endif
        let kinds = Dictionary(grouping: agents, by: { $0.kind.displayName }).map { "\($0.key) ×\($0.value.count)" }.sorted()
        let routes = Dictionary(grouping: agents, by: { "\($0.host?.app ?? "?") via \($0.route?.via ?? "none")" })
            .map { "\($0.key) ×\($0.value.count)" }.sorted()
        return """
        October Lantern \(version)
        macOS \(os), \(arch)
        Agents: \(kinds.isEmpty ? "none" : kinds.joined(separator: ", "))
        Terminals: \(routes.isEmpty ? "none" : routes.joined(separator: ", "))
        """
    }

    /// Opens an email to the October team, with files attached when Mail can take them.
    static func compose(subject: String, body: String, attachments: [URL] = []) {
        if let mail = NSSharingService(named: .composeEmail), mail.canPerform(withItems: [body] + attachments) {
            mail.recipients = [email]
            mail.subject = subject
            NSApp.activate()
            mail.perform(withItems: [body] + attachments)
            return
        }
        var parts = URLComponents()
        parts.scheme = "mailto"
        parts.path = email
        parts.queryItems = [URLQueryItem(name: "subject", value: subject), URLQueryItem(name: "body", value: body)]
        if let url = parts.url { NSWorkspace.shared.open(url) }
        if !attachments.isEmpty { NSWorkspace.shared.activateFileViewerSelecting(attachments) }
    }

    static func reportProblem(agents: [Agent]) {
        compose(
            subject: "October Lantern \(version): a problem",
            body: "What happened, and what did you expect?\n\n\n---\n\(diagnostics(agents: agents))\n",
            attachments: recentCrashReports(since: Date().addingTimeInterval(-7 * 86400))
        )
    }

    static func recentCrashReports(since: Date) -> [URL] {
        let dir = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/DiagnosticReports")
        let files = (try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        return files.filter { url in
            let name = url.lastPathComponent
            guard name.hasPrefix("OctoberLantern") || name.hasPrefix("lantern-engine") else { return false }
            let date = (try? url.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
            return (date ?? .distantPast) > since
        }
    }

    /// If Lantern crashed since the last launch, offer to send the report. Never sends anything
    /// without asking.
    static func offerCrashReportIfNeeded(agents: [Agent]) {
        let key = "lastCrashCheck"
        let last = UserDefaults.standard.object(forKey: key) as? Date ?? Date()
        UserDefaults.standard.set(Date(), forKey: key)
        let reports = recentCrashReports(since: last)
        guard !reports.isEmpty else { return }
        let alert = NSAlert()
        alert.messageText = "October Lantern quit unexpectedly"
        alert.informativeText = "Would you like to email the crash report to the October team? It has technical details about the crash, not your conversations or code. You'll see the email before it's sent."
        alert.addButton(withTitle: "Email Report")
        alert.addButton(withTitle: "Not Now")
        NSApp.activate()
        if alert.runModal() == .alertFirstButtonReturn {
            compose(subject: "October Lantern \(version): crash report",
                    body: "Anything you were doing when it happened?\n\n\n---\n\(diagnostics(agents: agents))\n",
                    attachments: reports)
        }
    }

    /// Signs out of October (the session leaves the Keychain), stops the phone host and the engine,
    /// removes the hooks (restoring the agents' settings), Lantern's support files, the login item and
    /// preferences, then moves the app to the Trash. Every step is checked; if one fails, Lantern
    /// says which and how to finish by hand, and asks before trashing the app.
    static func uninstall(model: AppModel) {
        let alert = NSAlert()
        alert.messageText = "Uninstall October Lantern?"
        alert.informativeText = "This signs out of October, removes Lantern's hooks from Claude Code and Codex (restoring your previous settings), its support files and its settings, then moves the app to the Trash. Your agents and any sessions Lantern started keep running. Paired phones aren't removed: they'll show this Mac as offline until you remove it in the October app."
        alert.addButton(withTitle: "Uninstall")
        alert.addButton(withTitle: "Cancel")
        alert.buttons.first?.hasDestructiveAction = true
        NSApp.activate()
        guard alert.runModal() == .alertFirstButtonReturn else { return }

        Task { @MainActor in
            var failures: [String] = []

            if OctoberAccount.shared.signedIn { OctoberAccount.shared.signOut() }
            PhoneModel.shared.stop()
            model.stop()

            let out = await Hooks.run(["hooks", "remove-all"])
            if let error = out.split(separator: "\n").first(where: { $0.hasPrefix("Error") || $0.hasPrefix("Couldn't") || $0.contains("engine is missing") }) {
                failures.append("Removing the hooks: \(error). Remove the Lantern entries from ~/.claude/settings.json (\"hooks\") and ~/.codex/config.toml (\"notify\") by hand; backups ending in .lantern-backup-… are next to each file. Then delete ~/Library/Application Support/October Lantern.")
            }
            do {
                if SMAppService.mainApp.status == .enabled { try await SMAppService.mainApp.unregister() }
            } catch {
                failures.append("Removing the login item: \(error.localizedDescription). Remove October Lantern in System Settings › General › Login Items.")
            }
            if let domain = Bundle.main.bundleIdentifier {
                UserDefaults.standard.removePersistentDomain(forName: domain)
                UserDefaults.standard.synchronize()
            }

            if !failures.isEmpty {
                let a = NSAlert()
                a.alertStyle = .warning
                a.messageText = "Some of Lantern couldn't be removed"
                a.informativeText = failures.joined(separator: "\n\n")
                a.addButton(withTitle: "Move App to Trash Anyway")
                a.addButton(withTitle: "Keep the App")
                NSApp.activate()
                guard a.runModal() == .alertFirstButtonReturn else { return }
            }
            NSWorkspace.shared.recycle([Bundle.main.bundleURL]) { _, error in
                DispatchQueue.main.async {
                    if let error {
                        let a = NSAlert()
                        a.messageText = "Couldn't move the app to the Trash"
                        a.informativeText = "\(error.localizedDescription)\n\nEverything else is removed. Drag October Lantern from Applications to the Trash."
                        a.runModal()
                    }
                    NSApp.terminate(nil)
                }
            }
        }
    }
}
