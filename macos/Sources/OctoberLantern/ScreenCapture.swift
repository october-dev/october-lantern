import AppKit
import ScreenCaptureKit

/// A screenshot of the screen you're working on, to give a new session context. Taken only when
/// "Include a screenshot" is on in New Session, of the display under the pointer, without
/// Lantern's own windows. Saved on this Mac (the screenshots folder, cleared after a week); the
/// agent gets its path in the first message.
@MainActor
final class ScreenCapture: ObservableObject {
    static let shared = ScreenCapture()

    @Published private(set) var image: CGImage?
    @Published private(set) var capturing = false
    /// Why the last capture failed; `needsPermission` when macOS hasn't allowed screen capture.
    @Published private(set) var error: String?
    @Published private(set) var needsPermission = false

    /// Where screenshots are kept; the engine only accepts screenshots from here.
    static var folder: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/October Lantern/screenshots", isDirectory: true)
    }

    /// Longest side of the saved image, in pixels: plenty to read text, small for the model.
    private static let maxSide = 2048.0

    func capture() async {
        guard !capturing else { return }
        guard CGPreflightScreenCaptureAccess() else {
            needsPermission = true
            error = "Lantern needs Screen Recording permission to include a screenshot."
            image = nil
            return
        }
        capturing = true
        defer { capturing = false }
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            let mouse = NSEvent.mouseLocation
            let screen = NSScreen.screens.first { NSMouseInRect(mouse, $0.frame, false) } ?? NSScreen.main
            let number = screen?.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID
            guard let display = content.displays.first(where: { $0.displayID == number }) ?? content.displays.first else {
                throw CaptureError.noDisplay
            }
            let mine = content.applications.filter { $0.processID == ProcessInfo.processInfo.processIdentifier }
            let filter = SCContentFilter(display: display, excludingApplications: mine, exceptingWindows: [])
            let config = SCStreamConfiguration()
            let scale = min(screen?.backingScaleFactor ?? 2, Self.maxSide / Double(max(display.width, display.height)))
            config.width = Int(Double(display.width) * scale)
            config.height = Int(Double(display.height) * scale)
            config.showsCursor = false
            image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
            error = nil
            needsPermission = false
        } catch {
            image = nil
            self.error = "Couldn't take the screenshot: \(error.localizedDescription)"
        }
    }

    func discard() {
        image = nil
        error = nil
    }

    /// Asks macOS for Screen Recording permission (it shows its prompt once, then only System
    /// Settings can change it; Lantern may need to be reopened afterwards).
    func requestPermission() {
        if !CGRequestScreenCaptureAccess() {
            NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")!)
        }
    }

    /// Writes the current screenshot to the screenshots folder (0600) and returns its path.
    /// Screenshots older than a week are removed at the same time.
    func save() -> URL? {
        guard let image else { return nil }
        let fm = FileManager.default
        try? fm.createDirectory(at: Self.folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        removeOld()
        let stamp = DateFormatter()
        stamp.dateFormat = "yyyyMMdd-HHmmss"
        let url = Self.folder.appendingPathComponent("screen-\(stamp.string(from: Date()))-\(UUID().uuidString.prefix(4)).png")
        guard let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]),
              fm.createFile(atPath: url.path, contents: png, attributes: [.posixPermissions: 0o600]) else { return nil }
        return url
    }

    private func removeOld() {
        let fm = FileManager.default
        let cutoff = Date().addingTimeInterval(-7 * 24 * 3600)
        let files = (try? fm.contentsOfDirectory(at: Self.folder, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        for f in files where ((try? f.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate ?? .distantFuture) < cutoff {
            try? fm.removeItem(at: f)
        }
    }

    private enum CaptureError: LocalizedError {
        case noDisplay
        var errorDescription: String? { "no display found" }
    }
}
