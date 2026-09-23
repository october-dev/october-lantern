import ServiceManagement
import SwiftUI

/// First-run welcome: what Lantern is, the agents it found, and the optional setup.
struct WelcomeView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var hooks = Hooks.shared
    @ObservedObject var prefs = Preferences.shared
    var onDone: () -> Void
    @State var step = 0
    @State private var openAtLogin = SMAppService.mainApp.status == .enabled
    @State private var notificationsAllowed = false

    var body: some View {
        VStack(spacing: 0) {
            Group {
                switch step {
                case 0: intro
                case 1: agents
                case 2: setup
                default: done
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .padding(.horizontal, 36)
            .padding(.top, 32)

            HStack {
                HStack(spacing: 6) {
                    ForEach(0..<4) { i in
                        Circle().fill(i == step ? Theme.amber : Color.white.opacity(0.2)).frame(width: 6, height: 6)
                    }
                }
                Spacer()
                if step > 0 && step < 3 {
                    Button("Back") { step -= 1 }.buttonStyle(.plain).foregroundStyle(Theme.muted)
                }
                Button(step == 3 ? "Start using Lantern" : "Continue") {
                    if step == 3 { onDone() } else { step += 1 }
                }
                .buttonStyle(AmberButtonStyle())
                .keyboardShortcut(.defaultAction)
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 20)
        }
        .frame(width: 540, height: 560)
        .background(Color(white: 0.09))
        .environment(\.colorScheme, .dark)
        .task { hooks.refresh() }
    }

    private var intro: some View {
        VStack(spacing: 18) {
            if let logo = Assets.logo {
                Image(nsImage: logo).resizable().aspectRatio(contentMode: .fit).frame(width: 96, height: 96)
                    .shadow(color: Theme.amber.opacity(0.5), radius: 24)
            }
            Text("October Lantern").font(.system(size: 26, weight: .bold)).foregroundStyle(Theme.ink)
            Text("A small light on the edge of your screen that watches every coding agent on your Mac, and lights up when one of them needs you.")
                .font(.system(size: 14)).foregroundStyle(Theme.muted).multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 12) {
                StateRow(glow: nil, count: nil, title: "Dim", detail: "Nothing needs you.")
                StateRow(glow: .orange, count: nil, title: "Soft glow", detail: "Your agents are working.")
                StateRow(glow: Theme.amber, count: 2, title: "Amber, with a number", detail: "That many agents finished or are asking you something.")
            }
            .padding(16)
            .background(RoundedRectangle(cornerRadius: 14).fill(Theme.faint))
            .padding(.top, 6)
        }
    }

    private var agents: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(model.agents.isEmpty ? "No agents running right now" : "Lantern found \(model.agents.count) agent\(model.agents.count == 1 ? "" : "s")")
                .font(.system(size: 22, weight: .bold)).foregroundStyle(Theme.ink)
            Text(model.agents.isEmpty
                 ? "Start Claude Code, Codex, OpenCode, Pi, Gemini or another agent in any terminal and it will appear here. You don't need to start them any differently."
                 : "These are running on your Mac right now. You don't need to start them any differently: Lantern finds agents in any terminal.")
                .font(.system(size: 13.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
            if !model.agents.isEmpty {
                VStack(spacing: 4) {
                    ForEach(model.ranked.prefix(6)) { AgentRow(agent: $0, selected: false) }
                    if model.agents.count > 6 {
                        Text("and \(model.agents.count - 6) more").font(.system(size: 12)).foregroundStyle(Theme.muted)
                    }
                }
                .padding(8)
                .background(RoundedRectangle(cornerRadius: 14).fill(Theme.faint))
            }
            Text("Works with").font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.muted).padding(.top, 4)
            HStack(spacing: 10) {
                ForEach(["claude", "codex", "opencode", "pi", "gemini", "grok", "cursor", "qwen", "october"], id: \.self) { k in
                    if let img = Assets.harness(AgentKind(rawValue: k)) {
                        Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 26, height: 26)
                            .clipShape(RoundedRectangle(cornerRadius: 6)).help(AgentKind(rawValue: k).displayName)
                    }
                }
            }
        }
    }

    private var setup: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("A few choices").font(.system(size: 22, weight: .bold)).foregroundStyle(Theme.ink)
            Text("All optional. You can change them later in Settings.").font(.system(size: 13.5)).foregroundStyle(Theme.muted)

            SetupRow(symbol: "power", title: "Open at login", detail: "Keep the lantern there whenever you're using your Mac.") {
                Toggle("", isOn: $openAtLogin).toggleStyle(.switch).labelsHidden()
                    .onChange(of: openAtLogin) { _, on in
                        try? on ? SMAppService.mainApp.register() : SMAppService.mainApp.unregister()
                    }
            }
            SetupRow(symbol: "bell.badge", title: "Notifications", detail: "A notification when an agent finishes or asks you something, even when the lantern is out of sight.") {
                if notificationsAllowed {
                    Label("On", systemImage: "checkmark").foregroundStyle(Theme.green).font(.system(size: 12, weight: .semibold))
                } else {
                    Button("Turn On") {
                        Task { notificationsAllowed = await Notifier.shared.requestPermission() }
                    }
                    .buttonStyle(SecondaryButtonStyle())
                }
            }
            SetupRow(symbol: "bolt", title: "Exact status (recommended)",
                     detail: "Lets Claude Code and Codex tell Lantern the moment they finish or need permission, so you can Allow or Deny from Lantern. Adds Lantern to ~/.claude/settings.json and ~/.codex/config.toml, after backing both up. Restart running agents afterwards.") {
                if hooks.installed {
                    Label("On", systemImage: "checkmark").foregroundStyle(Theme.green).font(.system(size: 12, weight: .semibold))
                } else {
                    Button("Turn On") { hooks.set(true) }.buttonStyle(SecondaryButtonStyle())
                }
            }
            SetupRow(symbol: "keyboard", title: "Shortcut", detail: "Opens the message box from anywhere.") {
                Text(prefs.hotkey.label).font(.system(size: 12.5, weight: .semibold, design: .rounded)).foregroundStyle(Theme.ink)
                    .padding(.horizontal, 8).padding(.vertical, 3).background(RoundedRectangle(cornerRadius: 6).fill(Theme.faint))
            }
        }
    }

    private var done: some View {
        VStack(spacing: 16) {
            if let logo = Assets.logo {
                Image(nsImage: logo).resizable().aspectRatio(contentMode: .fit).frame(width: 72, height: 72)
            }
            Text("You're set").font(.system(size: 24, weight: .bold)).foregroundStyle(Theme.ink)
            VStack(alignment: .leading, spacing: 10) {
                Tip(symbol: "arrow.right.to.line", text: "The lantern sits on the \(prefs.edge) edge of your screen. Drag it to move it.")
                Tip(symbol: "cursorarrow", text: "Hover over it to see everything it can do. Click it to see who's waiting.")
                Tip(symbol: "bubble.left.and.bubble.right", text: "Click any agent to read its conversation and reply to it.")
                Tip(symbol: "plus", text: "Start a new session with any agent from the + button.")
                Tip(symbol: "menubar.rectangle", text: "Settings, updates and help are in the lantern in your menu bar.")
            }
            .padding(16)
            .background(RoundedRectangle(cornerRadius: 14).fill(Theme.faint))
        }
    }
}

private struct StateRow: View {
    let glow: Color?
    let count: Int?
    let title: String
    let detail: String

    var body: some View {
        HStack(spacing: 14) {
            ZStack(alignment: .bottom) {
                if let glow { Circle().fill(glow.opacity(0.5)).blur(radius: 8) }
                if let logo = Assets.logo {
                    Image(nsImage: logo).resizable().aspectRatio(contentMode: .fit)
                        .saturation(glow == nil ? 0.55 : 1).opacity(glow == nil ? 0.8 : 1)
                }
                if let count {
                    Text("\(count)").font(.system(size: 9, weight: .bold, design: .rounded)).foregroundStyle(.black.opacity(0.85))
                        .frame(minWidth: 14, minHeight: 14).background(Capsule().fill(Theme.amber)).offset(y: 3)
                }
            }
            .frame(width: 32, height: 32)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                Text(detail).font(.system(size: 12)).foregroundStyle(Theme.muted)
            }
        }
    }
}

private struct SetupRow<Control: View>: View {
    let symbol: String
    let title: String
    let detail: String
    @ViewBuilder var control: Control

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: symbol).font(.system(size: 15)).foregroundStyle(Theme.amber).frame(width: 22)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.system(size: 13.5, weight: .semibold)).foregroundStyle(Theme.ink)
                Text(detail).font(.system(size: 12)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 12)
            control
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 12).fill(Theme.faint))
    }
}

private struct Tip: View {
    let symbol: String
    let text: String

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: symbol).foregroundStyle(Theme.amber).frame(width: 18)
            Text(text).font(.system(size: 13)).foregroundStyle(Theme.ink.opacity(0.9)).fixedSize(horizontal: false, vertical: true)
        }
    }
}

struct AmberButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(.black.opacity(0.85))
            .padding(.horizontal, 16).padding(.vertical, 8)
            .background(RoundedRectangle(cornerRadius: 9, style: .continuous).fill(Theme.amber.opacity(configuration.isPressed ? 0.8 : 1)))
    }
}

struct SecondaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(Theme.ink)
            .padding(.horizontal, 12).padding(.vertical, 6)
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Color.white.opacity(configuration.isPressed ? 0.18 : 0.1)))
    }
}
