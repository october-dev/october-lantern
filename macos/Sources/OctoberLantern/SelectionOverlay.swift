import AppKit

/// The first step of Point & Ask: the screen dims and you drag a box around what you mean, like
/// taking a screenshot of an area. Moving without dragging highlights the window under the
/// pointer, and a click picks that window. Esc cancels.
///
/// There's one overlay window per screen. `onDone` gets the chosen rectangle in global screen
/// coordinates (AppKit's, bottom-left origin) and the screen it's on, or nil when cancelled.
@MainActor
final class SelectionOverlay {
    private var windows: [OverlayWindow] = []
    private var onDone: ((NSRect, NSScreen)?) -> Void = { _ in }

    var isOpen: Bool { !windows.isEmpty }

    func begin(onDone: @escaping ((NSRect, NSScreen)?) -> Void) {
        end()
        self.onDone = onDone
        let targets = Self.windowFrames()
        for screen in NSScreen.screens {
            let window = OverlayWindow(screen: screen)
            let view = SelectionView(frame: NSRect(origin: .zero, size: screen.frame.size))
            view.screenFrame = screen.frame
            view.windowFrames = targets.compactMap { $0.intersects(screen.frame) ? $0.offsetBy(dx: -screen.frame.minX, dy: -screen.frame.minY) : nil }
            view.onFinish = { [weak self] rect in
                guard let self else { return }
                let chosen = rect.map { ($0.offsetBy(dx: screen.frame.minX, dy: screen.frame.minY), screen) }
                self.end()
                self.onDone(chosen)
            }
            window.contentView = view
            window.setFrame(screen.frame, display: true)
            window.orderFrontRegardless()
            windows.append(window)
        }
        // The window under the pointer takes the keyboard (for Esc) without activating Lantern.
        let mouse = NSEvent.mouseLocation
        let front = windows.first { NSMouseInRect(mouse, $0.frame, false) } ?? windows.first
        front?.makeKey()
        if let view = front?.contentView as? SelectionView { front?.makeFirstResponder(view) }
        NSCursor.crosshair.set()
    }

    func cancel() {
        guard isOpen else { return }
        end()
        onDone(nil)
    }

    private func end() {
        for w in windows { w.orderOut(nil) }
        windows = []
        NSCursor.arrow.set()
    }

    /// Other apps' normal windows on screen, front to back, in AppKit coordinates.
    private static func windowFrames() -> [NSRect] {
        guard let info = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]],
              let primary = NSScreen.screens.first else { return [] }
        let me = ProcessInfo.processInfo.processIdentifier
        return info.compactMap { w in
            guard (w[kCGWindowLayer as String] as? Int) == 0,
                  (w[kCGWindowOwnerPID as String] as? pid_t) != me,
                  (w[kCGWindowAlpha as String] as? Double ?? 1) > 0,
                  let dict = w[kCGWindowBounds as String] as? NSDictionary,
                  let cg = CGRect(dictionaryRepresentation: dict), cg.width > 40, cg.height > 40 else { return nil }
            // Quartz window bounds have a top-left origin on the primary screen.
            return NSRect(x: cg.minX, y: primary.frame.maxY - cg.maxY, width: cg.width, height: cg.height)
        }
    }
}

private final class OverlayWindow: NSPanel {
    init(screen: NSScreen) {
        super.init(contentRect: screen.frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        level = .screenSaver
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        backgroundColor = .clear
        isOpaque = false
        hasShadow = false
        ignoresMouseEvents = false
        acceptsMouseMovedEvents = true
        isReleasedWhenClosed = false
        hidesOnDeactivate = false
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

/// Draws the dimmed screen, the box being dragged (with its size) or the window under the pointer,
/// and a hint.
private final class SelectionView: NSView {
    var screenFrame: NSRect = .zero
    var windowFrames: [NSRect] = []
    var onFinish: (NSRect?) -> Void = { _ in }

    private var start: NSPoint?
    private var current: NSPoint?
    private var hovered: NSRect?
    private var tracking: NSTrackingArea?

    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseMoved, .mouseEnteredAndExited, .activeAlways, .cursorUpdate, .inVisibleRect], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func cursorUpdate(with event: NSEvent) { NSCursor.crosshair.set() }
    override func mouseEntered(with event: NSEvent) { NSCursor.crosshair.set() }

    override func mouseMoved(with event: NSEvent) {
        NSCursor.crosshair.set()
        let p = convert(event.locationInWindow, from: nil)
        let hit = windowFrames.first { $0.contains(p) }
        if hit != hovered {
            hovered = hit
            needsDisplay = true
        }
    }

    override func mouseExited(with event: NSEvent) {
        hovered = nil
        needsDisplay = true
    }

    override func mouseDown(with event: NSEvent) {
        start = convert(event.locationInWindow, from: nil)
        current = start
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) {
        current = convert(event.locationInWindow, from: nil)
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        let box = dragged
        start = nil
        current = nil
        if let box, box.width >= 8, box.height >= 8 {
            onFinish(box)
        } else if let hovered {
            // A click without a drag picks the window under the pointer.
            onFinish(hovered.intersection(bounds))
        } else {
            onFinish(bounds)
        }
    }

    override func rightMouseDown(with event: NSEvent) { onFinish(nil) }

    override func keyDown(with event: NSEvent) {
        switch Int(event.keyCode) {
        case 53: onFinish(nil)  // Esc
        case 36, 76: onFinish(bounds)  // Return: the whole screen
        default: super.keyDown(with: event)
        }
    }

    override func cancelOperation(_ sender: Any?) { onFinish(nil) }

    private var dragged: NSRect? {
        guard let start, let current else { return nil }
        return NSRect(x: min(start.x, current.x), y: min(start.y, current.y),
                      width: abs(current.x - start.x), height: abs(current.y - start.y)).integral
    }

    override func draw(_ dirtyRect: NSRect) {
        let box = dragged.flatMap { $0.width > 1 || $0.height > 1 ? $0 : nil }
        let focus = box ?? hovered

        // Dim everything outside the box.
        let dim = NSBezierPath(rect: bounds)
        if let focus { dim.append(NSBezierPath(rect: focus)); dim.windingRule = .evenOdd }
        NSColor.black.withAlphaComponent(box == nil ? 0.28 : 0.42).setFill()
        dim.fill()

        if let box {
            // A crisp white edge with a thin dark outline so it shows on light and dark content.
            let edge = NSBezierPath(rect: box.insetBy(dx: -0.5, dy: -0.5))
            edge.lineWidth = 3
            NSColor.black.withAlphaComponent(0.35).setStroke()
            edge.stroke()
            edge.lineWidth = 1.5
            NSColor.white.setStroke()
            edge.stroke()
            for corner in [NSPoint(x: box.minX, y: box.minY), NSPoint(x: box.maxX, y: box.minY),
                           NSPoint(x: box.minX, y: box.maxY), NSPoint(x: box.maxX, y: box.maxY)] {
                let dot = NSBezierPath(ovalIn: NSRect(x: corner.x - 4, y: corner.y - 4, width: 8, height: 8))
                NSColor.white.setFill()
                dot.fill()
                NSColor.black.withAlphaComponent(0.4).setStroke()
                dot.lineWidth = 1
                dot.stroke()
            }
            label("\(Int(box.width)) × \(Int(box.height))", below: box)
        } else if let hovered {
            NSColor.white.withAlphaComponent(0.06).setFill()
            NSBezierPath(rect: hovered).fill()
            let edge = NSBezierPath(rect: hovered.insetBy(dx: 1, dy: 1))
            edge.lineWidth = 2
            edge.setLineDash([6, 4], count: 2, phase: 0)
            Self.amber.setStroke()
            edge.stroke()
        }

        if box == nil { hint() }
    }

    private static let amber = NSColor(red: 0.98, green: 0.72, blue: 0.27, alpha: 1)

    private func label(_ text: String, below box: NSRect) {
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .semibold), .foregroundColor: NSColor.white,
        ]
        let size = (text as NSString).size(withAttributes: attrs)
        var origin = NSPoint(x: box.maxX - size.width - 12, y: box.minY - size.height - 12)
        if origin.y < 4 { origin.y = box.minY + 6 }
        origin.x = max(4, origin.x)
        let pill = NSRect(x: origin.x, y: origin.y, width: size.width + 12, height: size.height + 6)
        NSColor.black.withAlphaComponent(0.75).setFill()
        NSBezierPath(roundedRect: pill, xRadius: pill.height / 2, yRadius: pill.height / 2).fill()
        (text as NSString).draw(at: NSPoint(x: pill.minX + 6, y: pill.minY + 3), withAttributes: attrs)
    }

    private func hint() {
        let text = "Drag to select an area  ·  Click a window  ·  Esc to cancel"
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 13, weight: .medium), .foregroundColor: NSColor.white,
        ]
        let size = (text as NSString).size(withAttributes: attrs)
        let pill = NSRect(x: bounds.midX - size.width / 2 - 16, y: bounds.maxY - 90, width: size.width + 32, height: size.height + 14)
        NSColor.black.withAlphaComponent(0.78).setFill()
        NSBezierPath(roundedRect: pill, xRadius: pill.height / 2, yRadius: pill.height / 2).fill()
        NSColor.white.withAlphaComponent(0.18).setStroke()
        let rim = NSBezierPath(roundedRect: pill.insetBy(dx: 0.5, dy: 0.5), xRadius: pill.height / 2, yRadius: pill.height / 2)
        rim.lineWidth = 1
        rim.stroke()
        (text as NSString).draw(at: NSPoint(x: pill.minX + 16, y: pill.minY + 7), withAttributes: attrs)
    }
}
