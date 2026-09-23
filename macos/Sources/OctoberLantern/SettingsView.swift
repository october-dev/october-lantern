import ServiceManagement
import SwiftUI

struct SettingsView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var prefs = Preferences.shared
    @ObservedObject var hooks = Hooks.shared
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
        .task { hooks.refresh() }
    }

    @State private var openAtLogin = SMAppService.mainApp.status == .enabled
    @State private var autoUpdate = Updater.shared.automaticallyChecks

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
            if Updater.shared.available {
                Toggle("Check for updates automatically", isOn: $autoUpdate)
                    .onChange(of: autoUpdate) { _, on in Updater.shared.automaticallyChecks = on }
            }
            Section {
                LabeledContent("Exact status from agent hooks") {
                    Toggle("", isOn: Binding(get: { hooks.installed }, set: { model.show(hooks.set($0)) }))
                        .labelsHidden()
                        .toggleStyle(.switch)
                }
                Text("Claude Code and Codex tell Lantern the moment they finish or need permission. Lantern backs up ~/.claude/settings.json and ~/.codex/config.toml before changing them, and restores them when you turn this off. Restart running agents afterwards.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }

    private var notifications: some View {
        Form {
            Toggle("Notify me when an agent needs me", isOn: $prefs.notificationsEnabled)
            Toggle("Also when an agent finishes its turn", isOn: $prefs.notifyOnFinish).disabled(!prefs.notificationsEnabled)
            Text("Questions and permission prompts always notify (when notifications are on). Notifications are skipped for agents you've muted in Agents.")
                .font(.caption).foregroundStyle(.secondary)
            Button("Open Notification Settings…") {
                NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension")!)
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
            Section("Help") {
                Button("Report a Problem…") { Support.reportProblem(agents: model.agents) }
                Button("Show Welcome Guide") { onShowWelcome() }
                Link("lantern.october.dev", destination: Support.website)
                Link(Support.email, destination: URL(string: "mailto:\(Support.email)")!)
            }
            Section {
                Button("Uninstall October Lantern…", role: .destructive) { Support.uninstall() }
                Text("Removes the hooks (restoring your agents' settings), support files and settings, then moves the app to the Trash.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }
}
