import CoreGraphics
import ServiceManagement
import SwiftUI

/// First-run welcome: what Lantern is, the agents it found, and the optional setup.
struct WelcomeView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var hooks = Hooks.shared
    @State private var hooksError: String?
    @ObservedObject var notifier = Notifier.shared
    @ObservedObject var prefs = Preferences.shared
    var onDone: () -> Void
    @State var step = 0
    @State private var openAtLogin = SMAppService.mainApp.status == .enabled
    @State private var notificationsAllowed = false
    /// Screen Recording, which Point & Ask needs: the one setup step that isn't optional.
    @State private var screenAllowed = CGPreflightScreenCaptureAccess()
    @State private var screenAsked = false
    private let poll = Timer.publish(every: 1, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(spacing: 0) {
            Group {
                switch step {
                case 0: intro
                case 1: setup
                default: done
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .padding(.horizontal, 36)
            .padding(.top, 32)

            HStack {
                HStack(spacing: 6) {
                    ForEach(0..<3) { i in
                        Circle().fill(i == step ? Theme.amber : Color.white.opacity(0.2)).frame(width: 6, height: 6)
                    }
                }
                Spacer()
                if step == 1 && !screenAllowed {
                    Button("Skip for now") { step += 1 }.buttonStyle(.plain).foregroundStyle(Theme.muted)
                        .help("Point & Ask can't capture your screen until you allow it. It asks again the first time you use it.")
                }
                if step > 0 {
                    Button("Back") { step -= 1 }.buttonStyle(.plain).foregroundStyle(Theme.muted)
                }
                Button(step == 2 ? "Start using Lantern" : "Continue") {
                    if step == 2 { onDone() } else { step += 1 }
                }
                .buttonStyle(AmberButtonStyle())
                .keyboardShortcut(.defaultAction)
                .disabled(step == 1 && !screenAllowed)
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 20)
        }
        .frame(width: 540, height: 560)
        .background(Color(white: 0.09))
        .environment(\.colorScheme, .dark)
        .task { hooks.refresh() }
        .onReceive(poll) { _ in
            if step == 1, !screenAllowed { screenAllowed = CGPreflightScreenCaptureAccess() }
        }
        .onChange(of: step) { _, now in UserDefaults.standard.set(now, forKey: "welcomeStep") }
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
            Text(model.agents.isEmpty ? "No agents running yet. Start Claude Code, Codex or another agent as usual and Lantern finds it."
                 : "Lantern already found \(model.agents.count) agent\(model.agents.count == 1 ? "" : "s") on this Mac.")
                .font(.system(size: 13, weight: .medium)).foregroundStyle(Theme.amber).multilineTextAlignment(.center)
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

    @State private var shareUsage = Analytics.shared.enabled

    private var setup: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("A few choices").font(.system(size: 22, weight: .bold)).foregroundStyle(Theme.ink)
            Text("Point & Ask needs Screen Recording. The rest is optional.")
                .font(.system(size: 13.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)

            SetupRow(symbol: "rectangle.dashed.badge.record", title: "Screen Recording (needed)",
                     detail: "To capture what you select with Point & Ask. Only when you use it.") {
                if screenAllowed {
                    Label("On", systemImage: "checkmark").foregroundStyle(Theme.green).font(.system(size: 12, weight: .semibold))
                } else if screenAsked {
                    VStack(alignment: .trailing, spacing: 4) {
                        Button("Reopen Lantern") { Self.relaunch() }.buttonStyle(SecondaryButtonStyle())
                        Text("Allowed it? macOS applies it after Lantern reopens.").font(.system(size: 11)).foregroundStyle(Theme.muted)
                            .frame(maxWidth: 180, alignment: .trailing).multilineTextAlignment(.trailing)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                } else {
                    Button("Allow…") {
                        ScreenCapture.shared.requestPermission()
                        screenAsked = true
                    }
                    .buttonStyle(SecondaryButtonStyle())
                }
            }

            SetupRow(symbol: "power", title: "Open at login", detail: "Keep the lantern there whenever you're using your Mac.") {
                Toggle("", isOn: $openAtLogin).toggleStyle(.switch).labelsHidden()
                    .onChange(of: openAtLogin) { _, on in
                        try? on ? SMAppService.mainApp.register() : SMAppService.mainApp.unregister()
                    }
            }
            SetupRow(symbol: "bell.badge", title: "Notifications", detail: "A notification when an agent finishes or asks you something, even when the lantern is out of sight.") {
                if notificationsAllowed || notifier.status == .authorized || notifier.status == .provisional {
                    Label("On", systemImage: "checkmark").foregroundStyle(Theme.green).font(.system(size: 12, weight: .semibold))
                } else if notifier.status == .denied {
                    Button("Open Settings…") { Notifier.openSystemSettings() }.buttonStyle(SecondaryButtonStyle())
                        .help("Notifications are off for Lantern in System Settings")
                } else {
                    Button("Turn On") {
                        Task { notificationsAllowed = await Notifier.shared.requestPermission() }
                    }
                    .buttonStyle(SecondaryButtonStyle())
                }
            }
            SetupRow(symbol: "bolt", title: "Exact status (recommended)",
                     detail: "Claude Code and Codex tell Lantern the moment they finish or need permission, so you can Allow or Deny here. Adds a hook to their settings (backed up first); restart running agents.") {
                if hooks.installed {
                    Label("On", systemImage: "checkmark").foregroundStyle(Theme.green).font(.system(size: 12, weight: .semibold))
                } else if hooks.busy {
                    ProgressView().controlSize(.small)
                } else {
                    VStack(alignment: .trailing, spacing: 4) {
                        Button(hooks.claudeOutdated ? "Update" : "Turn On") {
                            hooksError = nil
                            Task {
                                let out = await hooks.set(true)
                                if out.hasPrefix("Error") || out.hasPrefix("Couldn't") || out.contains("engine is missing") { hooksError = out }
                            }
                        }
                        .buttonStyle(SecondaryButtonStyle())
                        if let hooksError {
                            Text(hooksError).font(.system(size: 11)).foregroundStyle(Theme.red).frame(maxWidth: 220, alignment: .trailing)
                                .fixedSize(horizontal: false, vertical: true)
                        } else if hooks.claude != hooks.codex {
                            Text(hooks.claude ? "On for Claude Code only" : "On for Codex only").font(.system(size: 11)).foregroundStyle(Theme.muted)
                        }
                    }
                }
            }
            if Analytics.shared.available {
                SetupRow(symbol: "chart.bar", title: "Share anonymous usage counts",
                         detail: "How often Lantern and its features are used, so we know what to improve. Never your code, messages, commands or folders. If you sign in to October, counts are linked to your account.") {
                    Toggle("", isOn: $shareUsage).toggleStyle(.switch).labelsHidden()
                        .onChange(of: shareUsage) { _, on in Analytics.shared.enabled = on }
                }
            }
            SetupRow(symbol: "keyboard", title: "Shortcut", detail: "Opens the message box from anywhere.") {
                Text(prefs.hotkey.label).font(.system(size: 12.5, weight: .semibold, design: .rounded)).foregroundStyle(Theme.ink)
                    .padding(.horizontal, 8).padding(.vertical, 3).background(RoundedRectangle(cornerRadius: 6).fill(Theme.faint))
            }
        }
    }

    /// Opens a new copy of Lantern and quits this one (macOS applies Screen Recording on reopen).
    static func relaunch() {
        let config = NSWorkspace.OpenConfiguration()
        config.createsNewApplicationInstance = true
        NSWorkspace.shared.openApplication(at: Bundle.main.bundleURL, configuration: config) { _, _ in
            DispatchQueue.main.async { NSApp.terminate(nil) }
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
                Tip(symbol: "square.stack.3d.up", text: "Hover it and click the stack to see who's waiting. Click any agent to read its conversation and reply.")
                Tip(symbol: "sparkles", text: "Click the lantern to Ask Lantern: \"which agent needs me first?\", or tell it what to do and it starts an agent.")
                Tip(symbol: "cursorarrow.and.square.on.square.dashed", text: "Point & Ask: press \(Preferences.shared.pointAsk == .none ? "the dotted-box button" : Preferences.shared.pointAsk.label), drag a box around anything on screen and ask about it.")
                Tip(symbol: "plus", text: "Start a new session with any agent from the + button.")
                Tip(symbol: "menubar.rectangle", text: "Right-click the lantern, or use the lantern in your menu bar, for Settings and more.")
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
