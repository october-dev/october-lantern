import AppKit
import SwiftUI

/// Header for the standalone panels (new session, October).
struct PanelTitle: View {
    let title: String
    @ObservedObject var model: AppModel

    var body: some View {
        HStack {
            Text(title.uppercased()).font(.system(size: 11, weight: .semibold)).tracking(0.8).foregroundStyle(Theme.ink)
            Spacer()
            Button { model.panel = nil } label: {
                Image(systemName: "xmark").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.muted)
                    .frame(width: 22, height: 22)
            }
            .buttonStyle(.plain)
            .keyboardShortcut(.cancelAction)
            .help("Close")
            .accessibilityLabel("Close")
        }
        .padding(.horizontal, 16).padding(.top, 14).padding(.bottom, 10)
    }
}

/// Start a new agent session: pick an agent, a folder, an optional first message, and where it runs.
/// Opened while you're in an app other than a terminal (DaVinci Resolve, Preview, Keynote...), it
/// becomes a task for that app: what you want done there comes first, and the agent is told about
/// the app, its document, your screen and what this Mac already has. ✕ on the app card makes it
/// a plain session.
struct NewSessionView: View {
    @ObservedObject var model: AppModel
    @State private var taskDismissed = false
    @ObservedObject private var app = AppContext.shared
    @State private var screenshotOn = false
    @State private var kind: AgentKind?
    @State private var folder: String?
    @State private var prompt = ""
    @State private var background = false
    /// nil: the agent's own default model.
    @State private var chosenModel: String?
    @State private var searchingModels = false
    @State private var modelQuery = ""
    @ObservedObject private var prefs = Preferences.shared
    @ObservedObject private var shot = ScreenCapture.shared
    /// Why the last start failed, shown above the Start button (the form keeps what you typed).
    @State private var failure: String?
    @ObservedObject private var metrics = PanelMetrics.shared

    private let columns = Array(repeating: GridItem(.flexible(), spacing: 8), count: 4)

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            form.padding(.horizontal, 16)
            footer.padding(.horizontal, 16)
        }
        .padding(.bottom, 16)
        .task {
            // The screen as it is when you open the panel (Lantern's own windows left out).
            screenshotOn = task || prefs.screenshotNewSessions
            if screenshotOn { await shot.capture() }
        }
        .onAppear { kindChanged() }
        .onChange(of: selectedKind) { kindChanged() }
        .onChange(of: model.launching) { was, now in
            // A start that succeeded moves to the Agents list; one that failed leaves us here.
            if was && !now && model.panel == .newSession {
                failure = model.launchError ?? "Couldn't start the session."
            }
        }
    }

    /// The message first, then one row of agents and one row of options.
    private var form: some View {
        VStack(alignment: .leading, spacing: 10) {
            if app.target != nil, !app.isTerminal { workOnApp }
            promptField(task ? "What should it do in \(app.target?.name ?? "this app")?" : "What should it work on? (optional)")
            agentRow
            optionRow
            if screenshotOn { screenshotPreview }
        }
    }

    // MARK: Agents

    /// Agents in the order you last used them; the rest keep Lantern's order.
    private func ordered(_ installed: [AgentKind]) -> [AgentKind] {
        let used = UserDefaults.standard.stringArray(forKey: "agentOrder") ?? []
        return installed.enumerated().sorted { a, b in
            let (ia, ib) = (used.firstIndex(of: a.element.rawValue) ?? Int.max, used.firstIndex(of: b.element.rawValue) ?? Int.max)
            return ia != ib ? ia < ib : a.offset < b.offset
        }.map(\.element)
    }

    @ViewBuilder private var agentRow: some View {
        if let installed = model.installedKinds {
            let all = ordered(installed)
            let shown = Array(all.prefix(5))
            let rest = Array(all.dropFirst(5))
            let others = AgentKind.supported.filter { !installed.contains($0) }
            HStack(spacing: 6) {
                if installed.isEmpty {
                    Text("No agents found on this Mac yet.").font(.system(size: 12)).foregroundStyle(Theme.muted)
                }
                ForEach(shown, id: \.self) { k in
                    AgentChip(kind: k, selected: selectedKind == k) { kind = k }
                }
                if !rest.isEmpty || !others.isEmpty {
                    Menu {
                        ForEach(rest, id: \.self) { k in Button(k.displayName) { kind = k } }
                        if !rest.isEmpty && !others.isEmpty { Divider() }
                        ForEach(others, id: \.self) { k in
                            Button("Get \(k.displayName)…") { if let url = k.website { NSWorkspace.shared.open(url) } }
                        }
                    } label: {
                        Image(systemName: "ellipsis").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.muted)
                            .frame(width: 30, height: 28)
                            .background(Capsule().fill(Theme.faint))
                    }
                    .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                    .help("More agents")
                    .accessibilityLabel("More agents")
                }
                Spacer(minLength: 0)
            }
        } else {
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Looking for installed agents…").font(.system(size: 12)).foregroundStyle(Theme.muted)
            }
        }
    }

    // MARK: Options

    private var optionRow: some View {
        HStack(spacing: 6) {
            if let k = selectedKind { modelChip(k) }
            folderChip
            screenshotChip
            openInChip
            Spacer(minLength: 0)
        }
    }

    @ViewBuilder private func modelChip(_ k: AgentKind) -> some View {
        if let list = model.models[k.rawValue] {
            if list.choosable {
                Menu {
                    Button("Default") { chosenModel = nil }
                    if !list.models.isEmpty && list.models.count <= 25 {
                        Divider()
                        ForEach(list.models, id: \.id) { m in Button(m.label) { chosenModel = m.id } }
                    }
                    Divider()
                    Button(list.models.count > 25 ? "Search \(list.models.count) models…" : "Other model…") { searchingModels = true }
                } label: {
                    Chip(symbol: "cpu", text: modelLabel(list), maxWidth: 88)
                }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                .help("Model: \(modelLabel(list))")
                .popover(isPresented: $searchingModels, arrowEdge: .bottom) {
                    modelSearch(list).padding(10).frame(width: 280)
                }
            } else {
                Chip(symbol: "cpu", text: "Default", maxWidth: 88).opacity(0.6)
                    .help("\(k.displayName) starts with its own default model.")
            }
        } else {
            Chip(symbol: "cpu", text: "Model…", maxWidth: 88).opacity(0.6).help("Loading \(k.displayName)'s models…")
        }
    }

    private var folderChip: some View {
        Menu {
            if task, let doc = documentFolder {
                Button("\(Self.short(doc)) (the document's folder)") { folder = doc }
                Divider()
            }
            ForEach(model.recentFolders, id: \.self) { f in Button(Self.short(f)) { folder = f } }
            if !model.recentFolders.isEmpty { Divider() }
            Button("Choose Folder…") { chooseFolder() }
        } label: {
            Chip(symbol: "folder", text: selectedFolder.map { URL(fileURLWithPath: $0).lastPathComponent } ?? "Folder", maxWidth: 100)
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
        .help(selectedFolder.map { "Runs in \(Self.short($0))" } ?? "Choose a folder")
    }

    /// On: a screenshot of your screen goes with the first message; its preview (and Retake)
    /// shows under the chips.
    private var screenshotChip: some View {
        Button {
            screenshotOn.toggle()
            // A plain session's choice is remembered; a task always starts with it on.
            if !task { prefs.screenshotNewSessions = screenshotOn }
            if screenshotOn { Task { await shot.capture() } } else { shot.discard() }
        } label: {
            Chip(symbol: screenshotOn ? "camera.fill" : "camera", text: nil, active: screenshotOn)
        }
        .buttonStyle(.plain)
        .help(screenshotOn ? "A screenshot of your screen goes with the first message" : "Include a screenshot of your screen")
        .accessibilityLabel(screenshotOn ? "Screenshot on" : "Screenshot off")
    }

    private var openInChip: some View {
        Menu {
            Button("Terminal window") { background = false }
            Button("Background (open it later)") { background = true }.disabled(!model.tmuxAvailable)
        } label: {
            Chip(symbol: background ? "moon" : "macwindow", text: background ? "Background" : "Terminal", maxWidth: 84)
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
        .help(model.tmuxAvailable
            ? "Lantern can reply directly to sessions it starts. Background sessions open in Terminal when you click Open."
            : "Install tmux (brew install tmux) so Lantern can reply directly and run sessions in the background.")
    }

    /// Always visible, however long the form.
    private var footer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let failure {
                Label(failure, systemImage: "exclamationmark.triangle.fill")
                    .font(.system(size: 11.5)).foregroundStyle(Theme.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Button(action: start) {
                HStack {
                    if model.launching { ProgressView().controlSize(.small) }
                    Text(model.launching ? "Starting…" : task ? "Start task" : "Start \(selectedKind?.displayName ?? "session")")
                }
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.black.opacity(0.85))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 9)
                .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.amber.opacity(canStart ? 1 : 0.35)))
            }
            .buttonStyle(.plain)
            .disabled(!canStart)
            .keyboardShortcut(.defaultAction)
        }
    }

    @ViewBuilder private var screenshotPreview: some View {
        if let image = shot.image {
            HStack(alignment: .top, spacing: 10) {
                Image(nsImage: NSImage(cgImage: image, size: .zero)).resizable().aspectRatio(contentMode: .fit)
                    .frame(width: 120)
                    .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(Theme.hairline))
                VStack(alignment: .leading, spacing: 6) {
                    Text("Saved on this Mac and given to the agent with its first message.")
                        .font(.system(size: 11)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                    Button("Retake") { Task { await shot.capture() } }
                        .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.amber)
                }
            }
        } else if shot.capturing {
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Taking a screenshot…").font(.system(size: 11)).foregroundStyle(Theme.muted)
            }
        } else if let error = shot.error {
            VStack(alignment: .leading, spacing: 6) {
                Text(error).font(.system(size: 11)).foregroundStyle(Theme.red).fixedSize(horizontal: false, vertical: true)
                if shot.needsPermission {
                    Button("Allow in System Settings…") { shot.requestPermission() }
                        .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.amber)
                    Text("After allowing it, quit and reopen Lantern.").font(.system(size: 11)).foregroundStyle(Theme.muted)
                } else {
                    Button("Try again") { Task { await shot.capture() } }
                        .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.amber)
                }
            }
        }
    }

    /// A new agent picked: load its models and start from the model used with it last time.
    private func kindChanged() {
        guard let k = selectedKind else { return }
        model.loadModels(k)
        chosenModel = model.lastModel(k)
        searchingModels = false
        modelQuery = ""
    }

    private func modelLabel(_ list: EngineMessage.ModelList) -> String {
        guard let chosenModel else { return "Default" }
        return list.models.first { $0.id == chosenModel }.map { m in m.group.map { "\(m.label) · \($0)" } ?? m.label } ?? chosenModel
    }

    /// Type to filter the agent's models, or use exactly what you typed.
    private func modelSearch(_ list: EngineMessage.ModelList) -> some View {
        let query = modelQuery.trimmingCharacters(in: .whitespaces)
        let matches = query.isEmpty ? Array(list.models.prefix(6))
            : Array(list.models.filter { $0.id.localizedCaseInsensitiveContains(query) || $0.label.localizedCaseInsensitiveContains(query) }.prefix(8))
        return VStack(alignment: .leading, spacing: 2) {
            TextField("Model name or id", text: $modelQuery)
                .textFieldStyle(.plain).font(.system(size: 12.5))
                .padding(.horizontal, 10).padding(.vertical, 7)
                .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Theme.faint))
                .onSubmit { if !query.isEmpty { pickModel(matches.first { $0.id == query }?.id ?? query) } }
            ForEach(matches, id: \.id) { m in
                Button { pickModel(m.id) } label: {
                    HStack {
                        Text(m.label).foregroundStyle(Theme.ink.opacity(0.9)).lineLimit(1)
                        Spacer()
                        if let g = m.group { Text(g).foregroundStyle(Theme.muted) }
                    }
                    .font(.system(size: 12)).padding(.horizontal, 10).padding(.vertical, 4).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            if !query.isEmpty && !list.models.contains(where: { $0.id == query }) {
                Button { pickModel(query) } label: {
                    Text("Use “\(query)”").font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.amber)
                        .padding(.horizontal, 10).padding(.vertical, 4)
                }
                .buttonStyle(.plain)
            }
        }
    }

    private func pickModel(_ id: String) {
        chosenModel = id
        searchingModels = false
        modelQuery = ""
    }

    private var selectedKind: AgentKind? { kind ?? model.installedKinds.flatMap { ordered($0).first } }

    /// A Task starts in the open document's folder, or where the last Task for this app ran.
    private var selectedFolder: String? {
        folder ?? (task ? documentFolder ?? model.taskFolder(for: app.target?.bundleId) : nil) ?? model.recentFolders.first
    }

    private var documentFolder: String? { app.target?.document?.deletingLastPathComponent().path }

    /// A task for the app you were in, unless it's a terminal or you closed its card.
    private var task: Bool { app.target != nil && !app.isTerminal && !taskDismissed }

    private func promptField(_ placeholder: String) -> some View {
        TextField(placeholder, text: $prompt, axis: .vertical)
            .textFieldStyle(.plain)
            .font(.system(size: 13))
            .lineLimit(3...6)
            .padding(.horizontal, 12).padding(.vertical, 9)
            .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.faint))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Theme.hairline))
    }

    /// "Work on DaVinci Resolve · video-3": on, the session is a task for the app you were in (with
    /// a screenshot); off, a plain session.
    @ViewBuilder private var workOnApp: some View {
        if let t = app.target {
            let on = !taskDismissed
            Button {
                taskDismissed.toggle()
                screenshotOn = !taskDismissed || prefs.screenshotNewSessions
                if screenshotOn { Task { await shot.capture() } } else { shot.discard() }
            } label: {
                HStack(spacing: 8) {
                    if let icon = t.icon { Image(nsImage: icon).resizable().frame(width: 20, height: 20) }
                    VStack(alignment: .leading, spacing: 0) {
                        Text("Work on \(t.name)").font(.system(size: 12.5, weight: .semibold)).foregroundStyle(on ? Theme.ink : Theme.muted)
                        if let detail = t.document?.lastPathComponent ?? t.windowTitle, !detail.isEmpty {
                            Text(detail).font(.system(size: 10.5)).foregroundStyle(Theme.muted).lineLimit(1).truncationMode(.middle)
                        }
                    }
                    Spacer(minLength: 4)
                    Image(systemName: on ? "checkmark.circle.fill" : "circle")
                        .font(.system(size: 15)).foregroundStyle(on ? Theme.amber : Theme.muted)
                }
                .padding(.horizontal, 10).padding(.vertical, 7)
                .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(on ? Theme.amber.opacity(0.12) : Theme.faint))
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .strokeBorder(on ? Theme.amber.opacity(0.45) : Theme.hairline)
                )
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(on
                ? "The agent is told about \(t.name), gets a screenshot and Lantern's list of tools on this Mac. Click for a plain session."
                : "Click to start this as a task for \(t.name)")
            .accessibilityLabel("Work on \(t.name)")
            .accessibilityAddTraits(on ? .isSelected : [])
            if on, !app.trusted {
                HStack(spacing: 4) {
                    Text("Lantern can't see which document is open.").font(.system(size: 10.5)).foregroundStyle(Theme.muted)
                    Button("Allow…") { app.requestAccess() }
                        .buttonStyle(.plain).font(.system(size: 10.5, weight: .medium)).foregroundStyle(Theme.amber)
                        .help("Allow Accessibility so Lantern can tell the agent which document is open.")
                }
            }
        }
    }

    private var canStart: Bool {
        selectedKind != nil && selectedFolder != nil && !model.launching && !(screenshotOn && shot.capturing)
            && (!task || !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
    }

    private func start() {
        guard let k = selectedKind, let f = selectedFolder else { return }
        failure = nil
        var used = UserDefaults.standard.stringArray(forKey: "agentOrder") ?? []
        used.removeAll { $0 == k.rawValue }
        UserDefaults.standard.set([k.rawValue] + used, forKey: "agentOrder")
        var screenshot: URL?
        if screenshotOn {
            guard let saved = shot.save() else {
                failure = shot.error ?? "The screenshot isn't ready. Retake it, or turn it off."
                return
            }
            screenshot = saved
        }
        // The prompt stays until the session has started (a success closes this panel).
        let chosen = model.models[k.rawValue]?.choosable == true ? chosenModel : nil
        model.launch(
            kind: k, folder: f, prompt: prompt, screenshot: screenshot, model: chosen,
            context: task ? app.target?.summary : nil, toolkit: task || prefs.toolkitInSessions,
            taskApp: task ? (app.target?.bundleId ?? app.target?.name) : nil, background: background
        )
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.level = NSWindow.Level(rawValue: NSWindow.Level.statusBar.rawValue + 1)
        NSApp.activate()
        if panel.runModal() == .OK, let url = panel.url { folder = url.path }
    }

    static func short(_ path: String) -> String {
        let home = NSHomeDirectory()
        return path.hasPrefix(home) ? "~" + path.dropFirst(home.count) : path
    }

}

/// A small rounded option: an icon, and its current value when there's room.
struct Chip: View {
    let symbol: String
    let text: String?
    var active = false
    var maxWidth: CGFloat = 100

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: symbol).font(.system(size: 10.5, weight: .medium))
            if let text {
                Text(text).lineLimit(1).truncationMode(.middle).frame(maxWidth: maxWidth, alignment: .leading).fixedSize(horizontal: true, vertical: false)
            }
        }
        .font(.system(size: 11.5, weight: .medium))
        .foregroundStyle(active ? Theme.amber : Theme.ink.opacity(0.85))
        .padding(.horizontal, 9).padding(.vertical, 5)
        .background(Capsule().fill(Theme.faint))
        .overlay(Capsule().strokeBorder(active ? Theme.amber.opacity(0.6) : Theme.hairline))
    }
}

/// An installed agent in New Session's row: its icon, and its name when it's the one chosen.
struct AgentChip: View {
    let kind: AgentKind
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Group {
                    if let img = Assets.harness(kind) {
                        Image(nsImage: img).resizable().interpolation(.high).aspectRatio(contentMode: .fit)
                            .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
                    } else {
                        Image(systemName: "terminal").font(.system(size: 11))
                    }
                }
                .frame(width: 18, height: 18)
                if selected {
                    Text(kind.displayName).font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.ink).lineLimit(1)
                }
            }
            .padding(.horizontal, selected ? 9 : 6).padding(.vertical, 5)
            .background(Capsule().fill(Theme.faint))
            .overlay(Capsule().strokeBorder(selected ? Theme.amber.opacity(0.7) : Theme.hairline, lineWidth: selected ? 1.5 : 1))
        }
        .buttonStyle(.plain)
        .help(kind.displayName)
        .accessibilityLabel(kind.displayName)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

struct AgentTile: View {
    let kind: AgentKind
    let selected: Bool
    /// Not installed: dimmed, and clicking opens where to get it.
    var installed = true
    let action: () -> Void
    @State private var hover = false

    var body: some View {
        Button(action: action) {
            VStack(spacing: 5) {
                Group {
                    if let img = Assets.harness(kind) {
                        Image(nsImage: img).resizable().interpolation(.high).aspectRatio(contentMode: .fit)
                            .clipShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
                    } else {
                        Image(systemName: "terminal").font(.system(size: 18))
                    }
                }
                .frame(width: 30, height: 30)
                Text(installed ? kind.displayName : "Get \(kind.displayName)").font(.system(size: 10.5, weight: .medium)).lineLimit(1)
                    .foregroundStyle(selected ? Theme.ink : Theme.muted)
            }
            .opacity(installed ? 1 : 0.4)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(selected || hover ? Theme.faint : .clear))
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(selected ? Theme.amber.opacity(0.7) : Theme.hairline, lineWidth: selected ? 1.5 : 1)
            )
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .accessibilityLabel(installed ? kind.displayName : "\(kind.displayName), not installed")
        .help(installed ? kind.displayName : "\(kind.displayName) isn't installed. Click to see how to get it.")
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

struct Choice: View {
    let title: String
    let symbol: String
    let selected: Bool
    var disabled = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: symbol)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(selected ? Theme.ink : Theme.muted)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(selected ? Theme.faint : .clear))
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .strokeBorder(selected ? Theme.amber.opacity(0.7) : Theme.hairline)
                )
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .opacity(disabled ? 0.4 : 1)
    }
}

/// October: the account, October Desktop and the phone app, each as its own card.
struct OctoberView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var account = OctoberAccount.shared

    @ObservedObject private var metrics = PanelMetrics.shared

    var body: some View {
        ScrollView {
            content
        }
        .frame(maxHeight: max(200, metrics.maxHeight - 60))
        .fixedSize(horizontal: false, vertical: true)
    }

    private var content: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                OctoberLogo(size: 40)
                VStack(alignment: .leading, spacing: 2) {
                    Text("October").font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.ink)
                    Text("Lantern works on its own. Connecting to October adds more.")
                        .font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                }
            }
            AccountCard()
            DesktopCard(model: model)
            TeamCard()
            PhoneCard(signedIn: account.signedIn, planAllowsPhone: account.plan?.features?.mobile?.enabled)
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 16)
    }
}

struct OctoberLogo: View {
    var size: CGFloat = 26

    var body: some View {
        Group {
            if let img = Assets.image("october.png") {
                Image(nsImage: img).resizable().interpolation(.high).aspectRatio(contentMode: .fit)
            } else {
                Image(systemName: "leaf.fill").foregroundStyle(.orange)
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: size * 0.24, style: .continuous))
    }
}
