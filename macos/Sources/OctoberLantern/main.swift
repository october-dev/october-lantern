import AppKit
import ServiceManagement

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let model = AppModel()
    private var windows: WindowController!
    private var statusItem: NSStatusItem!
    private var hotKey: HotKey?

    func applicationDidFinishLaunching(_ notification: Notification) {
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

        hotKey = HotKey { [weak self] in
            Task { @MainActor in
                guard let self else { return }
                if self.model.panel == nil { self.model.compose(to: nil) } else { self.model.panel = nil }
            }
        }

        model.start()
        windows.showPill()

        // Development: `--open inbox|agents` opens the panel at launch.
        let args = CommandLine.arguments
        if let i = args.firstIndex(of: "--open"), i + 1 < args.count {
            let mode: PanelMode = args[i + 1] == "agents" ? .agents : .inbox
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { self.model.panel = mode }
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        model.stop()
    }

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
        item("Message an Agent…  ⌃⌥Space") { [weak self] in self?.model.compose(to: nil) }
        menu.addItem(.separator())
        item(windows.pillVisible ? "Hide Lantern" : "Show Lantern") { [weak self] in
            guard let self else { return }
            if self.windows.pillVisible { self.windows.hidePill() } else { self.windows.showPill() }
        }
        let login = item("Open at Login") { Self.toggleLoginItem() }
        login.state = SMAppService.mainApp.status == .enabled ? .on : .off
        item("Exact Status with Agent Hooks…") { [weak self] in self?.offerHooks() }
        menu.addItem(.separator())
        item("Quit October Lantern", "q") { NSApp.terminate(nil) }
    }

    private func show(_ mode: PanelMode) {
        if !windows.pillVisible { windows.showPill() }
        model.panel = mode
    }

    private static func toggleLoginItem() {
        do {
            if SMAppService.mainApp.status == .enabled {
                try SMAppService.mainApp.unregister()
            } else {
                try SMAppService.mainApp.register()
            }
        } catch {
            NSLog("Lantern: login item: \(error)")
        }
    }

    /// Hooks change the agents' own config files, so explain and ask first.
    private func offerHooks() {
        guard let engine = EngineClient.engineURL() else { return }
        let status = run(engine, ["hooks", "status"])
        let installed = status.contains("\"claude\":true") || status.contains("\"codex\":true")

        let alert = NSAlert()
        if installed {
            alert.messageText = "Agent hooks are installed"
            alert.informativeText = "Claude Code and Codex tell Lantern exactly when they need you. Removing the hooks restores your previous settings, and Lantern goes back to reading session files."
            alert.addButton(withTitle: "Remove Hooks")
        } else {
            alert.messageText = "Get exact status from your agents?"
            alert.informativeText = """
            Without hooks, Lantern works out each agent's state from its session files. With hooks, Claude Code and Codex tell Lantern the moment they finish or need permission.

            This adds Lantern to the hooks in ~/.claude/settings.json and sets notify in ~/.codex/config.toml. Both files are backed up first, and an existing Codex notify program keeps working. Restart running agents afterwards.
            """
            alert.addButton(withTitle: "Install Hooks")
        }
        alert.addButton(withTitle: "Cancel")
        NSApp.activate()
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let output = run(engine, ["hooks", installed ? "uninstall" : "install"])
        model.show(output.split(separator: "\n").last.map(String.init) ?? "Done")
    }

    private func run(_ url: URL, _ args: [String]) -> String {
        let p = Process()
        p.executableURL = url
        p.arguments = args
        let out = Pipe()
        p.standardOutput = out
        p.standardError = out
        try? p.run()
        p.waitUntilExit()
        return String(data: out.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
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
