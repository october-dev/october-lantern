import AppKit
import ScreenCaptureKit
import SwiftUI

/// Show me: Lantern points at the control you asked about, on your screen, one step at a time.
///
/// The engine runs the guide (engine/src/show): it asks for a picture of the display, reads the
/// visible windows' controls through Cua Driver, checks October's answer and moves Lantern's cursor.
/// This class takes the pictures (the whole display, Lantern's own windows left out), sends each
/// round to October (`/api/lantern/ask`, mode "show") and shows the card. Without Cua Driver it
/// marks the place on the card's picture instead of pointing on screen.
///
/// What's sent to October each round: the picture of the whole display and the names and places of
/// the buttons, menus and tabs on it (never what's typed in fields). Nothing is kept: pictures live
/// in memory while the guide is open.
@MainActor
final class ShowMe: ObservableObject {
    static let shared = ShowMe()
    /// The engine's `show.*` requests (set by AppModel).
    var send: ((String, [String: Any]) -> Bool)?

    enum Phase: Equatable {
        case looking, asking, showing, notVisible, finished
        case failed(String)
    }

    @Published private(set) var open = false
    @Published private(set) var question = ""
    @Published private(set) var phase: Phase = .looking
    @Published private(set) var instruction = ""
    @Published private(set) var step = 1
    /// The latest picture, and the target on it when Lantern can't point on screen.
    @Published private(set) var picture: NSImage?
    @Published private(set) var marker: CGRect?
    @Published private(set) var live = false
    @Published private(set) var upgrade: URL?
    /// The first time, the card says what Show me sends.
    @Published var explained = UserDefaults.standard.bool(forKey: "showMeExplained") {
        didSet { UserDefaults.standard.set(explained, forKey: "showMeExplained") }
    }

    private var runId = ""
    private var screen: NSScreen?
    private var jpeg: Data?
    private var imageSize = CGSize.zero
    private var askTask: Task<Void, Never>?
    private var captureTask: Task<Void, Never>?
    private var panel: FloatingPanel?
    private var host: NSHostingController<ShowMeCard>?

    /// Starts a guide for `question` on `screen` (the display the person is looking at). A guide
    /// already open is replaced.
    func start(question: String, on screen: NSScreen?) {
        let words = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !words.isEmpty else { return }
        if open { stop() }
        runId = UUID().uuidString
        self.question = String(words.prefix(4000))
        self.screen = screen ?? NSScreen.main
        phase = .looking
        instruction = ""
        step = 1
        picture = nil
        marker = nil
        live = false
        upgrade = nil
        open = true
        showCard()
        guard send?("show.start", ["runId": runId]) == true else {
            phase = .failed("Lantern's engine isn't running. Try again in a moment.")
            return
        }
        Analytics.shared.capture("show_me_started", [:])
    }

    /// The person did the step: look for the next one.
    func next() {
        guard open else { return }
        _ = send?("show.next", ["runId": runId, "step": step])
    }

    /// Look at the screen again for this step (the window changed, or the pointer looks wrong).
    func again() {
        guard open else { return }
        _ = send?("show.again", ["runId": runId])
    }

    func stop() {
        guard open else { return }
        askTask?.cancel()
        captureTask?.cancel()
        _ = send?("show.stop", ["runId": runId])
        open = false
        jpeg = nil
        picture = nil
        panel?.orderOut(nil)
        Analytics.shared.capture("show_me_stopped", ["step": step])
    }

    /// A message from the engine about this guide.
    func receive(_ m: ShowMessage) {
        guard open, m.runId == runId else { return }
        switch m.type {
        case .showCapture:
            step = m.step
            captureTask?.cancel()
            captureTask = Task { await capture(step: m.step) }
        case .showQuery:
            ask(m)
        case .showState:
            apply(m)
        }
    }

    // MARK: Pictures

    /// The display in Cua Driver's screen space: points, origin at the main display's top left.
    private static func displayRect(_ screen: NSScreen) -> [String: Double] {
        let main = NSScreen.screens.first?.frame ?? screen.frame
        let f = screen.frame
        return ["x": f.minX, "y": main.maxY - f.maxY, "w": f.width, "h": f.height]
    }

    private func capture(step: Int) async {
        guard let screen else { return }
        guard CGPreflightScreenCaptureAccess() else {
            CGRequestScreenCaptureAccess()
            phase = .failed("Allow Screen Recording for October Lantern in System Settings, then try again.")
            return
        }
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            guard !Task.isCancelled, open else { return }
            guard let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID,
                  let display = content.displays.first(where: { $0.displayID == number }) else {
                phase = .failed("That display is no longer connected.")
                return
            }
            let mine = content.applications.filter { $0.processID == ProcessInfo.processInfo.processIdentifier }
            let filter = SCContentFilter(display: display, excludingApplications: mine, exceptingWindows: [])
            let config = SCStreamConfiguration()
            config.width = Int(screen.frame.width * screen.backingScaleFactor)
            config.height = Int(screen.frame.height * screen.backingScaleFactor)
            config.showsCursor = false
            let raw = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
            guard !Task.isCancelled, open, let (data, size) = Self.encode(raw) else {
                if open, !Task.isCancelled { phase = .failed("Couldn't prepare the picture of the screen.") }
                return
            }
            jpeg = data
            imageSize = size
            picture = NSImage(data: data)
            var fields: [String: Any] = ["runId": runId, "step": step, "display": Self.displayRect(screen),
                                         "imageWidth": Int(size.width), "imageHeight": Int(size.height),
                                         "excludePids": [Int(ProcessInfo.processInfo.processIdentifier)]]
            fields["display"] = Self.displayRect(screen)
            _ = send?("show.captured", fields)
        } catch {
            if open, !Task.isCancelled { phase = .failed("Couldn't take a picture of the screen: \(error.localizedDescription)") }
        }
    }

    /// The picture as a JPEG for October: at most 1600 pixels on its longer side and under 2.5 MB,
    /// with its exact size (the engine converts every position with it).
    static func encode(_ image: CGImage) -> (Data, CGSize)? {
        for (side, quality) in [(1600.0, 0.82), (1600.0, 0.6), (1200.0, 0.6)] {
            let scale = min(1, side / CGFloat(max(image.width, image.height)))
            let w = max(1, Int((CGFloat(image.width) * scale).rounded())), h = max(1, Int((CGFloat(image.height) * scale).rounded()))
            guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { return nil }
            ctx.interpolationQuality = .high
            ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
            guard let scaled = ctx.makeImage(),
                  let data = NSBitmapImageRep(cgImage: scaled).representation(using: .jpeg, properties: [.compressionFactor: quality]) else { return nil }
            if data.count <= 2_500_000 { return (data, CGSize(width: w, height: h)) }
        }
        return nil
    }

    // MARK: Asking October

    private func ask(_ q: ShowMessage) {
        guard let round = q.round, let captureId = q.captureId, let jpeg else { return }
        var guidance: [String: Any] = [
            "roundId": UUID().uuidString.lowercased(), "step": q.step, "captureId": captureId,
            "imageSize": [Int(imageSize.width), Int(imageSize.height)],
            "elements": (q.elements ?? []).map(\.json), "shown": q.shown ?? [],
        ]
        if let goal = q.goalId { guidance["goalId"] = goal }
        let request = OctoberAI.Request(turns: [], question: question, image: jpeg, context: "", mode: "show", guidance: guidance)
        let run = runId
        instruction = ""
        phase = .asking
        askTask?.cancel()
        askTask = Task {
            var result: [String: Any] = [:]
            do {
                for try await piece in OctoberAI.stream(request) {
                    guard !Task.isCancelled, open, runId == run else { return }
                    switch piece {
                    case .text(let t): instruction += t
                    case .guide(let g, let goal):
                        result["guidance"] = g ?? NSNull()
                        if let goal { result["goalId"] = goal }
                    case .note, .task: break
                    }
                }
                result["instruction"] = String(instruction.trimmingCharacters(in: .whitespacesAndNewlines).prefix(500))
                if result["guidance"] == nil { result["guidance"] = NSNull() }
            } catch is CancellationError {
                return
            } catch {
                guard !Task.isCancelled, open, runId == run else { return }
                let message = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
                upgrade = (error as? OctoberAI.Failure)?.upgradeURL
                result = ["error": message]
                phase = .failed(message)
            }
            guard !Task.isCancelled, open, runId == run else { return }
            _ = send?("show.result", ["runId": run, "round": round, "captureId": captureId, "result": result])
        }
    }

    // MARK: The engine's state

    private func apply(_ m: ShowMessage) {
        step = m.step
        switch m.phase {
        case "looking": phase = .looking
        case "asking": phase = .asking
        case "showing":
            live = m.live ?? false
            if let r = m.target?.image, r.count == 4 { marker = live ? nil : CGRect(x: r[0], y: r[1], width: r[2], height: r[3]) }
            phase = .showing
            place(targetX: m.target?.image.first)
            Analytics.shared.capture("show_me_shown", ["step": m.step, "live": live])
        case "notVisible": phase = .notVisible
        case "finished": phase = .finished
        case "changed": phase = .looking
        case "failed": phase = .failed(m.message ?? "Something went wrong. Try again.")
        case "ended":
            switch m.reason {
            case "stopped", "replaced": break
            case "limit": phase = .failed("That's as far as one guide goes. Ask again for the rest.")
            case "expired": phase = .failed("This guide timed out. Ask again to start over.")
            default: phase = .failed("Show me stopped. Try again.")
            }
            askTask?.cancel()
        default: break
        }
    }

    // MARK: The card

    static let cardWidth: CGFloat = 320

    private func showCard() {
        let panel = self.panel ?? FloatingPanel(keyable: true)
        if self.panel == nil {
            let host = NSHostingController(rootView: ShowMeCard(show: self))
            host.sizingOptions = [.preferredContentSize]
            panel.contentView = GlassContainer(content: host.view, cornerRadius: 18, tint: 0.62)
            self.host = host
            self.panel = panel
        }
        place(targetX: nil)
        // In front, but not taking the keyboard: the person keeps working in their app.
        panel.orderFrontRegardless()
    }

    /// On the side of the display away from the target, near the bottom.
    private func place(targetX: Double?) {
        guard let panel, let host, let screen else { return }
        let size = host.sizeThatFits(in: NSSize(width: Self.cardWidth, height: 2000))
        let visible = screen.visibleFrame
        var right = Preferences.shared.edge != "left"
        if let x = targetX, imageSize.width > 0 { right = x / imageSize.width < 0.5 }
        let x = right ? visible.maxX - size.width - 16 : visible.minX + 16
        panel.setFrame(NSRect(x: x, y: visible.minY + 16, width: Self.cardWidth, height: max(size.height, 120)), display: true)
    }

    /// Resizes the card to its content, keeping its bottom edge.
    func fit() {
        guard let panel, let host else { return }
        let height = max(host.sizeThatFits(in: NSSize(width: Self.cardWidth, height: 2000)).height, 120)
        guard abs(panel.frame.height - height) > 0.5 else { return }
        panel.setFrame(NSRect(x: panel.frame.minX, y: panel.frame.minY, width: Self.cardWidth, height: height), display: true)
    }
}

struct ShowMeCard: View {
    @ObservedObject var show: ShowMe

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "hand.point.up.left.fill").font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.amber)
                Text("Show me").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                Text("Step \(show.step)").font(.system(size: 11)).foregroundStyle(Theme.muted)
                Spacer()
                Button { show.stop() } label: {
                    Image(systemName: "xmark").font(.system(size: 10, weight: .bold)).foregroundStyle(Theme.muted)
                        .frame(width: 22, height: 22).background(Circle().fill(Theme.faint))
                }
                .buttonStyle(.plain).help("Stop showing me (Esc)").accessibilityLabel("Stop showing me")
            }
            Text(show.question).font(.system(size: 11.5)).foregroundStyle(Theme.muted).lineLimit(2)
            if !show.explained {
                HStack(alignment: .top, spacing: 6) {
                    Text("Show me sends a picture of this whole screen, and the names of the buttons and menus on it, to October's AI each step. Nothing is kept.")
                        .font(.system(size: 11)).foregroundStyle(Theme.ink.opacity(0.85)).fixedSize(horizontal: false, vertical: true)
                    Button("OK") { show.explained = true }.buttonStyle(.plain).font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.amber)
                }
                .padding(8).background(RoundedRectangle(cornerRadius: 8).fill(Theme.faint))
            }
            status
            if !show.live, show.marker != nil, let picture = show.picture { marked(picture) }
            buttons
        }
        .padding(14)
        .frame(width: ShowMe.cardWidth, alignment: .topLeading)
        .fixedSize(horizontal: false, vertical: true)
        .background(RoundedRectangle(cornerRadius: 18, style: .continuous).fill(Color(white: 0.08).opacity(0.9)))
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay(GlassRim(shape: RoundedRectangle(cornerRadius: 18, style: .continuous)))
        .environment(\.colorScheme, .dark)
        .onExitCommand { show.stop() }
        .onChange(of: show.instruction) { _, _ in show.fit() }
        .onChange(of: show.phase) { _, _ in show.fit() }
    }

    @ViewBuilder private var status: some View {
        switch show.phase {
        case .looking: working("Looking at your screen…")
        case .asking:
            if show.instruction.isEmpty { working("Finding it…") } else { instruction }
        case .showing:
            instruction
            Text(show.live ? "Lantern's pointer is on it. Do this step, then press Next." : "It's marked on the picture below.")
                .font(.system(size: 11)).foregroundStyle(Theme.muted)
        case .notVisible:
            instruction
            Text("Lantern can't point at it on this screen.").font(.system(size: 11)).foregroundStyle(Theme.muted)
        case .finished:
            instruction
            Text("That's the last step.").font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.green)
        case .failed(let message):
            Text(message).font(.system(size: 12)).foregroundStyle(show.upgrade != nil ? Theme.amber : Theme.red)
                .fixedSize(horizontal: false, vertical: true)
            if let url = show.upgrade { UpgradeButton(url: url, from: "show_me") }
        }
    }

    private var instruction: some View {
        Text(markdown(show.instruction.isEmpty ? "…" : show.instruction)).font(.system(size: 13)).foregroundStyle(Theme.ink)
            .fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
    }

    private func working(_ text: String) -> some View {
        HStack(spacing: 6) {
            ProgressView().controlSize(.small)
            Text(text).font(.system(size: 12)).foregroundStyle(Theme.muted)
        }
    }

    /// The picture with the target ringed, when Lantern can't point on screen.
    private func marked(_ picture: NSImage) -> some View {
        Image(nsImage: picture).resizable().aspectRatio(contentMode: .fit)
            .overlay {
                GeometryReader { geo in
                    if let m = show.marker, picture.size.width > 0 {
                        let s = geo.size.width / picture.size.width
                        RoundedRectangle(cornerRadius: 4).stroke(Theme.amber, lineWidth: 2.5)
                            .frame(width: max(m.width * s, 10) + 8, height: max(m.height * s, 10) + 8)
                            .position(x: m.midX * s, y: m.midY * s)
                    }
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            .accessibilityLabel("Picture of your screen with the place marked")
    }

    private var buttons: some View {
        HStack(spacing: 8) {
            switch show.phase {
            case .showing:
                primary("Next") { show.next() }.help("I did it: show the next step")
                secondary("Look again") { show.again() }
            case .notVisible:
                primary("Look again") { show.again() }.help("Open what it said, then look again")
            case .failed, .finished:
                secondary("Close") { show.stop() }
            default:
                EmptyView()
            }
            Spacer()
            if show.phase != .finished { secondary("Stop") { show.stop() } }
        }
    }

    private func primary(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title).font(.system(size: 12, weight: .semibold)).foregroundStyle(.black.opacity(0.85))
                .padding(.horizontal, 14).padding(.vertical, 5).background(Capsule().fill(Theme.amber))
        }
        .buttonStyle(.plain)
    }

    private func secondary(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title).font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.ink)
                .padding(.horizontal, 12).padding(.vertical, 5).background(Capsule().fill(Theme.faint))
        }
        .buttonStyle(.plain)
    }
}
