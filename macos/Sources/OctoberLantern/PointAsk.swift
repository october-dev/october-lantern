import AppKit
import ApplicationServices
import Combine
import ScreenCaptureKit
import SwiftUI

/// Point & Ask: press the shortcut (⌃⌥P by default) or the pill's selection button, drag a box
/// around what you mean (or click a window), then say or type your question. A card beside the
/// selection shows the screenshot, the app and window, the page's address in a browser and any
/// selected text.
///
/// - **Ask** answers right in the card with one call to October's AI (needs October sign-in).
/// - **Send to agent** hands it to a coding agent instead, like a reply.
///
/// Holding the shortcut listens while you select; a quick tap keeps listening until you press it
/// again or send. Screenshots sent to an agent are saved inside its project, in `.lantern/shots/`,
/// which Lantern adds to the repository's local exclude list (never `.gitignore`).
@MainActor
final class PointAsk: ObservableObject {
    static let shared = PointAsk()
    weak var model: AppModel?

    struct Context {
        var app: String?
        var bundleId: String?
        var window: String?
        var document: URL?
        var url: String?
        var selection: String?

        /// The context as lines of text, for the model or an agent.
        var lines: [String] {
            var out: [String] = []
            if let app { out.append("App: \(app)" + (window.map { ", window \"\($0)\"" } ?? "")) }
            if let document { out.append("Document: \(document.path)") }
            if let url { out.append("Page: \(url)") }
            if let selection, !selection.isEmpty { out.append("Selected text:\n\(String(selection.prefix(2000)))") }
            return out
        }
    }

    struct Turn: Identifiable {
        let id = UUID()
        let question: String
        var answer: String
        var failed = false
    }

    @Published var text = ""
    @Published var recipientId: String?
    @Published private(set) var image: NSImage?
    @Published private(set) var context = Context()
    @Published private(set) var problem: String?
    @Published private(set) var turns: [Turn] = []
    @Published private(set) var asking = false
    let dictation = Dictation()

    private let overlay = SelectionOverlay()
    private var panel: FloatingPanel?
    private var host: NSHostingView<PointAskCard>?
    private var pressedAt: Date?
    private var shot: CGImage?
    private var selection: (rect: NSRect, screen: NSScreen)?
    private var askTask: Task<Void, Never>?
    private var resize: AnyCancellable?

    private init() {
        dictation.onText = { [weak self] text, _ in self?.text = text }
        dictation.onError = { [weak self] message in self?.problem = message }
    }

    var isOpen: Bool { (panel?.isVisible ?? false) || overlay.isOpen }
    var signedIn: Bool { OctoberAccount.shared.signedIn }

    // MARK: Starting

    /// The shortcut went down.
    func pressed() {
        if overlay.isOpen { return }
        if panel?.isVisible ?? false {
            // Pressed again: stop or resume listening.
            if dictation.isActive { dictation.stop() } else { dictation.start(prefix: text, owner: "point-ask") }
            return
        }
        pressedAt = Date()
        start(listen: true)
    }

    /// Let go after holding: stop listening and wait for Enter. A quick tap keeps listening.
    func released() {
        guard let pressedAt, Date().timeIntervalSince(pressedAt) > 0.35 else { return }
        dictation.stop()
    }

    /// From the pill's button: close Lantern's panel so it's out of the way, then select.
    func start(fromPill: Bool) {
        if overlay.isOpen { return }
        pressedAt = nil
        model?.panel = nil
        // Let the panel fade before the screen dims.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { self.start(listen: false) }
    }

    private func start(listen: Bool) {
        askTask?.cancel()
        panel?.orderOut(nil)
        text = ""
        problem = nil
        image = nil
        shot = nil
        turns = []
        asking = false
        // Read the app you're in before anything of Lantern's takes the keyboard.
        AppContext.shared.capture()
        let t = AppContext.shared.target
        context = Context(app: t?.name, bundleId: t?.bundleId, window: t?.windowTitle, document: t?.document)
        recipientId = defaultRecipient()?.id
        Task { await readPageAndSelection(pid: t?.pid, bundleId: t?.bundleId) }
        if listen { dictation.start(prefix: "", owner: "point-ask") }
        select()
    }

    /// Shows the selection overlay; the card appears once an area is chosen.
    func select() {
        panel?.orderOut(nil)
        overlay.begin { [weak self] chosen in
            guard let self else { return }
            guard let chosen else {
                // Cancelled before choosing anything: nothing to ask about.
                if self.shot == nil { self.cancel() } else { self.panel?.orderFront(nil) }
                return
            }
            self.selection = (chosen.0, chosen.1)
            self.image = nil
            self.problem = nil
            self.showCard()
            Task { await self.capture(chosen.0, on: chosen.1) }
        }
    }

    func cancel() {
        askTask?.cancel()
        overlay.cancel()
        dictation.stop()
        panel?.orderOut(nil)
    }

    // MARK: Ask

    /// The action for Enter: Ask when signed in to October, otherwise Send to the agent.
    func primary() {
        if signedIn || recipient == nil { ask() } else { send() }
    }

    func ask() {
        guard signedIn else {
            cancel()
            model?.panel = .october
            return
        }
        let words = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !asking, !words.isEmpty || turns.isEmpty else { return }
        dictation.stop()
        let question = words.isEmpty ? "What is this?" : words
        text = ""
        turns.append(Turn(question: question, answer: ""))
        asking = true
        let index = turns.count - 1
        let request = OctoberAI.Request(
            turns: turns.dropLast().map { ($0.question, $0.answer) }, question: question,
            image: shot.flatMap(Self.jpeg), context: context.lines.joined(separator: "\n")
        )
        askTask = Task {
            do {
                for try await piece in OctoberAI.stream(request) {
                    guard index < turns.count else { return }
                    turns[index].answer += piece
                }
                if turns.indices.contains(index), turns[index].answer.isEmpty {
                    turns[index].answer = "No answer came back. Try again."
                    turns[index].failed = true
                }
            } catch is CancellationError {
            } catch {
                if turns.indices.contains(index) {
                    turns[index].answer = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
                    turns[index].failed = true
                }
            }
            asking = false
            Analytics.shared.capture("point_ask_asked", ["follow_up": index > 0, "had_url": context.url != nil])
        }
    }

    func copyAnswer() {
        guard let last = turns.last(where: { !$0.failed && !$0.answer.isEmpty }) else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(last.answer, forType: .string)
    }

    // MARK: Send to an agent

    var recipient: Agent? { model?.agents.first { $0.id == recipientId } }

    func send() {
        guard let model, let agent = recipient else { return }
        var words = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if words.isEmpty, let last = turns.last { words = last.question }
        guard !words.isEmpty || shot != nil else { return }
        dictation.stop()
        var lines = [words.isEmpty ? "Take a look at what I've selected on my screen." : words]
        if let path = saveShot(for: agent) {
            lines.append("\nScreenshot of the part of my screen I selected: \(path)")
        }
        let where_ = context.lines
        if !where_.isEmpty { lines.append(where_.joined(separator: "\n")) }
        if let last = turns.last(where: { !$0.failed && !$0.answer.isEmpty }) {
            lines.append("October's quick answer, for reference:\n\(String(last.answer.prefix(3000)))")
        }
        model.sendDirect(lines.joined(separator: "\n"), to: agent)
        Analytics.shared.capture(
            "point_ask_sent",
            ["kind": agent.kind.rawValue, "route": agent.route?.via ?? "none", "had_url": context.url != nil,
             "had_selection": !(context.selection ?? "").isEmpty, "after_ask": !turns.isEmpty]
        )
        cancel()
    }

    /// Saves the screenshot in the agent's project (`.lantern/shots/`, excluded from git), or in
    /// Lantern's screenshots folder when the agent has no folder.
    private func saveShot(for agent: Agent) -> String? {
        guard let shot, let png = NSBitmapImageRep(cgImage: shot).representation(using: .png, properties: [:]) else { return nil }
        let fm = FileManager.default
        var dir = ScreenCapture.folder
        if let cwd = agent.cwd, fm.fileExists(atPath: cwd) {
            let root = URL(fileURLWithPath: cwd)
            dir = root.appendingPathComponent(".lantern/shots", isDirectory: true)
            Self.excludeFromGit(root)
        }
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        let stamp = DateFormatter()
        stamp.dateFormat = "yyyyMMdd-HHmmss"
        let url = dir.appendingPathComponent("point-\(stamp.string(from: Date())).png")
        return fm.createFile(atPath: url.path, contents: png) ? url.path : nil
    }

    /// Adds `.lantern/` to the repository's local exclude file (`.git/info/exclude`), once.
    static func excludeFromGit(_ root: URL) {
        let git = root.appendingPathComponent(".git")
        var isDir: ObjCBool = false
        guard FileManager.default.fileExists(atPath: git.path, isDirectory: &isDir), isDir.boolValue else { return }
        let exclude = git.appendingPathComponent("info/exclude")
        let current = (try? String(contentsOf: exclude, encoding: .utf8)) ?? ""
        guard !current.split(separator: "\n").contains(where: { $0.trimmingCharacters(in: .whitespaces) == ".lantern/" }) else { return }
        try? FileManager.default.createDirectory(at: exclude.deletingLastPathComponent(), withIntermediateDirectories: true)
        let line = (current.isEmpty || current.hasSuffix("\n") ? "" : "\n") + "# October Lantern screenshots\n.lantern/\n"
        if let handle = try? FileHandle(forWritingTo: exclude) {
            handle.seekToEndOfFile()
            handle.write(Data(line.utf8))
            try? handle.close()
        } else {
            try? line.write(to: exclude, atomically: true, encoding: .utf8)
        }
    }

    // MARK: Who it goes to

    /// The agent working on what's in front: the open document's project, a project named in the
    /// window title, an agent in the terminal in front; otherwise the composer's agent.
    private func defaultRecipient() -> Agent? {
        guard let model else { return nil }
        let live = model.agents.filter(\.isLive)
        if let doc = context.document?.path,
           let a = live.filter({ $0.cwd.map { doc.hasPrefix($0 + "/") } ?? false }).max(by: { ($0.cwd?.count ?? 0) < ($1.cwd?.count ?? 0) }) {
            return a
        }
        if let title = context.window,
           let a = live.first(where: { $0.project.map { !$0.isEmpty && title.localizedCaseInsensitiveContains($0) } ?? false }) {
            return a
        }
        if let pid = AppContext.shared.target?.pid, let a = live.first(where: { $0.host?.pid == pid }) {
            return a
        }
        return model.target ?? live.first
    }

    // MARK: Context

    /// A screenshot of the selected area (global AppKit coordinates), Lantern's windows left out.
    private func capture(_ rect: NSRect, on screen: NSScreen) async {
        guard CGPreflightScreenCaptureAccess() else {
            CGRequestScreenCaptureAccess()
            problem = "Allow Screen Recording for October Lantern in System Settings to include a screenshot, then reopen Lantern."
            return
        }
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            guard let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID,
                  let display = content.displays.first(where: { $0.displayID == number }) else { return }
            let mine = content.applications.filter { $0.processID == ProcessInfo.processInfo.processIdentifier }
            let filter = SCContentFilter(display: display, excludingApplications: mine, exceptingWindows: [])
            // Display-local, top-left origin.
            let local = CGRect(x: rect.minX - screen.frame.minX, y: screen.frame.maxY - rect.maxY, width: rect.width, height: rect.height)
            let config = SCStreamConfiguration()
            config.sourceRect = local
            config.width = Int(local.width * screen.backingScaleFactor)
            config.height = Int(local.height * screen.backingScaleFactor)
            config.showsCursor = false
            let raw = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
            shot = raw
            image = NSImage(cgImage: raw, size: .zero)
        } catch {
            problem = "Couldn't take the screenshot: \(error.localizedDescription)"
        }
    }

    /// The screenshot as a JPEG for the model, at most 1600 pixels on its longer side.
    private static func jpeg(_ image: CGImage) -> Data? {
        let longest = CGFloat(max(image.width, image.height))
        let scale = min(1, 1600 / longest)
        let w = Int(CGFloat(image.width) * scale), h = Int(CGFloat(image.height) * scale)
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { return nil }
        ctx.interpolationQuality = .high
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
        guard let scaled = ctx.makeImage() else { return nil }
        return NSBitmapImageRep(cgImage: scaled).representation(using: .jpeg, properties: [.compressionFactor: 0.82])
    }

    /// The page's address in a browser (AppleScript, asked once per browser) and the selected text
    /// (Accessibility, when allowed).
    private func readPageAndSelection(pid: pid_t?, bundleId: String?) async {
        if AXIsProcessTrusted() {
            let system = AXUIElementCreateSystemWide()
            var focused: CFTypeRef?
            if AXUIElementCopyAttributeValue(system, kAXFocusedUIElementAttribute as CFString, &focused) == .success,
               let element = focused, CFGetTypeID(element) == AXUIElementGetTypeID() {
                var selected: CFTypeRef?
                if AXUIElementCopyAttributeValue(element as! AXUIElement, kAXSelectedTextAttribute as CFString, &selected) == .success {
                    context.selection = (selected as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
                }
            }
        }
        if let bundleId, let script = Self.urlScript(bundleId) {
            let url = await Task.detached { () -> String? in
                let p = Process()
                p.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
                p.arguments = ["-e", script]
                let out = Pipe()
                p.standardOutput = out
                p.standardError = Pipe()
                guard (try? p.run()) != nil else { return nil }
                let deadline = Date().addingTimeInterval(3)
                while p.isRunning && Date() < deadline { usleep(20_000) }
                if p.isRunning { p.terminate(); return nil }
                let s = String(data: out.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                return s?.isEmpty == false ? s : nil
            }.value
            context.url = url
        }
    }

    private static func urlScript(_ bundleId: String) -> String? {
        switch bundleId {
        case "com.apple.Safari": "tell application id \"com.apple.Safari\" to return URL of current tab of front window"
        case "com.google.Chrome", "com.brave.Browser", "company.thebrowser.Browser", "com.microsoft.edgemac":
            "tell application id \"\(bundleId)\" to return URL of active tab of front window"
        default: nil
        }
    }

    /// Fills the card with sample content, for `--snapshot`.
    func demo(answered: Bool) {
        image = Assets.image("october/hero-palace.jpg") ?? Assets.logo
        context = Context(app: "Safari", window: "Checkout · Stripe", url: "https://dashboard.stripe.com/test/payments",
                          selection: nil)
        turns = answered ? [Turn(question: "Why is this payment failing?", answer: "The card was **declined by the issuer** (`card_declined`). In test mode that's the `4000 0000 0000 0002` card.\n\n- Use `4242 4242 4242 4242` to see a success\n- Or handle `card_declined` in your checkout and show the message to the customer")] : []
        text = answered ? "" : "Why is this payment failing?"
    }

    // MARK: The card

    static let cardWidth: CGFloat = 400

    private func showCard() {
        let panel = self.panel ?? FloatingPanel(keyable: true)
        if self.panel == nil {
            let host = NSHostingView(rootView: PointAskCard(point: self))
            host.sizingOptions = []
            panel.contentView = GlassContainer(content: host, cornerRadius: 20, tint: 0.62)
            self.host = host
            self.panel = panel
            // Grow and shrink with the answer, keeping the top edge where it is.
            resize = objectWillChange
                .merge(with: dictation.objectWillChange)
                .receive(on: RunLoop.main)
                .sink { [weak self] in self?.fit(keepTop: true) }
        }
        fit(keepTop: false)
        panel.makeKeyAndOrderFront(nil)
    }

    /// Sizes the card to its content and places it beside the selection, on screen.
    private func fit(keepTop: Bool) {
        guard let panel, let host, let selection else { return }
        let height = min(host.fittingSize.height, (selection.screen.visibleFrame.height) - 16)
        let size = NSSize(width: Self.cardWidth, height: max(height, 120))
        if keepTop, panel.isVisible {
            guard abs(panel.frame.height - size.height) > 0.5 else { return }
            let top = panel.frame.maxY
            let visible = selection.screen.visibleFrame
            let y = max(top - size.height, visible.minY + 8)
            panel.setFrame(NSRect(x: panel.frame.minX, y: y, width: size.width, height: size.height), display: true)
            return
        }
        let visible = selection.screen.visibleFrame
        let r = selection.rect
        // To the right of the selection, else the left, else inside its right edge.
        var x = r.maxX + 14
        if x + size.width > visible.maxX - 8 { x = r.minX - size.width - 14 }
        if x < visible.minX + 8 { x = min(r.maxX - size.width - 14, visible.maxX - size.width - 8) }
        x = max(x, visible.minX + 8)
        let y = min(max(r.maxY - size.height, visible.minY + 8), visible.maxY - size.height - 8)
        panel.setFrame(NSRect(x: x, y: y, width: size.width, height: size.height), display: true)
    }
}

/// The card beside the selection: the screenshot, where it's from, the conversation, and the two
/// ways to send.
struct PointAskCard: View {
    @ObservedObject var point: PointAsk
    @ObservedObject var dictation: Dictation
    @ObservedObject var account = OctoberAccount.shared
    @FocusState private var focused: Bool

    init(point: PointAsk) {
        self.point = point
        self.dictation = point.dictation
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header
            screenshot
            contextLines
            if !point.turns.isEmpty { conversation }
            input
            footer
        }
        .padding(16)
        .frame(width: PointAsk.cardWidth, alignment: .topLeading)
        .fixedSize(horizontal: false, vertical: true)
        // Nearly opaque, so it reads the same over any window.
        .background(RoundedRectangle(cornerRadius: 20, style: .continuous).fill(Color(white: 0.08).opacity(0.88)))
        .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay(GlassRim(shape: RoundedRectangle(cornerRadius: 20, style: .continuous)))
        .environment(\.colorScheme, .dark)
        .onExitCommand { point.cancel() }
        .onAppear { focused = true }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Image(systemName: "cursorarrow.and.square.on.square.dashed")
                .font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.amber)
            Text("Point & Ask").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
            Spacer()
            if dictation.isRecording {
                Label("Listening", systemImage: "waveform").font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.red)
                    .symbolEffect(.variableColor.iterative, isActive: true)
            }
            Button { point.cancel() } label: {
                Image(systemName: "xmark").font(.system(size: 10, weight: .bold)).foregroundStyle(Theme.muted)
                    .frame(width: 22, height: 22).background(Circle().fill(Theme.faint))
            }
            .buttonStyle(.plain).help("Close (Esc)").accessibilityLabel("Close")
        }
    }

    @ViewBuilder private var screenshot: some View {
        ZStack(alignment: .topTrailing) {
            Group {
                if let image = point.image {
                    Image(nsImage: image).resizable().aspectRatio(contentMode: .fit)
                        .frame(maxWidth: .infinity, maxHeight: point.turns.isEmpty ? 190 : 110)
                } else if let problem = point.problem {
                    Text(problem).font(.system(size: 11.5)).foregroundStyle(Theme.muted)
                        .fixedSize(horizontal: false, vertical: true).padding(12)
                        .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.small)
                        Text("Capturing…").font(.system(size: 11.5)).foregroundStyle(Theme.muted)
                    }
                    .frame(maxWidth: .infinity, minHeight: 80)
                }
            }
            .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Color.black.opacity(0.35)))
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Theme.stroke))

            Button { point.select() } label: {
                Label("Reselect", systemImage: "selection.pin.in.out").font(.system(size: 10.5, weight: .semibold))
                    .foregroundStyle(Theme.ink)
                    .padding(.horizontal, 8).padding(.vertical, 4)
                    .background(Capsule().fill(Color.black.opacity(0.7)))
                    .overlay(Capsule().strokeBorder(Theme.stroke))
            }
            .buttonStyle(.plain).padding(6).help("Select a different area")
        }
    }

    @ViewBuilder private var contextLines: some View {
        let c = point.context
        if c.app != nil || c.url != nil || !(c.selection ?? "").isEmpty {
            VStack(alignment: .leading, spacing: 3) {
                if let app = c.app {
                    Label([app, c.window].compactMap { $0 }.joined(separator: " · "), systemImage: "macwindow").lineLimit(1)
                }
                if let url = c.url { Label(url, systemImage: "link").lineLimit(1).truncationMode(.middle) }
                if let sel = c.selection, !sel.isEmpty { Label("“\(sel)”", systemImage: "text.cursor").lineLimit(2) }
            }
            .font(.system(size: 11)).foregroundStyle(Theme.muted)
        }
    }

    private var conversation: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(point.turns) { turn in
                        VStack(alignment: .leading, spacing: 6) {
                            Text(turn.question).font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.ink)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            if turn.answer.isEmpty {
                                HStack(spacing: 6) {
                                    ProgressView().controlSize(.small)
                                    Text("Thinking…").font(.system(size: 12)).foregroundStyle(Theme.muted)
                                }
                            } else {
                                Text(Self.markdown(turn.answer)).font(.system(size: 12.5)).lineSpacing(2)
                                    .foregroundStyle(turn.failed ? Theme.red : Theme.ink.opacity(0.92))
                                    .textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                        .id(turn.id)
                    }
                }
                .padding(12)
            }
            .frame(maxHeight: 320)
            .fixedSize(horizontal: false, vertical: true)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Theme.faint))
            .onChange(of: point.turns.last?.answer) { _, _ in
                if let last = point.turns.last { proxy.scrollTo(last.id, anchor: .bottom) }
            }
        }
    }

    private var input: some View {
        HStack(alignment: .bottom, spacing: 6) {
            TextField(placeholder, text: $point.text, axis: .vertical)
                .textFieldStyle(.plain).font(.system(size: 13)).lineLimit(1...5)
                .focused($focused)
                .onSubmit { point.primary() }
            Button {
                if dictation.isActive { dictation.stop() } else { dictation.start(prefix: point.text, owner: "point-ask") }
            } label: {
                Image(systemName: dictation.isRecording ? "mic.fill" : "mic").font(.system(size: 12, weight: .medium))
                    .foregroundStyle(dictation.isRecording ? Theme.red : Theme.muted)
                    .frame(width: 22, height: 22)
            }
            .buttonStyle(.plain).help(dictation.isRecording ? "Stop listening" : "Speak")
        }
        .padding(.leading, 11).padding(.trailing, 6).padding(.vertical, 7)
        .background(RoundedRectangle(cornerRadius: 11, style: .continuous).fill(Color.black.opacity(0.3)))
        .overlay(RoundedRectangle(cornerRadius: 11, style: .continuous).strokeBorder(focused ? Theme.amber.opacity(0.5) : Theme.stroke))
    }

    private var placeholder: String {
        if dictation.isRecording { return "Listening… say what you want" }
        return point.turns.isEmpty ? "Ask about this, or tell an agent what to do" : "Ask a follow-up"
    }

    private var footer: some View {
        HStack(spacing: 8) {
            agentMenu
            if !point.turns.isEmpty, !point.asking {
                Button { point.copyAnswer() } label: { Image(systemName: "doc.on.doc").font(.system(size: 11)) }
                    .buttonStyle(.plain).foregroundStyle(Theme.muted).help("Copy the answer")
            }
            Spacer()
            if account.signedIn {
                primaryButton(point.asking ? "Asking…" : "Ask ⏎", enabled: !point.asking) { point.ask() }
            } else {
                Button { point.ask() } label: {
                    Text("Sign in to Ask").font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.amber)
                        .padding(.horizontal, 12).padding(.vertical, 5)
                        .overlay(Capsule().strokeBorder(Theme.amber.opacity(0.6)))
                }
                .buttonStyle(.plain)
                .help("Ask answers right here with October's AI. Sign in to October to use it.")
            }
        }
    }

    /// "Send to @agent", with a menu to pick the agent.
    @ViewBuilder private var agentMenu: some View {
        let live = point.model?.agents.filter(\.isLive) ?? []
        if point.recipient == nil {
            Text("No agent running").font(.system(size: 11.5)).foregroundStyle(Theme.muted)
        } else {
            agentButton(live)
        }
    }

    private func agentButton(_ live: [Agent]) -> some View {
        HStack(spacing: 0) {
            Button { point.send() } label: {
                HStack(spacing: 5) {
                    Image(systemName: "paperplane").font(.system(size: 10.5, weight: .semibold))
                    Text(point.recipient.map { "Send to @\($0.handle)" } ?? "No agent running").lineLimit(1)
                    if !account.signedIn, point.recipient != nil { Text("⏎").foregroundStyle(Theme.muted) }
                }
                .font(.system(size: 11.5, weight: .semibold))
                .foregroundStyle(point.recipient == nil ? Theme.muted : Theme.ink)
                .padding(.leading, 10).padding(.trailing, 6).padding(.vertical, 5)
            }
            .buttonStyle(.plain).disabled(point.recipient == nil)
            .help("Send the screenshot and your words to this agent instead")
            if live.count > 1 {
                Menu {
                    ForEach(live) { a in
                        Button("@\(a.handle) · \(a.project ?? a.kind.displayName)") { point.recipientId = a.id }
                    }
                } label: {
                    Image(systemName: "chevron.down").font(.system(size: 9, weight: .bold)).foregroundStyle(Theme.muted)
                }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().padding(.trailing, 8)
            }
        }
        .background(Capsule().fill(Theme.faint))
        .overlay(Capsule().strokeBorder(Theme.stroke))
    }

    private func primaryButton(_ title: String, enabled: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title).font(.system(size: 12, weight: .semibold)).foregroundStyle(.black.opacity(0.85))
                .padding(.horizontal, 14).padding(.vertical, 6)
                .background(Capsule().fill(enabled ? Theme.amber : Theme.amber.opacity(0.4)))
        }
        .buttonStyle(.plain).disabled(!enabled)
    }

    private static func markdown(_ s: String) -> AttributedString {
        (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(s)
    }
}
