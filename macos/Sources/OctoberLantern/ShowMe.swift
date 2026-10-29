import AppKit
import ScreenCaptureKit
import QuartzCore
import SwiftUI

/// Show me: Lantern points at the control you asked about, on your screen, one step at a time.
///
/// The engine runs the guide (engine/src/show): it asks for a picture of the display, reads the
/// visible windows' controls through Cua Driver, checks October's answer and authorizes a short native highlight.
/// This class takes the pictures (the whole display, Lantern's own windows left out), sends each
/// round to October (`/api/lantern/ask`, mode "show") and shows the card. Without Cua Driver it
/// marks the place on the card's picture instead of pointing on screen.
///
/// What's sent to October each round: the picture of the whole display and the names and places of
/// the buttons, menus and tabs on it (field values are excluded from the control list; visible content can still appear in the picture). Nothing is kept: pictures live
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
    /// What a failed round offers (the trial and Lifetime, your key's settings, October's AI).
    @Published private(set) var fix: OctoberAI.Fix?
    /// Your own key is answering: its provider.
    @Published private(set) var via: String?
    /// The first time, the card says what Show me sends.
    @Published var explained = UserDefaults.standard.bool(forKey: "showMeExplained") {
        didSet { UserDefaults.standard.set(explained, forKey: "showMeExplained") }
    }

    private var runId = ""
    /// "Ask October's AI instead" was chosen: the rest of this guide asks October.
    private var useOctober = false
    /// The round whose own-key provider failed before answering, kept from the engine until the
    /// person chooses to ask October's AI instead (or closes the guide).
    private var pendingRound: ShowMessage?

    var canAskOctober: Bool { pendingRound != nil }

    /// Ask October's AI instead: the same round again, and the rest of the guide, without your key.
    func askOctober() {
        guard let q = pendingRound, open else { return }
        pendingRound = nil
        useOctober = true
        fix = nil
        via = nil
        Analytics.shared.capture("own_key_fallback", [:])
        ask(q)
    }
    private var screen: NSScreen?
    private var jpeg: Data?
    private var imageSize = CGSize.zero
    private var askTask: Task<Void, Never>?
    private var captureTask: Task<Void, Never>?
    private var panel: FloatingPanel?
    private var host: NSHostingController<ShowMeCard>?
    private var ring: NSPanel?
    private var ringExpiry: Task<Void, Never>?
    /// Where the ring is, on screen, and the watch for clicks inside it (`clicked`).
    private var ringTarget: CGRect?
    private var clickMonitor: Any?
    private var cueRevision = 0

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
        fix = nil
        via = nil
        useOctober = false
        pendingRound = nil
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
        ringExpiry?.cancel(); cueRevision += 1
        guard open else { return }
        askTask?.cancel()
        captureTask?.cancel()
        _ = send?("show.stop", ["runId": runId])
        open = false
        jpeg = nil
        picture = nil
        ring?.orderOut(nil)
        stopWatchingClicks()
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
            captureTask = Task { await capture(step: m.step, captureId: m.captureId ?? "") }
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

    private func capture(step: Int, captureId: String) async {
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
            var fields: [String: Any] = ["runId": runId, "step": step, "captureId": captureId, "display": Self.displayRect(screen),
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
        var request = OctoberAI.Request(turns: [], question: question, image: jpeg, context: "", mode: "show", guidance: guidance)
        request.ownKey = !useOctober
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
                    case .via(let p): via = p
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
                var fix = (error as? OctoberAI.Failure)?.fix
                if fix == .useOctober {
                    // Before any answer: the engine waits while the person decides.
                    if instruction.isEmpty {
                        self.fix = fix
                        pendingRound = q
                        phase = .failed(message)
                        return
                    }
                    fix = nil
                }
                self.fix = fix
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
        cueRevision += 1; ringExpiry?.cancel()
        ring?.orderOut(nil); ringTarget = nil
        switch m.phase {
        case "hide": live = false; marker = nil
        case "looking": phase = .looking
        case "asking": phase = .asking
        case "showing":
            let now = Date().timeIntervalSince1970 * 1000
            live = m.live == true && (m.issuedAt ?? .infinity) <= now && (m.expiresAt ?? 0) > now
            // The picture is marked too, as a backup to the ring on screen.
            if let r = m.target?.image, r.count == 4 { marker = CGRect(x: r[0], y: r[1], width: r[2], height: r[3]) }
            if live, let rect = m.target?.image {
                // Until Next, Look again, Stop or the screen changing (the engine says "hide"),
                // and never past the engine's lease.
                let remaining = ((m.expiresAt ?? now) - now) / 1000
                showRing(rect, remaining: remaining)
                let revision = cueRevision, run = runId
                ringExpiry = Task { @MainActor in
                    try? await Task.sleep(for: .seconds(remaining))
                    guard !Task.isCancelled, open, runId == run, cueRevision == revision else { return }
                    ring?.orderOut(nil); live = false
                }
            }
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
            captureTask?.cancel()
            askTask?.cancel()
            jpeg = nil
            marker = nil
            picture = nil
            if m.reason == "stopped" || m.reason == "replaced" { stop() }
        default: break
        }
    }

    /// A pass-through overlay over the whole display, sized from this capture's exact transform: a
    /// ring around the place, and an arrow pointing at it from the middle of the screen with the
    /// step's label. It never accepts input.
    private func showRing(_ box: [Double], remaining: Double) {
        guard box.count == 4, let screen, imageSize.width > 0, imageSize.height > 0 else { return }
        let sx = screen.frame.width / imageSize.width, sy = screen.frame.height / imageSize.height
        let target = CGRect(x: screen.frame.minX + box[0] * sx,
                            y: screen.frame.maxY - (box[1] + box[3]) * sy,
                            width: box[2] * sx, height: box[3] * sy)
        let panel = ring ?? NSPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isOpaque = false; panel.backgroundColor = .clear; panel.hasShadow = false
        panel.ignoresMouseEvents = true; panel.level = .screenSaver
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        // In the overlay's own coordinates (top-left origin).
        let local = CGRect(x: target.minX - screen.frame.minX, y: screen.frame.maxY - target.maxY, width: target.width, height: target.height)
        panel.contentView = NSHostingView(rootView: ShowMeOverlay(step: step, target: local, bounds: screen.frame.size))
        panel.setFrame(screen.frame, display: true)
        if let view = panel.contentView {
            view.wantsLayer = true
            let expiry = CABasicAnimation(keyPath: "opacity")
            expiry.fromValue = 1; expiry.toValue = 0
            expiry.beginTime = CACurrentMediaTime() + remaining; expiry.duration = 0.01
            expiry.fillMode = .forwards; expiry.isRemovedOnCompletion = false
            view.layer?.add(expiry, forKey: "marker-lease")
        }
        ring = panel; panel.orderFrontRegardless()
        ringTarget = target
        watchClicks()
    }

    /// A click inside the ring is the step being done: the ring goes at once, and a moment later
    /// (once what the click opened is on screen) Lantern looks for the next step, as Next does.
    private func watchClicks() {
        guard clickMonitor == nil else { return }
        clickMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            let at = NSEvent.mouseLocation
            Task { @MainActor in self?.clicked(at) }
        }
    }

    private func clicked(_ at: CGPoint) {
        guard open, live, let target = ringTarget, target.insetBy(dx: -16, dy: -16).contains(at) else { return }
        ring?.orderOut(nil); live = false; ringTarget = nil
        cueRevision += 1
        let revision = cueRevision, run = runId
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(1))
            guard open, runId == run, cueRevision == revision, phase == .showing else { return }
            next()
        }
    }

    private func stopWatchingClicks() {
        if let monitor = clickMonitor { NSEvent.removeMonitor(monitor) }
        clickMonitor = nil
        ringTarget = nil
    }

    /// What Lantern draws on the screen: an orange ring around the place (never smaller than 44
    /// points), and a thick arrow pointing at it from the middle of the screen, with the step's
    /// label at its tail, kept on screen. Both move gently to catch the eye (still, with Reduce
    /// Motion).
    private struct ShowMeOverlay: View {
        let step: Int
        let target: CGRect
        let bounds: CGSize
        @State private var pulse = false
        @Environment(\.accessibilityReduceMotion) private var reduceMotion

        var body: some View {
            let size = CGSize(width: max(44, target.width + 16), height: max(44, target.height + 16))
            // Kept whole on screen, even around the Apple menu in the corner.
            let ring = CGRect(x: min(max(target.midX - size.width / 2, 9), bounds.width - size.width - 9),
                              y: min(max(target.midY - size.height / 2, 9), bounds.height - size.height - 9),
                              width: size.width, height: size.height)
            let center = CGPoint(x: ring.midX, y: ring.midY)
            // Towards the middle of the screen, so the arrow and label are always on it.
            var dx = bounds.width / 2 - center.x, dy = bounds.height / 2 - center.y
            let length = max(1, (dx * dx + dy * dy).squareRoot())
            (dx, dy) = length < 40 ? (0.6, 0.8) : (dx / length, dy / length)
            let reach = max(ring.width, ring.height) / 2 + 14
            let bob: CGFloat = pulse ? 10 : 0
            let tip = CGPoint(x: center.x + dx * (reach + bob), y: center.y + dy * (reach + bob))
            let tail = CGPoint(x: tip.x + dx * 170, y: tip.y + dy * 170)
            let labelAt = CGPoint(x: min(max(tail.x + dx * 40, 110), bounds.width - 110), y: min(max(tail.y + dy * 26, 30), bounds.height - 30))
            return ZStack(alignment: .topLeading) {
                RoundedRectangle(cornerRadius: 12).stroke(Color.black.opacity(0.55), lineWidth: 10)
                    .frame(width: ring.width, height: ring.height).position(center)
                RoundedRectangle(cornerRadius: 12).stroke(Color.orange, lineWidth: 6)
                    .frame(width: ring.width, height: ring.height).position(center)
                    .shadow(color: .orange.opacity(0.9), radius: pulse ? 16 : 6)
                    .scaleEffect(pulse ? 1.08 : 1, anchor: UnitPoint(x: center.x / bounds.width, y: center.y / bounds.height))
                Arrow(from: tail, to: tip).stroke(Color.black.opacity(0.55), style: StrokeStyle(lineWidth: 13, lineCap: .round, lineJoin: .round))
                Arrow(from: tail, to: tip).stroke(Color.orange, style: StrokeStyle(lineWidth: 8, lineCap: .round, lineJoin: .round))
                    .shadow(color: .orange.opacity(0.7), radius: 6)
                Text("Lantern · Step \(step)").font(.system(size: 17, weight: .bold))
                    .foregroundStyle(.black).padding(.horizontal, 14).padding(.vertical, 7)
                    .background(Capsule().fill(Color.orange))
                    .overlay(Capsule().strokeBorder(Color.black.opacity(0.35)))
                    .shadow(color: .black.opacity(0.4), radius: 6, y: 2)
                    .fixedSize()
                    .position(labelAt)
            }
            .frame(width: bounds.width, height: bounds.height)
            .onAppear {
                guard !reduceMotion else { return }
                withAnimation(.easeInOut(duration: 0.7).repeatForever(autoreverses: true)) { pulse = true }
            }
        }
    }

    /// A line with an arrowhead at `to`.
    private struct Arrow: Shape {
        var from: CGPoint
        var to: CGPoint

        var animatableData: AnimatablePair<AnimatablePair<CGFloat, CGFloat>, AnimatablePair<CGFloat, CGFloat>> {
            get { AnimatablePair(AnimatablePair(from.x, from.y), AnimatablePair(to.x, to.y)) }
            set { (from, to) = (CGPoint(x: newValue.first.first, y: newValue.first.second), CGPoint(x: newValue.second.first, y: newValue.second.second)) }
        }

        func path(in rect: CGRect) -> Path {
            let angle = atan2(to.y - from.y, to.x - from.x), head: CGFloat = 28, spread: CGFloat = .pi / 6
            var p = Path()
            p.move(to: from); p.addLine(to: to)
            p.move(to: CGPoint(x: to.x - head * cos(angle - spread), y: to.y - head * sin(angle - spread)))
            p.addLine(to: to)
            p.addLine(to: CGPoint(x: to.x - head * cos(angle + spread), y: to.y - head * sin(angle + spread)))
            return p
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
                    Text("Show me sends a picture of this whole screen, and the names of the buttons and menus on it, to October's AI each step. Lantern doesn't save these pictures.")
                        .font(.system(size: 11)).foregroundStyle(Theme.ink.opacity(0.85)).fixedSize(horizontal: false, vertical: true)
                    Button("OK") { show.explained = true }.buttonStyle(.plain).font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.amber)
                }
                .padding(8).background(RoundedRectangle(cornerRadius: 8).fill(Theme.faint))
            }
            status
            if show.marker != nil, let picture = show.picture { marked(picture) }
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
            Text(show.live ? "It's circled on your screen in orange. Do this step, then press Next." : "It's marked on the picture below.")
                .font(.system(size: 11)).foregroundStyle(Theme.muted)
        case .notVisible:
            instruction
            Text("Lantern can't point at it on this screen.").font(.system(size: 11)).foregroundStyle(Theme.muted)
        case .finished:
            instruction
            Text("That's the last step.").font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.green)
        case .failed(let message):
            Text(message).font(.system(size: 12)).foregroundStyle(show.fix != nil ? Theme.amber : Theme.red)
                .fixedSize(horizontal: false, vertical: true)
            if let fix = show.fix {
                FixButtons(fix: fix, from: "show_me", useOctober: show.canAskOctober ? { show.askOctober() } : nil)
            }
        }
    }

    private var instruction: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(markdown(show.instruction.isEmpty ? "…" : show.instruction)).font(.system(size: 13)).foregroundStyle(Theme.ink)
                .fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
            if let via = show.via { ViaLabel(provider: via) }
        }
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
                        RoundedRectangle(cornerRadius: 5).stroke(Color.orange, lineWidth: 3)
                            .frame(width: max(m.width * s, 14) + 10, height: max(m.height * s, 14) + 10)
                            .shadow(color: .orange.opacity(0.9), radius: 4)
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

/// The button that starts Show me (Ask Lantern's answers, Point & Ask). Hovering says what it does
/// and what it sends; Lantern's panels don't take focus, so a system tooltip wouldn't show.
struct ShowMeButton: View {
    var title = "Show me on screen"
    let action: () -> Void
    @State private var hovering = false

    static let explanation = "Lantern points at each step on your screen, one at a time, until you're done. Each step sends a picture of your screen to October."

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: "hand.point.up.left.fill").font(.system(size: 13, weight: .semibold))
                Text(title).font(.system(size: 13, weight: .semibold))
            }
            .foregroundStyle(Theme.amber)
            .padding(.horizontal, 14).padding(.vertical, 7)
            .background(Capsule().fill(Theme.amber.opacity(hovering ? 0.24 : 0.14)))
            .overlay(Capsule().strokeBorder(Theme.amber.opacity(hovering ? 0.7 : 0.4)))
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .onHover { inside in withAnimation(.easeOut(duration: 0.15)) { hovering = inside } }
        .overlay(alignment: .topLeading) {
            // Above the button: a point at its top-left corner, with the note growing upwards.
            if hovering {
                Color.clear.frame(width: 0, height: 0).overlay(alignment: .bottomLeading) {
                    Text(Self.explanation)
                        .font(.system(size: 11.5)).foregroundStyle(Theme.ink)
                        .frame(width: 230, alignment: .leading).fixedSize(horizontal: false, vertical: true)
                        .padding(.horizontal, 10).padding(.vertical, 8)
                        .background(RoundedRectangle(cornerRadius: 9, style: .continuous).fill(Color(white: 0.07)))
                        .overlay(RoundedRectangle(cornerRadius: 9, style: .continuous).strokeBorder(Theme.stroke))
                        .fixedSize()
                        .padding(.bottom, 6)
                }
                .allowsHitTesting(false)
                .transition(.opacity)
            }
        }
        .zIndex(hovering ? 1 : 0)
        .accessibilityHint(Self.explanation)
    }
}
