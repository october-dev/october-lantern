import AppKit
import Combine
import ServiceManagement
import SwiftUI

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let model = AppModel()
    private var windows: WindowController!
    private var statusItem: NSStatusItem!
    private var hotKey: HotKey?
    private var pointAskKey: HotKey?
    private var welcomeWindow: NSWindow?
    private var settingsWindow: NSWindow?
    private var subscriptions = Set<AnyCancellable>()

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Development: `--snapshot <dir>` draws the welcome pages and settings tabs offscreen into
        // PNGs and quits, for checking layouts without putting windows on screen.
        // Development: `--ask <image> <question>` asks October's AI once, prints the answer and quits
        // (`--ask chat <question>` asks the way the assistant does: no image, chat mode).
        if let i = CommandLine.arguments.firstIndex(of: "--ask"), i + 2 < CommandLine.arguments.count {
            let image = NSImage(contentsOfFile: CommandLine.arguments[i + 1])?.cgImage(forProposedRect: nil, context: nil, hints: nil)
            Task { @MainActor in
                do {
                    let chat = CommandLine.arguments[i + 1] == "chat"
                    let request = OctoberAI.Request(turns: [], question: CommandLine.arguments[i + 2],
                                                    image: chat ? nil : image.flatMap(PointAsk.jpeg),
                                                    context: chat ? "- @claude-1 (Claude Code) in october-lantern: waiting; asking: Run the release script?" : "App: Safari",
                                                    mode: chat ? "chat" : nil)
                    for try await piece in OctoberAI.stream(request) {
                        switch piece {
                        case .text(let t): print(t, terminator: "")
                        case .note(let n): print("\n[note] \(n)")
                        case .via(let p): print("[via \(p)]")
                        case .task(let prompt, let agent, let project, let to): print("\n[task] agent=\(agent ?? "-") project=\(project ?? "-") to=\(to ?? "-") prompt=\(prompt)")
                        case .guide(let g, let goal): print("\n[guide] \(g ?? [:]) goal=\(goal ?? "-")")
                        }
                        fflush(stdout)
                    }
                    print("\n[done]")
                } catch {
                    print("\n[error] \((error as? LocalizedError)?.errorDescription ?? "\(error)")")
                }
                exit(0)
            }
            return
        }
        // The question cards with sample questions, without starting the engine.
        if let i = CommandLine.arguments.firstIndex(of: "--snapshot-questions"), i + 1 < CommandLine.arguments.count {
            let dir = URL(fileURLWithPath: CommandLine.arguments[i + 1])
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            var cards: [(String, AnyView)] = QuestionCard.samples().map { name, agent in
                (name, AnyView(QuestionCard(agent: agent, model: model).padding(14).frame(width: 380)))
            }
            model.answered["p5"] = .picks([[1]])
            func next() {
                guard !cards.isEmpty else { exit(0) }
                let (name, card) = cards.removeFirst()
                let view = card.background(Color.black)
                let window = NSWindow(contentViewController: NSHostingController(rootView: view.environment(\.colorScheme, .dark)))
                window.appearance = NSAppearance(named: .darkAqua)
                window.setFrameOrigin(NSPoint(x: -10000, y: -10000))
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                    if let content = window.contentView, let rep = content.bitmapImageRepForCachingDisplay(in: content.bounds) {
                        content.cacheDisplay(in: content.bounds, to: rep)
                        try? rep.representation(using: .png, properties: [:])?.write(to: dir.appendingPathComponent("question-\(name).png"))
                    }
                    window.close()
                    next()
                }
            }
            next()
            return
        }
        if let i = CommandLine.arguments.firstIndex(of: "--snapshot"), i + 1 < CommandLine.arguments.count {
            model.start()
            snapshot(into: URL(fileURLWithPath: CommandLine.arguments[i + 1]))
            return
        }

        // One Lantern at a time: a second copy would run its own engine and save its own chats
        // over the first's. Opening Lantern again shows the one that's running instead.
        if Self.anotherCopyRunning() {
            DistributedNotificationCenter.default().postNotificationName(Self.showRunning, object: nil, deliverImmediately: true)
            NSApp.terminate(nil)
            return
        }
        DistributedNotificationCenter.default().addObserver(forName: Self.showRunning, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                AppLog.write("opened again: showing Ask Lantern")
                if !self.windows.pillVisible { self.windows.showPill() }
                if self.model.panel == nil { self.model.panel = .assistant } else { self.model.reshowPanel.send() }
            }
        }
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
        AppLog.write("Lantern \(version) started (macOS \(ProcessInfo.processInfo.operatingSystemVersionString))")

        windows = WindowController(model: model)
        // The lantern's right-click menu ends with Settings…
        windows.onMenu = { [weak self] _ in self?.showSettings() }

        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        let icon = (Assets.logo?.copy() as? NSImage) ?? NSImage(systemSymbolName: "flame", accessibilityDescription: nil)
        icon?.size = NSSize(width: 18, height: 18)
        icon?.accessibilityDescription = "October Lantern"
        statusItem.button?.image = icon
        statusItem.menu = menu()

        Preferences.shared.$hotkey
            .sink { [weak self] preset in self?.registerHotKey(preset) }
            .store(in: &subscriptions)
        PointAsk.shared.model = model
        Assistant.shared.model = model
        Preferences.shared.$pointAsk
            .sink { [weak self] preset in self?.registerPointAsk(preset) }
            .store(in: &subscriptions)

        Notifier.shared.setUp()
        AppContext.shared.start()
        Notifier.shared.onOpen = { [weak self] agentId in
            guard let self, let agent = self.model.agents.first(where: { $0.id == agentId }) else { return }
            if !self.windows.pillVisible { self.windows.showPill() }
            self.model.openChat(agent)
        }
        model.onNotify = { agent in Notifier.shared.post(for: agent) }

        _ = Updater.shared
        // Restore a saved October sign-in now, not when the October panel is first opened.
        _ = OctoberAccount.shared
        model.start()
        windows.showPill()

        if !UserDefaults.standard.bool(forKey: "welcomed") {
            // Give the engine a moment to find agents so the welcome can show them.
            // Reopened mid-welcome (macOS asks to after allowing Screen Recording): pick up there.
            let step = UserDefaults.standard.integer(forKey: "welcomeStep")
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { self.showWelcome(step: step) }
        } else if !WelcomeView.permissionsAllowed, !UserDefaults.standard.bool(forKey: "permissionsSkipped") {
            // Screen Recording or Accessibility is still off (allowed after the welcome, so macOS
            // reopened Lantern; or set up before the welcome asked for them): the permissions page
            // again, until both are on or the person chooses Skip for now.
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { self.showWelcome(permissionsOnly: true) }
        } else {
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { Support.offerCrashReportIfNeeded(agents: self.model.agents) }
        }

        // Development: `--open inbox|agents|new|october|chat` opens that panel at launch
        // (`chat` opens the conversation with the first waiting agent); `--open welcome|settings`
        // opens those windows (`welcome:2`, `settings:3` pick a page).
        let args = CommandLine.arguments
        if let i = args.firstIndex(of: "--open"), i + 1 < args.count {
            let what = args[i + 1]
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
                let page = Int(what.split(separator: ":").last ?? "") ?? 0
                switch what.split(separator: ":").first ?? "" {
                case "welcome": self.showWelcome(step: page)
                case "settings": self.showSettings(tab: page)
                case "point": PointAsk.shared.showDemo()
                default:
                    self.model.panel = ["agents": .agents, "new": .newSession, "october": .october, "assistant": .assistant, "routines": .routines][what] ?? .inbox
                    if what == "chat", let first = self.model.inbox.first ?? self.model.agents.first {
                        self.model.openChat(first)
                    }
                }
            }
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        Analytics.shared.persist()
        model.stop()
    }

    /// Point & Ask: hold the shortcut, point and speak, let go to see the card.
    private func registerPointAsk(_ preset: PointAskPreset) {
        pointAskKey = nil
        guard let code = preset.keyCode else { return }
        pointAskKey = HotKey(
            keyCode: code, modifiers: preset.modifiers, id: 2,
            onRelease: { Task { @MainActor in PointAsk.shared.released() } }
        ) { Task { @MainActor in PointAsk.shared.pressed() } }
    }

    private func registerHotKey(_ preset: HotKeyPreset) {
        hotKey = nil
        guard let code = preset.keyCode else { return }
        hotKey = HotKey(keyCode: code, modifiers: preset.modifiers) { [weak self] in
            Task { @MainActor in
                guard let self else { return }
                if self.model.panel == nil { self.model.compose(to: nil) } else { self.model.panel = nil }
            }
        }
    }

    // MARK: Windows

    func showWelcome(step: Int = 0, permissionsOnly: Bool = false) {
        if welcomeWindow == nil {
            let view = WelcomeView(model: model, onDone: { [weak self] in
                UserDefaults.standard.set(true, forKey: "welcomed")
                self?.welcomeWindow?.close()
                self?.windows.showPill()
                self?.windows.flash()
            }, step: permissionsOnly ? 0 : step, permissionsOnly: permissionsOnly)
            welcomeWindow = makeWindow(title: permissionsOnly ? "October Lantern" : "Welcome to October Lantern", content: view)
            welcomeWindow?.titlebarAppearsTransparent = true
        }
        present(welcomeWindow)
    }

    /// Settings, on page `tab` when one is named (an open window turns to it).
    func showSettings(tab: Int? = nil) {
        let view = SettingsView(model: model, onShowWelcome: { [weak self] in self?.showWelcome() }, tab: tab ?? 0)
        if settingsWindow == nil {
            settingsWindow = makeWindow(title: "October Lantern Settings", content: view)
        } else if tab != nil {
            settingsWindow?.contentViewController = NSHostingController(rootView: view)
        }
        present(settingsWindow)
    }

    private func makeWindow<V: View>(title: String, content: V) -> NSWindow {
        let window = NSWindow(contentViewController: NSHostingController(rootView: content))
        window.title = title
        window.styleMask = [.titled, .closable, .fullSizeContentView]
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: .darkAqua)
        window.center()
        return window
    }

    private static let showRunning = Notification.Name("dev.october.lantern.show")

    /// Another October Lantern is running (an update's relaunch gets a few seconds for the old one
    /// to quit). `--allow-second-copy` is for development.
    private static func anotherCopyRunning() -> Bool {
        guard let id = Bundle.main.bundleIdentifier, !CommandLine.arguments.contains("--allow-second-copy") else { return false }
        let me = ProcessInfo.processInfo.processIdentifier
        for _ in 0..<15 {
            let others = NSRunningApplication.runningApplications(withBundleIdentifier: id)
                .filter { $0.processIdentifier != me && !$0.isTerminated }
            if others.isEmpty { return false }
            Thread.sleep(forTimeInterval: 0.2)
        }
        AppLog.write("another copy of Lantern is already running; showing it instead")
        return true
    }

    private func present(_ window: NSWindow?) {
        NSApp.activate()
        window?.makeKeyAndOrderFront(nil)
        // At launch (macOS reopening Lantern after Screen Recording was allowed) activating can be
        // refused, and Lantern has no Dock icon to find the window by: put it in front anyway.
        window?.orderFrontRegardless()
    }

    private func snapshot(into dir: URL) {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        var jobs: [(String, AnyView)] = (0..<4).map { ("welcome-\($0)", AnyView(WelcomeView(model: model, onDone: {}, step: $0))) }
        jobs.append(("welcome-permissions-only", AnyView(WelcomeView(model: model, onDone: {}, permissionsOnly: true))))
        jobs += (0..<4).map { ("settings-\($0)", AnyView(SettingsView(model: model, onShowWelcome: {}, tab: $0))) }
        let panels: [(String, PanelMode)] = [("inbox", .inbox), ("agents", .agents), ("new", .newSession), ("october", .october), ("chat", .agents), ("assistant", .assistant),
                                                ("starting", .inbox), ("starting-failed", .inbox),
                                                ("assistant-recent", .assistant), ("assistant-task", .assistant)]
        jobs += panels.map { name, mode in
            ("panel-\(name)", AnyView(PanelView(model: model, dictation: model.dictation).onAppear {
                self.model.closeChat()
                self.model.panel = mode
                if name == "chat", let first = self.model.inbox.first ?? self.model.agents.first { self.model.openChat(first) }
                if name.hasPrefix("assistant-") { Assistant.shared.demo(thread: name == "assistant-task") }
                if name.hasPrefix("starting") {
                    self.model.showPreview(LaunchPreview(
                        kind: .claude, prompt: "Find my passport scan and tell me where it is.\n\n(Started from October Lantern on macOS.)",
                        folder: NSHomeDirectory(), error: name == "starting" ? nil : "tmux exited before the agent started."))
                }
            }))
        }
        jobs.append(("panel-assistant-signed-out", AnyView(AssistantHome(model: model, account: .detached()).frame(width: 380).background(Color.black))))
        jobs.append(("account-signed-out", AnyView(AccountCard(account: .detached()).padding(14).frame(width: 380).background(Color.black))))
        jobs.append(("point-ask", AnyView(PointAskCard(point: PointAsk.shared).onAppear { PointAsk.shared.demo(answered: false) })))
        jobs.append(("point-ask-answer", AnyView(PointAskCard(point: PointAsk.shared).onAppear { PointAsk.shared.demo(answered: true) })))
        func next() {
            guard !jobs.isEmpty else { NSApp.terminate(nil); return }
            let (name, view) = jobs.removeFirst()
            let window = NSWindow(contentViewController: NSHostingController(rootView: view.environment(\.colorScheme, .dark)))
            window.appearance = NSAppearance(named: .darkAqua)
            window.setFrameOrigin(NSPoint(x: -10000, y: -10000))
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.2) {
                guard let content = window.contentView, let rep = content.bitmapImageRepForCachingDisplay(in: content.bounds) else { next(); return }
                content.cacheDisplay(in: content.bounds, to: rep)
                try? rep.representation(using: .png, properties: [:])?.write(to: dir.appendingPathComponent("\(name).png"))
                window.close()
                next()
            }
        }
        // Let the engine report agents first, so the agent page has something to show.
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) { next() }
    }

    // MARK: Menu

    private func menu() -> NSMenu {
        let menu = NSMenu()
        menu.delegate = self
        return menu
    }

    fileprivate func populate(_ menu: NSMenu) {
        menu.removeAllItems()
        @discardableResult
        func item(_ title: String, _ key: String = "", _ action: @escaping () -> Void) -> NSMenuItem {
            let i = ClosureMenuItem(title: title, key: key, action: action)
            menu.addItem(i)
            return i
        }
        item("Waiting (\(model.inbox.count))") { [weak self] in self?.show(.inbox) }
        item("Agents (\(model.agents.count))") { [weak self] in self?.show(.agents) }
        item("New Session or Task…") { [weak self] in
            AppContext.shared.capture()
            self?.show(.newSession)
        }
        let point = item("Point & Ask…") { PointAsk.shared.start(fromPill: true) }
        if Preferences.shared.pointAsk != .none { point.title += "  \(Preferences.shared.pointAsk.label)" }
        let compose = item("Message an Agent…") { [weak self] in self?.model.compose(to: nil) }
        if Preferences.shared.hotkey != .none { compose.title += "  \(Preferences.shared.hotkey.label)" }
        menu.addItem(.separator())
        item(windows.pillVisible ? "Hide Lantern" : "Show Lantern") { [weak self] in
            guard let self else { return }
            if self.windows.pillVisible { self.windows.hidePill() } else { self.windows.showPill() }
        }
        item("Routines") { [weak self] in self?.show(.routines) }
        item("October…") { [weak self] in self?.show(.october) }
        item("Settings…", ",") { [weak self] in self?.showSettings() }
        if Updater.shared.available {
            item("Check for Updates…") { Updater.shared.checkForUpdates() }
        }
        menu.addItem(.separator())
        item("Report a Problem…") { [weak self] in Support.reportProblem(agents: self?.model.agents ?? []) }
        item("Send Logs…") { [weak self] in if let model = self?.model { Support.sendLogs(model: model) } }
        item("Welcome Guide") { [weak self] in self?.showWelcome() }
        menu.addItem(.separator())
        item("Quit October Lantern", "q") { NSApp.terminate(nil) }
    }

    private func show(_ mode: PanelMode) {
        if !windows.pillVisible { windows.showPill() }
        model.panel = mode
    }
}

extension AppDelegate: NSMenuDelegate {
    nonisolated func menuNeedsUpdate(_ menu: NSMenu) {
        MainActor.assumeIsolated { populate(menu) }
    }
}

final class ClosureMenuItem: NSMenuItem {
    private let handler: () -> Void

    init(title: String, key: String, action: @escaping () -> Void) {
        handler = action
        super.init(title: title, action: #selector(fire), keyEquivalent: key)
        target = self
    }

    required init(coder: NSCoder) { fatalError() }

    @objc private func fire() { handler() }
}

MainActor.assumeIsolated {
    let app = NSApplication.shared
    let delegate = AppDelegate()
    app.delegate = delegate
    app.setActivationPolicy(.accessory)
    app.mainMenu = editMenu()
    app.run()
}

/// Lantern has no menu bar of its own, but text fields only get ⌘C, ⌘V, ⌘X, ⌘A and ⌘Z from the
/// main menu's key equivalents, so it has a hidden one with just the Edit commands.
@MainActor
func editMenu() -> NSMenu {
    let main = NSMenu()
    let editItem = NSMenuItem()
    main.addItem(editItem)
    let edit = NSMenu(title: "Edit")
    edit.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
    let redo = edit.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "z")
    redo.keyEquivalentModifierMask = [.command, .shift]
    edit.addItem(.separator())
    edit.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
    edit.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
    edit.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
    edit.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
    editItem.submenu = edit
    return main
}
