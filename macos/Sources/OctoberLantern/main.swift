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
    private var welcomeWindow: NSWindow?
    private var settingsWindow: NSWindow?
    private var subscriptions = Set<AnyCancellable>()

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Development: `--snapshot <dir>` draws the welcome pages and settings tabs offscreen into
        // PNGs and quits, for checking layouts without putting windows on screen.
        if let i = CommandLine.arguments.firstIndex(of: "--snapshot"), i + 1 < CommandLine.arguments.count {
            model.start()
            snapshot(into: URL(fileURLWithPath: CommandLine.arguments[i + 1]))
            return
        }

        windows = WindowController(model: model)
        windows.onMenu = { [weak self] view in
            guard let self else { return }
            self.menu().popUp(positioning: nil, at: NSPoint(x: view.bounds.midX, y: view.bounds.midY), in: view)
        }

        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        let icon = (Assets.logo?.copy() as? NSImage) ?? NSImage(systemSymbolName: "flame", accessibilityDescription: nil)
        icon?.size = NSSize(width: 18, height: 18)
        icon?.accessibilityDescription = "October Lantern"
        statusItem.button?.image = icon
        statusItem.menu = menu()

        Preferences.shared.$hotkey
            .sink { [weak self] preset in self?.registerHotKey(preset) }
            .store(in: &subscriptions)

        Notifier.shared.setUp()
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
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { self.showWelcome() }
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
                default:
                    self.model.panel = ["agents": .agents, "new": .newSession, "october": .october][what] ?? .inbox
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

    func showWelcome(step: Int = 0) {
        if welcomeWindow == nil {
            let view = WelcomeView(model: model, onDone: { [weak self] in
                UserDefaults.standard.set(true, forKey: "welcomed")
                self?.welcomeWindow?.close()
                self?.windows.showPill()
                self?.windows.flash()
            }, step: step)
            welcomeWindow = makeWindow(title: "Welcome to October Lantern", content: view)
            welcomeWindow?.titlebarAppearsTransparent = true
        }
        present(welcomeWindow)
    }

    func showSettings(tab: Int = 0) {
        if settingsWindow == nil {
            settingsWindow = makeWindow(
                title: "October Lantern Settings",
                content: SettingsView(model: model, onShowWelcome: { [weak self] in self?.showWelcome() }, tab: tab)
            )
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

    private func present(_ window: NSWindow?) {
        NSApp.activate()
        window?.makeKeyAndOrderFront(nil)
    }

    private func snapshot(into dir: URL) {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        var jobs: [(String, AnyView)] = (0..<4).map { ("welcome-\($0)", AnyView(WelcomeView(model: model, onDone: {}, step: $0))) }
        jobs += (0..<4).map { ("settings-\($0)", AnyView(SettingsView(model: model, onShowWelcome: {}, tab: $0))) }
        let panels: [(String, PanelMode)] = [("inbox", .inbox), ("agents", .agents), ("new", .newSession), ("october", .october), ("chat", .agents)]
        jobs += panels.map { name, mode in
            ("panel-\(name)", AnyView(PanelView(model: model, dictation: model.dictation).onAppear {
                self.model.closeChat()
                self.model.panel = mode
                if name == "chat", let first = self.model.inbox.first ?? self.model.agents.first { self.model.openChat(first) }
            }))
        }
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
        item("New Session…") { [weak self] in self?.show(.newSession) }
        let compose = item("Message an Agent…") { [weak self] in self?.model.compose(to: nil) }
        if Preferences.shared.hotkey != .none { compose.title += "  \(Preferences.shared.hotkey.label)" }
        menu.addItem(.separator())
        item(windows.pillVisible ? "Hide Lantern" : "Show Lantern") { [weak self] in
            guard let self else { return }
            if self.windows.pillVisible { self.windows.hidePill() } else { self.windows.showPill() }
        }
        item("Settings…", ",") { [weak self] in self?.showSettings() }
        if Updater.shared.available {
            item("Check for Updates…") { Updater.shared.checkForUpdates() }
        }
        menu.addItem(.separator())
        item("Report a Problem…") { [weak self] in Support.reportProblem(agents: self?.model.agents ?? []) }
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
    app.run()
}
