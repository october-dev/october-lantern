import ServiceManagement
import SwiftUI

struct SettingsView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var prefs = Preferences.shared
    @ObservedObject var hooks = Hooks.shared
    @ObservedObject var notifier = Notifier.shared
    var onShowWelcome: () -> Void
    @State var tab = 0

    var body: some View {
        TabView(selection: $tab) {
            general.tabItem { Label("General", systemImage: "gearshape") }.tag(0)
            notifications.tabItem { Label("Notifications", systemImage: "bell") }.tag(1)
            agents.tabItem { Label("Agents", systemImage: "square.stack.3d.up") }.tag(2)
            about.tabItem { Label("About", systemImage: "info.circle") }.tag(3)
        }
        .frame(width: 520, height: 440)
        .task {
            hooks.refresh()
            await notifier.refreshStatus()
        }
    }

    @State private var openAtLogin = SMAppService.mainApp.status == .enabled
    @State private var autoUpdate = Updater.shared.automaticallyChecks
    @State private var shareUsage = Analytics.shared.enabled

    private var general: some View {
        Form {
            Toggle("Open at login", isOn: $openAtLogin)
                .onChange(of: openAtLogin) { _, on in try? on ? SMAppService.mainApp.register() : SMAppService.mainApp.unregister() }
            Picker("Lantern position", selection: $prefs.edge) {
                Text("Right edge").tag("right")
                Text("Left edge").tag("left")
            }
            Picker("Shortcut to message an agent", selection: $prefs.hotkey) {
                ForEach(HotKeyPreset.allCases) { Text($0.label).tag($0) }
            }
            Picker("Point & Ask (hold, point, speak)", selection: $prefs.pointAsk) {
                ForEach(PointAskPreset.allCases) { Text($0.label).tag($0) }
            }
            .help("Hold the shortcut, point at something on screen and say what you want. A card shows a screenshot around the pointer; Enter sends it to the agent.")
            if Updater.shared.available {
                Toggle("Check for updates automatically", isOn: $autoUpdate)
                    .onChange(of: autoUpdate) { _, on in Updater.shared.automaticallyChecks = on }
            }
            Section {
                Toggle("Include a screenshot of my screen when starting a session", isOn: $prefs.screenshotNewSessions)
                Text("The new agent gets a screenshot of the screen you're on (without Lantern) with its first message, for context. It stays on this Mac. Needs Screen Recording permission; you can also switch it per session.")
                    .font(.caption).foregroundStyle(.secondary)
                Toggle("Connect new sessions to October Bus", isOn: $prefs.busForSessions)
                Text("Agents Lantern starts (Claude Code, Codex, OpenCode, October) can see and message each other and share tasks. Everything stays on this Mac.")
                    .font(.caption).foregroundStyle(.secondary)
                Toggle("Also give new sessions the list of tools on this Mac", isOn: $prefs.toolkitInSessions)
                HStack {
                    Text("Lantern keeps a short list of what this Mac has (tools, apps, local models) for agents it starts. Tasks always get it. You can add your own notes at the end of the list.")
                        .font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button("Open List") {
                        let url = FileManager.default.homeDirectoryForCurrentUser
                            .appendingPathComponent("Library/Application Support/October Lantern/toolkit.md")
                        if FileManager.default.fileExists(atPath: url.path) { NSWorkspace.shared.open(url) } else { model.refreshToolkit() }
                    }
                    Button("Refresh") { model.refreshToolkit() }
                }
            } header: {
                Text("New sessions and tasks")
            }
            Section {
                HooksSetting(hooks: hooks)
            } header: {
                Text("Exact status from agent hooks")
            }
        }
        .formStyle(.grouped)
    }

    private var notifications: some View {
        Form {
            Toggle("Notify me when an agent needs me", isOn: $prefs.notificationsEnabled)
                .onChange(of: prefs.notificationsEnabled) { _, on in
                    // Ask macOS the first time they're turned on.
                    if on && notifier.status == .notDetermined { Task { _ = await notifier.requestPermission() } }
                }
            if prefs.notificationsEnabled && notifier.status == .denied {
                LabeledContent {
                    Button("Open System Settings…") { Notifier.openSystemSettings() }
                } label: {
                    Label("Off in System Settings", systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                }
                Text("macOS is blocking Lantern's notifications. Allow them for October Lantern in System Settings › Notifications.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Toggle("Also when an agent finishes its turn", isOn: $prefs.notifyOnFinish).disabled(!prefs.notificationsEnabled)
            Text("Questions and permission prompts always notify (when notifications are on). Notifications are skipped for agents you've muted in Agents.")
                .font(.caption).foregroundStyle(.secondary)
            if notifier.status != .denied {
                Button("Open Notification Settings…") { Notifier.openSystemSettings() }
            }
        }
        .formStyle(.grouped)
    }

    private var agents: some View {
        Form {
            Section {
                ForEach(allKinds, id: \.self) { kind in
                    Toggle(isOn: Binding(
                        get: { prefs.counts(kind) },
                        set: { on in if on { prefs.mutedKinds.remove(kind.rawValue) } else { prefs.mutedKinds.insert(kind.rawValue) } }
                    )) {
                        HStack(spacing: 8) {
                            if let img = Assets.harness(kind) {
                                Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 18, height: 18)
                                    .clipShape(RoundedRectangle(cornerRadius: 4))
                            }
                            Text(kind.displayName)
                        }
                    }
                }
            } header: {
                Text("Count in Waiting and notify")
            } footer: {
                Text("Turned-off agents still appear in the Agents list.").font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }

    private var allKinds: [AgentKind] {
        ["claude", "codex", "opencode", "pi", "october", "gemini", "grok", "cursor", "qwen", "goose", "aider", "amp",
         "copilot", "kimi", "droid", "crush", "auggie"].map { AgentKind(rawValue: $0) }
    }

    private var about: some View {
        Form {
            Section {
                HStack(spacing: 14) {
                    if let logo = Assets.logo {
                        Image(nsImage: logo).resizable().aspectRatio(contentMode: .fit).frame(width: 52, height: 52)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text("October Lantern").font(.title3.bold())
                        Text("Version \(Support.version)").foregroundStyle(.secondary)
                    }
                }
                if Updater.shared.available {
                    Button("Check for Updates…") { Updater.shared.checkForUpdates() }
                }
            }
            if Analytics.shared.available {
                Section {
                    Toggle("Share anonymous usage counts", isOn: $shareUsage)
                        .onChange(of: shareUsage) { _, on in Analytics.shared.enabled = on }
                    Text("How often Lantern and its features are used, so we know what to improve. Never your code, messages, commands or folder names. If you sign in to October, counts are linked to your account.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            Section("Help") {
                Button("Report a Problem…") { Support.reportProblem(agents: model.agents) }
                Button("Show Welcome Guide") { onShowWelcome() }
                Link("lantern.october.dev", destination: Support.website)
                Link(Support.email, destination: URL(string: "mailto:\(Support.email)")!)
            }
            Section {
                Button("Uninstall October Lantern…", role: .destructive) { Support.uninstall(model: model) }
                Text("Signs out of October, removes the hooks (restoring your agents' settings), support files and settings, then moves the app to the Trash.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }
}

/// Claude Code and Codex hooks, each with its own state. An older Lantern's Claude hooks (missing
/// events this version needs) offer an update. Errors show here, under the control.
struct HooksSetting: View {
    @ObservedObject var hooks: Hooks
    @State private var error: String?

    var body: some View {
        LabeledContent("Claude Code") { state(on: hooks.claude, outdated: hooks.claudeOutdated) }
        LabeledContent("Codex") { state(on: hooks.codex, outdated: false) }
        HStack {
            if hooks.busy { ProgressView().controlSize(.small) }
            Spacer()
            if hooks.anyInstalled {
                Button("Turn Off") { run(false) }.disabled(hooks.busy)
            }
            if !hooks.installed {
                Button(hooks.claudeOutdated ? "Update" : hooks.anyInstalled ? "Turn On for Both" : "Turn On") { run(true) }
                    .disabled(hooks.busy)
            }
        }
        if let error {
            Label(error, systemImage: "exclamationmark.triangle.fill").font(.caption).foregroundStyle(.orange)
                .fixedSize(horizontal: false, vertical: true)
        }
        Text("Claude Code and Codex tell Lantern the moment they finish or need permission. Lantern backs up ~/.claude/settings.json and ~/.codex/config.toml before changing them, and restores them when you turn this off. Restart running agents afterwards.")
            .font(.caption).foregroundStyle(.secondary)
    }

    @ViewBuilder private func state(on: Bool, outdated: Bool) -> some View {
        if on {
            Label("On", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
        } else if outdated {
            Label("Needs update", systemImage: "arrow.triangle.2.circlepath").foregroundStyle(.orange)
        } else {
            Text("Off").foregroundStyle(.secondary)
        }
    }

    private func run(_ on: Bool) {
        error = nil
        Task {
            let out = await hooks.set(on)
            if out.hasPrefix("Error") || out.hasPrefix("Couldn't") || out.contains("engine is missing") { error = out }
        }
    }
}
