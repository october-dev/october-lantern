import CoreGraphics
import ServiceManagement
import SwiftUI

/// First-run welcome: what Lantern is, the two Mac permissions it needs (and why), the optional
/// setup, then tips. Its page is saved as it goes, so a relaunch (macOS asks to reopen Lantern
/// after Screen Recording is allowed) picks up there. On later launches, while Screen Recording or
/// Accessibility is still off and the person hasn't chosen Skip for now, the permissions page
/// shows again on its own (`permissionsOnly`).
struct WelcomeView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var hooks = Hooks.shared
    @State private var hooksError: String?
    @ObservedObject var notifier = Notifier.shared
    @ObservedObject var prefs = Preferences.shared
    var onDone: () -> Void
    @State var step = 0
    /// Only the permissions page, with Done instead of the rest of the welcome.
    var permissionsOnly = false
    @State private var openAtLogin = SMAppService.mainApp.status == .enabled
    @State private var notificationsAllowed = false
    /// Screen Recording: Point & Ask sees what you select, agents see the apps they work in.
    @State private var screenAllowed = CGPreflightScreenCaptureAccess()
    @State private var screenAsked = false
    /// Accessibility: agents use your apps (Cua Driver), and Lantern sees which window you're in.
    @State private var accessAllowed = AXIsProcessTrusted()
    @State private var accessAsked = false
    private var permissionsAllowed: Bool { screenAllowed && accessAllowed }
    private let poll = Timer.publish(every: 1, on: .main, in: .common).autoconnect()

    /// Both permissions are on (the welcome's permissions page has nothing left to ask).
    static var permissionsAllowed: Bool { CGPreflightScreenCaptureAccess() && AXIsProcessTrusted() }

    enum Page { case intro, permissions, setup, done }
    private var pages: [Page] { permissionsOnly ? [.permissions] : [.intro, .permissions, .setup, .done] }
    private var page: Page { pages[min(max(step, 0), pages.count - 1)] }
    private var last: Bool { step >= pages.count - 1 }

    var body: some View {
        VStack(spacing: 0) {
            // Scrolls when a page is taller than the window, so Continue (below) is always visible.
            ScrollView(.vertical) {
                Group {
                    switch page {
                    case .intro: intro
                    case .permissions: permissions
                    case .setup: setup
                    case .done: done
                    }
                }
                .frame(maxWidth: .infinity, alignment: .top)
                .padding(.horizontal, 36)
                .padding(.top, 32)
                .padding(.bottom, 12)
            }
            .scrollIndicators(.automatic)
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            HStack {
                if pages.count > 1 {
                    HStack(spacing: 6) {
                        ForEach(pages.indices, id: \.self) { i in
                            Circle().fill(i == step ? Theme.amber : Color.white.opacity(0.2)).frame(width: 6, height: 6)
                        }
                    }
                }
                Spacer()
                if page == .permissions && !permissionsAllowed {
                    Button("Skip for now") {
                        // Not asked again on its own; Settings keeps the switches.
                        UserDefaults.standard.set(true, forKey: "permissionsSkipped")
                        if last { onDone() } else { step += 1 }
                    }
                    .buttonStyle(.plain).foregroundStyle(Theme.muted)
                    .help("Point & Ask and agents using your apps need these. You can allow them later in System Settings › Privacy & Security.")
                }
                if step > 0 {
                    Button("Back") { step -= 1 }.buttonStyle(.plain).foregroundStyle(Theme.muted)
                }
                Button(last ? (permissionsOnly ? "Done" : "Start using Lantern") : "Continue") {
                    if last { onDone() } else { step += 1 }
                }
                .buttonStyle(AmberButtonStyle())
                .keyboardShortcut(.defaultAction)
                .disabled(page == .permissions && !permissionsAllowed)
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 20)
        }
        .frame(width: 540, height: 640)
        .background(Color(white: 0.09))
        .environment(\.colorScheme, .dark)
        .task { hooks.refresh() }
        .onReceive(poll) { _ in
            if page == .permissions, !screenAllowed { screenAllowed = CGPreflightScreenCaptureAccess() }
            if page == .permissions, !accessAllowed, AXIsProcessTrusted() {
                accessAllowed = true
                // The driver checks its permissions when it starts: start it again with this one.
                model.restartComputer()
            }
        }
        .onChange(of: step) { _, now in if !permissionsOnly { UserDefaults.standard.set(now, forKey: "welcomeStep") } }
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

    private var permissions: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Let Lantern see and use your screen").font(.system(size: 22, weight: .bold)).foregroundStyle(Theme.ink)
                .fixedSize(horizontal: false, vertical: true)
            Text("Two Mac permissions make Lantern's best parts work. Here's what each one is for. You can turn them off anytime in System Settings › Privacy & Security.")
                .font(.system(size: 13.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)

            PermissionCard(
                symbol: "rectangle.dashed.badge.record", title: "Screen Recording", allowed: screenAllowed,
                why: "Lets Point & Ask see the part of your screen you select, and lets agents you start see the apps they're working in. Lantern only looks when you or an agent asks.",
                next: screenAsked
                    ? "Allowed it? macOS applies it once Lantern reopens: choose Quit & Reopen when macOS asks, or Reopen Lantern here. You'll come back to this page."
                    : "After you allow it, macOS asks to quit and reopen Lantern. That's expected: you'll come back to this page."
            ) {
                if screenAsked {
                    Button("Reopen Lantern") { Self.relaunch() }.buttonStyle(SecondaryButtonStyle())
                } else {
                    Button("Allow…") {
                        ScreenCapture.shared.requestPermission()
                        screenAsked = true
                    }
                    .buttonStyle(SecondaryButtonStyle())
                }
            }

            PermissionCard(
                symbol: "cursorarrow.click.2", title: "Accessibility", allowed: accessAllowed,
                why: "Lets agents use your apps for you: click, type and read windows in the background, with their own cursor, while you keep working. Lantern also uses it to see which window you're in.",
                next: accessAsked
                    ? "In Privacy & Security › Accessibility, switch on October Lantern. This page updates by itself."
                    : "macOS opens a list of apps: switch on October Lantern, then come back here."
            ) {
                if accessAsked {
                    Button("Open Settings…") {
                        NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!)
                    }
                    .buttonStyle(SecondaryButtonStyle())
                    .help("Switch on October Lantern under Privacy & Security › Accessibility")
                } else {
                    Button("Allow…") {
                        AppContext.shared.requestAccess()
                        accessAsked = true
                    }
                    .buttonStyle(SecondaryButtonStyle())
                }
            }

            if permissionsAllowed {
                Label("All set. Continue when you're ready.", systemImage: "checkmark.circle.fill")
                    .font(.system(size: 13, weight: .medium)).foregroundStyle(Theme.green)
            }
        }
    }

    private var setup: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("A few choices").font(.system(size: 22, weight: .bold)).foregroundStyle(Theme.ink)
            Text("All optional. You can change them later in Settings.")
                .font(.system(size: 13.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)

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

/// A permission on the welcome's permissions page: what it's for, its state, and what happens next.
private struct PermissionCard<Control: View>: View {
    let symbol: String
    let title: String
    let allowed: Bool
    let why: String
    let next: String
    @ViewBuilder var control: Control

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                Image(systemName: symbol).font(.system(size: 17)).foregroundStyle(Theme.amber).frame(width: 24)
                Text(title).font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.ink)
                Spacer(minLength: 12)
                if allowed {
                    Label("Allowed", systemImage: "checkmark").foregroundStyle(Theme.green).font(.system(size: 12, weight: .semibold))
                } else {
                    control
                }
            }
            Text(why).font(.system(size: 12.5)).foregroundStyle(Theme.ink.opacity(0.85)).fixedSize(horizontal: false, vertical: true)
            if !allowed {
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: "info.circle").font(.system(size: 11)).foregroundStyle(Theme.muted).padding(.top, 1)
                    Text(next).font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 14).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(allowed ? Theme.green.opacity(0.35) : Color.clear))
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
