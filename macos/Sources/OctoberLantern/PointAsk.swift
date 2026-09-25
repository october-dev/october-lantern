import AppKit
import ApplicationServices
import ScreenCaptureKit
import SwiftUI

/// Point & Ask: hold the shortcut (⌃⌥P by default), point at something, say what you want, let go.
/// A card by the pointer shows a screenshot of the area around the pointer (the pointer marked),
/// the app and window, the page's address in a browser and any selected text, and the agent it
/// goes to. Enter sends, Esc cancels. A quick tap instead of a hold keeps listening until you
/// press the shortcut again or Enter.
///
/// The screenshot is saved inside the agent's project, in `.lantern/shots/`, which Lantern adds
/// to the repository's local exclude list (never `.gitignore`), so it's never committed.
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
    }

    @Published var text = ""
    @Published var recipientId: String?
    @Published private(set) var image: NSImage?
    @Published private(set) var context = Context()
    @Published private(set) var problem: String?
    let dictation = Dictation()

    private var panel: FloatingPanel?
    private var pressedAt: Date?
    private var shot: CGImage?

    private init() {
        dictation.onText = { [weak self] text, _ in self?.text = text }
        dictation.onError = { [weak self] message in self?.problem = message }
    }

    var isOpen: Bool { panel?.isVisible ?? false }

    // MARK: The shortcut

    func pressed() {
        if isOpen {
            // Pressed again: stop or resume listening.
            if dictation.isActive { dictation.stop() } else { dictation.start(prefix: text, owner: "point-ask") }
            return
        }
        pressedAt = Date()
        text = ""
        problem = nil
        image = nil
        shot = nil
        let mouse = NSEvent.mouseLocation
        AppContext.shared.capture()
        let t = AppContext.shared.target
        context = Context(app: t?.name, bundleId: t?.bundleId, window: t?.windowTitle, document: t?.document)
        recipientId = defaultRecipient()?.id
        show(near: mouse)
        dictation.start(prefix: "", owner: "point-ask")
        Task { await capture(around: mouse) }
        Task { await readPageAndSelection(pid: t?.pid, bundleId: t?.bundleId) }
    }

    /// Let go after holding: stop listening and wait for Enter. A quick tap keeps listening.
    func released() {
        guard let pressedAt, Date().timeIntervalSince(pressedAt) > 0.35 else { return }
        dictation.stop()
    }

    func cancel() {
        dictation.stop()
        panel?.orderOut(nil)
    }

    // MARK: Sending

    var recipient: Agent? { model?.agents.first { $0.id == recipientId } }

    func send() {
        guard let model, let agent = recipient else { return }
        let words = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !words.isEmpty || shot != nil else { return }
        dictation.stop()
        var lines = [words.isEmpty ? "Take a look at what I'm pointing at." : words]
        if let path = saveShot(for: agent) {
            lines.append("\nScreenshot around where I'm pointing (the ring marks the pointer): \(path)")
        }
        var where_: [String] = []
        if let app = context.app { where_.append("App: \(app)" + (context.window.map { ", window \"\($0)\"" } ?? "")) }
        if let doc = context.document { where_.append("Document: \(doc.path)") }
        if let url = context.url { where_.append("Page: \(url)") }
        if !where_.isEmpty { lines.append(where_.joined(separator: "\n")) }
        if let sel = context.selection, !sel.isEmpty {
            lines.append("Selected text:\n\(String(sel.prefix(2000)))")
        }
        model.sendDirect(lines.joined(separator: "\n"), to: agent)
        Analytics.shared.capture(
            "point_ask_sent",
            ["kind": agent.kind.rawValue, "route": agent.route?.via ?? "none", "had_url": context.url != nil,
             "had_selection": !(context.selection ?? "").isEmpty]
        )
        panel?.orderOut(nil)
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

    /// A screenshot of about 900×600 points around the pointer, Lantern's windows left out, with
    /// a ring where the pointer was.
    private func capture(around mouse: NSPoint) async {
        guard CGPreflightScreenCaptureAccess() else {
            problem = "Allow Screen Recording to include a screenshot (then reopen Lantern)."
            return
        }
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            guard let screen = NSScreen.screens.first(where: { NSMouseInRect(mouse, $0.frame, false) }) ?? NSScreen.main,
                  let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID,
                  let display = content.displays.first(where: { $0.displayID == number }) else { return }
            let mine = content.applications.filter { $0.processID == ProcessInfo.processInfo.processIdentifier }
            let filter = SCContentFilter(display: display, excludingApplications: mine, exceptingWindows: [])
            // Display-local, top-left origin.
            let local = CGPoint(x: mouse.x - screen.frame.minX, y: screen.frame.maxY - mouse.y)
            let size = CGSize(width: min(900, screen.frame.width), height: min(600, screen.frame.height))
            let origin = CGPoint(
                x: min(max(local.x - size.width / 2, 0), screen.frame.width - size.width),
                y: min(max(local.y - size.height / 2, 0), screen.frame.height - size.height)
            )
            let config = SCStreamConfiguration()
            config.sourceRect = CGRect(origin: origin, size: size)
            config.width = Int(size.width * screen.backingScaleFactor)
            config.height = Int(size.height * screen.backingScaleFactor)
            config.showsCursor = false
            let raw = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
            let pointer = CGPoint(x: (local.x - origin.x) / size.width, y: (local.y - origin.y) / size.height)
            shot = Self.marked(raw, at: pointer) ?? raw
            if let shot { image = NSImage(cgImage: shot, size: .zero) }
        } catch {
            problem = "Couldn't take the screenshot: \(error.localizedDescription)"
        }
    }

    /// Draws a ring at `at` (fractions of the image, top-left origin).
    private static func marked(_ image: CGImage, at p: CGPoint) -> CGImage? {
        let w = image.width, h = image.height
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
        let r = CGFloat(min(w, h)) * 0.035
        let c = CGPoint(x: p.x * CGFloat(w), y: (1 - p.y) * CGFloat(h))
        ctx.setStrokeColor(NSColor.systemOrange.cgColor)
        ctx.setLineWidth(max(3, r * 0.22))
        ctx.strokeEllipse(in: CGRect(x: c.x - r, y: c.y - r, width: r * 2, height: r * 2))
        ctx.setFillColor(NSColor.systemOrange.withAlphaComponent(0.25).cgColor)
        ctx.fillEllipse(in: CGRect(x: c.x - r, y: c.y - r, width: r * 2, height: r * 2))
        return ctx.makeImage()
    }

    /// The page's address in a browser (AppleScript, asked once per browser) and the selected text
    /// (Accessibility, when allowed).
    private func readPageAndSelection(pid: pid_t?, bundleId: String?) async {
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
    }

    private static func urlScript(_ bundleId: String) -> String? {
        switch bundleId {
        case "com.apple.Safari": "tell application id \"com.apple.Safari\" to return URL of current tab of front window"
        case "com.google.Chrome", "com.brave.Browser", "company.thebrowser.Browser", "com.microsoft.edgemac":
            "tell application id \"\(bundleId)\" to return URL of active tab of front window"
        default: nil
        }
    }

    // MARK: The card

    private func show(near mouse: NSPoint) {
        let panel = self.panel ?? FloatingPanel(keyable: true)
        self.panel = panel
        let host = NSHostingView(rootView: PointAskCard(point: self))
        panel.contentView = host
        let size = NSSize(width: 360, height: max(host.fittingSize.height, 300))
        let screen = NSScreen.screens.first { NSMouseInRect(mouse, $0.frame, false) } ?? NSScreen.main
        let visible = screen?.visibleFrame ?? .zero
        // Beside the pointer, kept on screen.
        var x = mouse.x + 24
        if x + size.width > visible.maxX { x = mouse.x - size.width - 24 }
        let y = min(max(mouse.y - size.height / 2, visible.minY + 8), visible.maxY - size.height - 8)
        panel.setFrame(NSRect(x: max(x, visible.minX + 8), y: y, width: size.width, height: size.height), display: true)
        panel.makeKeyAndOrderFront(nil)
    }
}

/// The card by the pointer: what you're pointing at, what you said, who it goes to.
struct PointAskCard: View {
    @ObservedObject var point: PointAsk
    @ObservedObject var dictation: Dictation
    @FocusState private var focused: Bool

    init(point: PointAsk) {
        self.point = point
        self.dictation = point.dictation
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Image(systemName: "hand.point.up.left.fill").foregroundStyle(Theme.amber)
                Text("POINT & ASK").font(.system(size: 11, weight: .semibold)).tracking(0.8).foregroundStyle(Theme.ink)
                Spacer()
                if dictation.isRecording {
                    Label("Listening", systemImage: "waveform").font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.red)
                        .symbolEffect(.variableColor.iterative, isActive: true)
                }
                Button { point.cancel() } label: {
                    Image(systemName: "xmark").font(.system(size: 10, weight: .semibold)).foregroundStyle(Theme.muted)
                }
                .buttonStyle(.plain).accessibilityLabel("Cancel")
            }
            Group {
                if let image = point.image {
                    Image(nsImage: image).resizable().aspectRatio(contentMode: .fit)
                        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Theme.hairline))
                } else if let problem = point.problem {
                    Text(problem).font(.system(size: 11)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                } else {
                    HStack(spacing: 6) { ProgressView().controlSize(.small); Text("Capturing…").font(.system(size: 11)).foregroundStyle(Theme.muted) }
                }
            }
            contextLines
            TextField(dictation.isRecording ? "Listening… say what you want" : "What should the agent do?", text: $point.text, axis: .vertical)
                .textFieldStyle(.plain).font(.system(size: 13)).lineLimit(2...5)
                .padding(.horizontal, 10).padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.faint))
                .focused($focused)
                .onSubmit { point.send() }
            HStack {
                Menu {
                    ForEach(point.model?.agents.filter(\.isLive) ?? []) { a in
                        Button("@\(a.handle) · \(a.project ?? "")") { point.recipientId = a.id }
                    }
                } label: {
                    Text(point.recipient.map { "To @\($0.handle)" } ?? "Choose an agent").font(.system(size: 11.5, weight: .medium))
                        .foregroundStyle(Theme.muted)
                }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                Spacer()
                Button("Send ⏎") { point.send() }
                    .buttonStyle(.plain).font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.black.opacity(0.85))
                    .padding(.horizontal, 12).padding(.vertical, 5)
                    .background(Capsule().fill(point.recipient == nil ? Theme.amber.opacity(0.35) : Theme.amber))
                    .disabled(point.recipient == nil)
            }
        }
        .padding(14)
        .frame(width: 360)
        .glassSurface(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .environment(\.colorScheme, .dark)
        .onExitCommand { point.cancel() }
        .onAppear { focused = true }
    }

    @ViewBuilder private var contextLines: some View {
        let c = point.context
        VStack(alignment: .leading, spacing: 2) {
            if let app = c.app {
                Text([app, c.window].compactMap { $0 }.joined(separator: " · ")).lineLimit(1)
            }
            if let url = c.url { Text(url).lineLimit(1).truncationMode(.middle) }
            if let sel = c.selection, !sel.isEmpty { Text("Selected: “\(sel)”").lineLimit(2) }
        }
        .font(.system(size: 11)).foregroundStyle(Theme.muted)
    }
}
