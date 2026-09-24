import AppKit
import ApplicationServices

/// The app you're working in, for a Task: which app, its front window's title, and the document
/// that window has open. Lantern's panels never take focus, so the frontmost app when you click
/// the pill is still yours; Lantern remembers the last one in case one of its own windows is in
/// front.
///
/// The window title and document come from the Accessibility API, which needs Accessibility
/// permission (asked the first time a Task needs it). Without it, a Task still knows the app.
@MainActor
final class AppContext: ObservableObject {
    static let shared = AppContext()

    struct Target: Equatable {
        let name: String
        let bundleId: String?
        let pid: pid_t
        let icon: NSImage?
        var windowTitle: String?
        /// The file the front window has open, when the app says.
        var document: URL?

        /// The text added to the agent's first message.
        var summary: String {
            var lines = ["I'm working in \(name) on this Mac."]
            if let windowTitle, !windowTitle.isEmpty { lines.append("Its front window is \"\(windowTitle)\".") }
            if let document { lines.append("The open document is \(document.path).") }
            return lines.joined(separator: " ")
        }
    }

    @Published private(set) var target: Target?
    @Published private(set) var trusted = AXIsProcessTrusted()
    private var lastOther: NSRunningApplication?
    private var observer: NSObjectProtocol?

    /// Apps where a Task makes no sense (you'd start a normal session instead).
    static let terminals: Set<String> = [
        "com.apple.Terminal", "com.googlecode.iterm2", "com.mitchellh.ghostty", "dev.warp.Warp-Stable", "com.cmuxterm.app",
        "net.kovidgoyal.kitty", "io.alacritty", "com.github.wez.wezterm",
    ]

    private init() {
        observer = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main
        ) { [weak self] note in
            guard let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,
                  app.processIdentifier != ProcessInfo.processInfo.processIdentifier else { return }
            MainActor.assumeIsolated { self?.lastOther = app }
        }
    }

    /// Starts watching which app is in front (call once at launch).
    func start() {
        if let front = NSWorkspace.shared.frontmostApplication, front.processIdentifier != ProcessInfo.processInfo.processIdentifier {
            lastOther = front
        }
    }

    /// Reads the app you're in right now (or were in, if a Lantern window is in front).
    func capture() {
        trusted = AXIsProcessTrusted()
        let me = ProcessInfo.processInfo.processIdentifier
        let front = NSWorkspace.shared.frontmostApplication
        guard let app = (front?.processIdentifier == me ? nil : front) ?? lastOther, !app.isTerminated else {
            target = nil
            return
        }
        var t = Target(
            name: app.localizedName ?? "this app", bundleId: app.bundleIdentifier, pid: app.processIdentifier, icon: app.icon
        )
        if trusted { (t.windowTitle, t.document) = Self.frontWindow(of: app.processIdentifier) }
        target = t
    }

    var isTerminal: Bool { target?.bundleId.map { Self.terminals.contains($0) } ?? false }

    /// Asks macOS for Accessibility permission (it shows its own prompt, pointing to System
    /// Settings); Lantern re-reads the window once it's allowed.
    func requestAccess() {
        let key = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
        trusted = AXIsProcessTrustedWithOptions([key: true] as CFDictionary)
    }

    /// The front window's title and document, through the Accessibility API.
    private static func frontWindow(of pid: pid_t) -> (String?, URL?) {
        let app = AXUIElementCreateApplication(pid)
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute as CFString, &value) == .success,
              let window = value, CFGetTypeID(window) == AXUIElementGetTypeID() else { return (nil, nil) }
        let element = window as! AXUIElement
        func string(_ attribute: String) -> String? {
            var v: CFTypeRef?
            guard AXUIElementCopyAttributeValue(element, attribute as CFString, &v) == .success else { return nil }
            return v as? String
        }
        let title = string(kAXTitleAttribute)
        // AXDocument is a file URL string for document-based apps (Preview, Pages, TextEdit...).
        let document = string(kAXDocumentAttribute).flatMap { URL(string: $0) }.flatMap { $0.isFileURL ? $0 : nil }
        return (title, document)
    }
}
