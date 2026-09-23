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
        hasShadow = true
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
        get { Edge(rawValue: UserDefaults.standard.string(forKey: "pillEdge") ?? "") ?? .right }
        set { UserDefaults.standard.set(newValue.rawValue, forKey: "pillEdge") }
    }
    /// Vertical centre of the pill as a fraction of the screen's visible height.
    private var yFraction: CGFloat {
        get { UserDefaults.standard.object(forKey: "pillY") as? CGFloat ?? 0.5 }
        set { UserDefaults.standard.set(newValue, forKey: "pillY") }
    }

    init(model: AppModel) {
        self.model = model
        pillHost = FirstClickHostingView(rootView: PillView(
            model: model, dictation: model.dictation,
            onDrag: { [weak self] _ in self?.dragMoved() },
            onDragEnd: { [weak self] in self?.dragEnded() },
            onMenu: { [weak self] in
                guard let self else { return }
                self.onMenu?(self.pillHost)
            }
        ))
        pill.contentView = pillHost
        panelHost = FirstClickHostingView(rootView: PanelView(model: model, dictation: model.dictation))
        panel.contentView = panelHost

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
        NotificationCenter.default.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.layout() }
        }
    }

    var pillVisible: Bool { pill.isVisible }

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
        let centerY = visible.minY + visible.height * yFraction
        let y = min(max(centerY - size.height / 2, visible.minY + 8), visible.maxY - size.height - 8)
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

    // MARK: Dragging (by the grip at the bottom of the pill)

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
        yFraction = min(max((pill.frame.midY - visible.minY) / visible.height, 0.05), 0.95)
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.18
            self.layout()
        }
    }
}
