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

    /// Report a Problem: with their OK, the engine sends Lantern's logs (secrets taken out) to the
    /// October team and gives a code, then an email opens with that code for them to say what
    /// happened. Logs that can't be sent are saved to the Desktop and attached instead.
    static func reportProblem(model: AppModel) {
        let alert = NSAlert()
        alert.messageText = "Report a problem"
        alert.informativeText = "Lantern sends its logs to the October team, then opens an email for you to say what happened. The logs have Lantern's version, your Mac's version and Lantern's own logs, with passwords and keys taken out. Never your code or your agents' conversations."
        alert.addButton(withTitle: "Send Logs and Write")
        alert.addButton(withTitle: "Just Write")
        alert.addButton(withTitle: "Cancel")
        NSApp.activate()
        let choice = alert.runModal()
        let crashes = recentCrashReports(since: Date().addingTimeInterval(-7 * 86400))
        if choice == .alertSecondButtonReturn { emailProblem(agents: model.agents, logs: nil, attachments: crashes) }
        guard choice == .alertFirstButtonReturn else { return }
        var details = diagnostics(agents: model.agents)
        details += "\nEngine: \(model.engineHealth)"
        details += "\nComputer use: \(model.computer?.status ?? "off")"
        details += "\nSigned in to October: \(OctoberAccount.shared.session == nil ? "no" : "yes")"
        details += "\nSaved chats: \(Assistant.loadNote) (\(Assistant.shared.conversations.count) now)"
        details += "\nAsk Lantern tasks, newest first:\n\(Assistant.shared.reportLines())"
        details += "\nEach agent:\n" + model.agents.map { a in
            "- \(a.id) \(a.state) via \(a.route?.via ?? "none")\(a.tmux.map { " pane \($0.socket.map { "-L \($0) " } ?? "")\($0.target)" } ?? "")\(a.host.map { " in \($0.app)" } ?? "")"
        }.joined(separator: "\n")
        let token = OctoberAccount.shared.session?.accessToken
        let agents = model.agents
        model.show("Sending logs…")
        Task {
            let result = await runReport(details: details, token: token, version: version)
            let error = result["error"] as? String ?? "something went wrong"
            if result["sent"] as? Bool == true, let code = result["code"] as? String {
                emailProblem(agents: agents, logs: "Logs: \(code)", attachments: crashes)
            } else if let saved = result["saved"] as? String {
                emailProblem(agents: agents, logs: "Logs couldn't be sent (\(error)), so they're attached.",
                             attachments: crashes + [URL(fileURLWithPath: saved)])
            } else {
                emailProblem(agents: agents, logs: "Logs couldn't be sent: \(error)", attachments: crashes)
            }
        }
    }

    private static func emailProblem(agents: [Agent], logs: String?, attachments: [URL]) {
        compose(
            subject: "October Lantern \(version): a problem",
            body: "What happened, and what did you expect?\n\n\n---\n\(logs.map { "\($0)\n" } ?? "")\(diagnostics(agents: agents))\n",
            attachments: attachments
        )
    }

    private nonisolated static func runReport(details: String, token: String?, version: String) async -> [String: Any] {
        await Task.detached {
            guard let engine = EngineClient.engineURL() else { return ["error": "Lantern's engine is missing"] }
            let p = Process()
            p.executableURL = engine
            p.arguments = ["report", "--app", "mac", "--app-version", version]
            var env = ProcessInfo.processInfo.environment
            if let token { env["LANTERN_REPORT_TOKEN"] = token }
            p.environment = env
            let input = Pipe(), out = Pipe()
            p.standardInput = input
            p.standardOutput = out
            p.standardError = FileHandle.nullDevice
            do { try p.run() } catch { return ["error": error.localizedDescription] }
            input.fileHandleForWriting.write(Data(details.utf8))
            try? input.fileHandleForWriting.close()
            let data = out.fileHandleForReading.readDataToEndOfFile()
            p.waitUntilExit()
            let line = String(data: data, encoding: .utf8)?.split(separator: "\n").last.map(String.init) ?? ""
            return (try? JSONSerialization.jsonObject(with: Data(line.utf8))) as? [String: Any] ?? ["error": "The engine didn't say how it went"]
        }.value
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
            await OwnKey.shared.remove()
            if OwnKey.shared.saved != nil { failures.append("Removing your own AI key from the Keychain. Delete \"dev.october.lantern.ai-key\" in Keychain Access.") }
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
