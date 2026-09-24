import AppKit
import Combine
import SwiftUI

/// A borderless, non-activating panel: clicking it doesn't take focus from the app you're in,
/// it floats above other windows, and it appears on every Space and over full-screen apps.
final class FloatingPanel: NSPanel {
    private let keyable: Bool

    init(keyable: Bool) {
        self.keyable = keyable
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isFloatingPanel = true
        level = .statusBar
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        backgroundColor = .clear
        isOpaque = false
        // The glass draws its own edge; a window shadow would outline the rectangular window.
        hasShadow = false
        hidesOnDeactivate = false
        becomesKeyOnlyIfNeeded = !keyable
        isReleasedWhenClosed = false
    }

    override var canBecomeKey: Bool { keyable }
    override var canBecomeMain: Bool { false }
}

enum Edge: String { case left, right }

/// Owns the pill and the panel beside it, and keeps them positioned.
@MainActor
final class WindowController {
    private let model: AppModel
    private let pill = FloatingPanel(keyable: false)
    private let panel = FloatingPanel(keyable: true)
    private var pillHost: FirstClickHostingView<PillView>!
    private var panelHost: FirstClickHostingView<PanelView>!
    private var subscriptions = Set<AnyCancellable>()
    private var outsideClickMonitor: Any?
    private var dragStart: (mouse: NSPoint, origin: NSPoint)?
    var onMenu: ((NSView) -> Void)?

    private var edge: Edge {
        get { Edge(rawValue: Preferences.shared.edge) ?? .right }
        set { Preferences.shared.edge = newValue.rawValue }
    }
    /// Top of the pill as a fraction of the screen's visible height. The pill grows downward from
    /// here when it expands.
    private var topFraction: CGFloat {
        get { UserDefaults.standard.object(forKey: "pillTop") as? CGFloat ?? 0.72 }
        set { UserDefaults.standard.set(newValue, forKey: "pillTop") }
    }
    private var hoverTimer: Timer?
    private var outsideSince: Date?

    init(model: AppModel) {
        self.model = model
        pillHost = FirstClickHostingView(rootView: PillView(
            model: model, dictation: model.dictation,
            onDrag: { [weak self] in self?.dragMoved() },
            onDragEnd: { [weak self] in self?.dragEnded() },
            onMenu: { [weak self] in
                guard let self else { return }
                self.onMenu?(self.pillHost)
            }
        ))
        pill.contentView = GlassContainer(content: pillHost, cornerRadius: nil, tint: 0.45)
        panelHost = FirstClickHostingView(rootView: PanelView(model: model, dictation: model.dictation))
        panel.contentView = GlassContainer(content: panelHost, cornerRadius: 22, tint: 0.5)

        // Re-lay out whenever the model changes (agent count changes the pill's height).
        model.objectWillChange
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in self?.layout() }
            .store(in: &subscriptions)
        model.$panel
            .removeDuplicates()
            .receive(on: RunLoop.main)
            .sink { [weak self] mode in self?.setPanelVisible(mode != nil) }
            .store(in: &subscriptions)
        // Expand while the mouse is over the pill; tuck back in shortly after it leaves.
        // Polling the mouse position is reliable for non-activating panels, unlike tracking areas.
        hoverTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.checkHover() }
        }
        Preferences.shared.$edge
            .dropFirst()
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in self?.layout() }
            .store(in: &subscriptions)
        NotificationCenter.default.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.layout() }
        }
    }

    var pillVisible: Bool { pill.isVisible }

    /// Expands the pill for a few seconds so people can find it (after the welcome).
    func flash() {
        model.pillExpanded = true
        outsideSince = Date().addingTimeInterval(3)
    }

    func showPill() {
        layout()
        pill.orderFrontRegardless()
    }

    func hidePill() {
        model.panel = nil
        pill.orderOut(nil)
    }

    private var screen: NSScreen {
        pill.screen ?? NSScreen.screens.first { $0.frame.contains(NSEvent.mouseLocation) } ?? NSScreen.main ?? NSScreen.screens[0]
    }

    func layout() {
        guard dragStart == nil else { return }
        let visible = screen.visibleFrame
        let size = pillHost.fittingSize
        let x = edge == .right ? visible.maxX - size.width - 10 : visible.minX + 10
        let top = visible.minY + visible.height * topFraction
        let y = min(max(top - size.height, visible.minY + 8), visible.maxY - size.height - 8)
        pill.setFrame(NSRect(x: x, y: y, width: size.width, height: size.height), display: true)
        layoutPanel()
    }

    private func layoutPanel() {
        guard panel.isVisible else { return }
        let visible = screen.visibleFrame
        let size = panelHost.fittingSize
        let x = edge == .right ? pill.frame.minX - size.width - 10 : pill.frame.maxX + 10
        // Align the panel's top with the pill's top, kept on screen.
        let y = min(max(pill.frame.maxY - size.height, visible.minY + 8), visible.maxY - size.height - 8)
        panel.setFrame(NSRect(x: x, y: y, width: size.width, height: size.height), display: true)
    }

    private func setPanelVisible(_ visible: Bool) {
        if visible {
            panel.alphaValue = 0
            panel.makeKeyAndOrderFront(nil)
            layoutPanel()
            NSAnimationContext.runAnimationGroup { ctx in
                ctx.duration = 0.12
                panel.animator().alphaValue = 1
            }
            installOutsideClickMonitor()
        } else {
            panel.orderOut(nil)
            if let m = outsideClickMonitor { NSEvent.removeMonitor(m) }
            outsideClickMonitor = nil
        }
    }

    /// Clicking anywhere else closes the panel, unless you're in the middle of writing something.
    private func installOutsideClickMonitor() {
        guard outsideClickMonitor == nil else { return }
        outsideClickMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                let p = NSEvent.mouseLocation
                if self.panel.frame.contains(p) || self.pill.frame.contains(p) { return }
                if self.model.draft.isEmpty && !self.model.dictation.isRecording { self.model.panel = nil }
            }
        }
    }

    private func checkHover() {
        guard pill.isVisible, dragStart == nil else { return }
        let inside = pill.frame.insetBy(dx: -6, dy: -6).contains(NSEvent.mouseLocation)
        if inside {
            outsideSince = nil
            if !model.pillExpanded { model.pillExpanded = true }
        } else if model.pillExpanded {
            if outsideSince == nil { outsideSince = Date() }
            if let since = outsideSince, Date().timeIntervalSince(since) > 0.7 {
                model.pillExpanded = false
                outsideSince = nil
            }
        }
    }

    // MARK: Dragging (by the lantern)

    private func dragMoved() {
        let mouse = NSEvent.mouseLocation
        if dragStart == nil { dragStart = (mouse, pill.frame.origin) }
        guard let start = dragStart else { return }
        pill.setFrameOrigin(NSPoint(x: start.origin.x + mouse.x - start.mouse.x, y: start.origin.y + mouse.y - start.mouse.y))
        layoutPanel()
    }

    private func dragEnded() {
        dragStart = nil
        let visible = screen.visibleFrame
        edge = pill.frame.midX < visible.midX ? .left : .right
        topFraction = min(max((pill.frame.maxY - visible.minY) / visible.height, 0.1), 1)
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.18
            self.layout()
        }
    }
}
