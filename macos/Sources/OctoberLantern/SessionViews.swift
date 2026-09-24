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
    @State private var showOthers = false
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
            ScrollView {
                form.padding(.horizontal, 16).padding(.bottom, 4)
            }
            .frame(maxHeight: max(200, metrics.maxHeight - 150))
            .fixedSize(horizontal: false, vertical: true)
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

    private var form: some View {
        VStack(alignment: .leading, spacing: 14) {
            if task, app.target != nil {
                taskCard
                section("What should it do\(app.target.map { " in \($0.name)" } ?? "")?") { promptField("e.g. Normalize the voice and fix the lighting") }
            }
            section("Agent") {
                // nil while the engine is still looking.
                if let installed = model.installedKinds {
                    if installed.isEmpty {
                        Text("No agents found on this Mac yet. Click one below to get it, then reopen this panel.")
                            .font(.system(size: 12)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                    }
                    // Installed agents; the others folded away, dimmed, each opening where to get it.
                    let others = AgentKind.supported.filter { !installed.contains($0) }
                    LazyVGrid(columns: columns, spacing: 8) {
                        ForEach(installed + (showOthers ? others : []), id: \.self) { k in
                            let has = installed.contains(k)
                            AgentTile(kind: k, selected: has && selectedKind == k, installed: has) {
                                if has { kind = k } else if let url = k.website { NSWorkspace.shared.open(url) }
                            }
                        }
                    }
                    if !others.isEmpty {
                        Button(showOthers ? "Show only installed agents" : "\(others.count) more agents Lantern works with…") {
                            showOthers.toggle()
                        }
                        .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
                    }
                } else {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.small)
                        Text("Looking for installed agents…").font(.system(size: 12)).foregroundStyle(Theme.muted)
                    }
                }
            }

            if let k = selectedKind { section("Model") { modelPicker(k) } }

            section("Folder") {
                Menu {
                    if task, let doc = documentFolder {
                        Button("\(Self.short(doc)) (the document's folder)") { folder = doc }
                        Divider()
                    }
                    ForEach(model.recentFolders, id: \.self) { f in
                        Button(Self.short(f)) { folder = f }
                    }
                    if !model.recentFolders.isEmpty { Divider() }
                    Button("Choose Folder…") { chooseFolder() }
                } label: {
                    HStack {
                        Image(systemName: "folder")
                        Text(selectedFolder.map(Self.short) ?? "Choose a folder…").lineLimit(1).truncationMode(.head)
                        Spacer()
                        Image(systemName: "chevron.up.chevron.down").font(.system(size: 9, weight: .semibold))
                    }
                    .font(.system(size: 12.5))
                    .foregroundStyle(Theme.ink.opacity(0.9))
                    .padding(.horizontal, 12).padding(.vertical, 9)
                    .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.faint))
                    .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Theme.hairline))
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
            }

            if !task {
                section("First message (optional)") { promptField("What should it work on?") }
            }

            section("Context") {
                Toggle(isOn: Binding(
                    get: { screenshotOn },
                    set: { on in
                        screenshotOn = on
                        // A plain session's choice is remembered; a Task always starts with it on.
                        if !task { prefs.screenshotNewSessions = on }
                        if on { Task { await shot.capture() } } else { shot.discard() }
                    }
                )) {
                    Text("Include a screenshot of my screen").font(.system(size: 12.5)).foregroundStyle(Theme.ink.opacity(0.9))
                }
                .toggleStyle(.switch).controlSize(.small)
                if screenshotOn { screenshotPreview }
                if task {
                    Text("The agent is also told about \(app.target?.name ?? "the app") and given Lantern's list of tools on this Mac.")
                        .font(.system(size: 11)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                }
            }

            section("Open in") {
                HStack(spacing: 8) {
                    Choice(title: "Terminal window", symbol: "macwindow", selected: !background) { background = false }
                    Choice(title: "Background", symbol: "moon", selected: background, disabled: !model.tmuxAvailable) { background = true }
                }
                Text(model.tmuxAvailable
                     ? "Lantern can reply directly to sessions it starts. Background sessions open in Terminal when you click Open."
                     : "Install tmux (brew install tmux) so Lantern can reply directly and run sessions in the background.")
                    .font(.system(size: 11)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
            }
        }
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

    @ViewBuilder
    private func modelPicker(_ k: AgentKind) -> some View {
        if let list = model.models[k.rawValue] {
            if !list.choosable {
                Text("\(k.displayName) starts with its own default model.").font(.system(size: 11.5)).foregroundStyle(Theme.muted)
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    Menu {
                        Button("Default") { chosenModel = nil; searchingModels = false }
                        if !list.models.isEmpty && list.models.count <= 25 {
                            Divider()
                            ForEach(list.models, id: \.id) { m in Button(m.label) { chosenModel = m.id; searchingModels = false } }
                        }
                        Divider()
                        Button(list.models.count > 25 ? "Search \(list.models.count) models…" : "Other model…") { searchingModels = true }
                    } label: {
                        fieldLabel(symbol: "cpu", text: modelLabel(list))
                    }
                    .menuStyle(.borderlessButton)
                    .menuIndicator(.hidden)
                    if searchingModels { modelSearch(list) }
                }
            }
        } else {
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Loading \(k.displayName)'s models…").font(.system(size: 11.5)).foregroundStyle(Theme.muted)
            }
        }
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

    private func fieldLabel(symbol: String, text: String) -> some View {
        HStack {
            Image(systemName: symbol)
            Text(text).lineLimit(1).truncationMode(.middle)
            Spacer()
            Image(systemName: "chevron.up.chevron.down").font(.system(size: 9, weight: .semibold))
        }
        .font(.system(size: 12.5))
        .foregroundStyle(Theme.ink.opacity(0.9))
        .padding(.horizontal, 12).padding(.vertical, 9)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Theme.hairline))
    }

    private var selectedKind: AgentKind? { kind ?? model.installedKinds?.first }

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
            .lineLimit(task ? 3...6 : 2...5)
            .padding(.horizontal, 12).padding(.vertical, 9)
            .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.faint))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Theme.hairline))
    }

    /// The app the Task is for: its icon, name, window and document.
    @ViewBuilder private var taskCard: some View {
        if let t = app.target {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 10) {
                    if let icon = t.icon { Image(nsImage: icon).resizable().frame(width: 30, height: 30) }
                    VStack(alignment: .leading, spacing: 1) {
                        Text(t.name).font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                        if let detail = t.document?.lastPathComponent ?? t.windowTitle, !detail.isEmpty {
                            Text(detail).font(.system(size: 11)).foregroundStyle(Theme.muted).lineLimit(1).truncationMode(.middle)
                        }
                    }
                    Spacer()
                    Button {
                        taskDismissed = true
                        screenshotOn = prefs.screenshotNewSessions
                        if !screenshotOn { shot.discard() }
                    } label: {
                        Image(systemName: "xmark").font(.system(size: 10, weight: .semibold)).foregroundStyle(Theme.muted)
                    }
                    .buttonStyle(.plain)
                    .help("Start a plain session instead")
                    .accessibilityLabel("Not for \(t.name)")
                }
                if !app.trusted {
                    HStack(spacing: 8) {
                        Text("Allow Accessibility so Lantern can tell the agent which document is open.")
                            .font(.system(size: 11)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                        Button("Allow…") { app.requestAccess() }
                            .buttonStyle(.plain).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.amber)
                    }
                }
            }
            .padding(10)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Theme.faint))
            .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Theme.hairline))
        }
    }
    /// A screenshot that's on must be ready (or failed, which the form shows) before starting.
    private var canStart: Bool {
        selectedKind != nil && selectedFolder != nil && !model.launching && !(screenshotOn && shot.capturing)
            && (!task || !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
    }

    private func start() {
        guard let k = selectedKind, let f = selectedFolder else { return }
        failure = nil
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

    @ViewBuilder
    private func section<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
            content()
        }
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
