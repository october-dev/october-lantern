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

    /// Removes the hooks (restoring the agents' settings), Lantern's support files, the login
    /// item and preferences, then moves the app to the Trash.
    static func uninstall() {
        let alert = NSAlert()
        alert.messageText = "Uninstall October Lantern?"
        alert.informativeText = "This removes Lantern's hooks from Claude Code and Codex (restoring your previous settings), its support files and its settings, then moves the app to the Trash. Your agents and any sessions Lantern started keep running."
        alert.addButton(withTitle: "Uninstall")
        alert.addButton(withTitle: "Cancel")
        alert.buttons.first?.hasDestructiveAction = true
        NSApp.activate()
        guard alert.runModal() == .alertFirstButtonReturn else { return }

        if let engine = EngineClient.engineURL() {
            let p = Process()
            p.executableURL = engine
            p.arguments = ["hooks", "remove-all"]
            try? p.run()
            p.waitUntilExit()
        }
        try? SMAppService.mainApp.unregister()
        if let domain = Bundle.main.bundleIdentifier { UserDefaults.standard.removePersistentDomain(forName: domain) }
        NSWorkspace.shared.recycle([Bundle.main.bundleURL]) { _, _ in
            DispatchQueue.main.async { NSApp.terminate(nil) }
        }
    }
}
