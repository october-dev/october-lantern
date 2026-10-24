import AppKit
import UniformTypeIdentifiers

/// Images you add to Ask Lantern (pasted, dropped or picked): saved as private files in the
/// screenshots folder, the one place the engine hands an agent images from. The agent gets their
/// paths; October's answer gets the first one.
enum ImageAttachments {
    /// At most this many on one message.
    static let limit = 6
    /// Kept as they are; anything else (HEIC, TIFF, a pasted picture) is saved as PNG.
    private static let kept: Set<String> = ["png", "jpg", "jpeg", "gif", "webp"]
    /// Bigger files are re-saved smaller.
    private static let keptBytes = 10_000_000
    /// The longest side of an image Lantern re-saves.
    private static let longestSide: CGFloat = 2400

    /// The images on the pasteboard, saved: image files (copied in Finder), else a picture
    /// (a screenshot copied with Control, or Copy Image in a browser). Text wins over a picture,
    /// since apps like Excel and Word copy a picture of the text along with it.
    static func fromPasteboard(_ pasteboard: NSPasteboard = .general) -> [URL]? {
        let files = (pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL]) ?? []
        if !files.isEmpty {
            let images = files.filter(isImage)
            return images.isEmpty ? nil : images.compactMap(save(file:))
        }
        if pasteboard.string(forType: .string) != nil { return nil }
        guard let image = NSImage(pasteboard: pasteboard) else { return nil }
        return save(image).map { [$0] }
    }

    static func isImage(_ url: URL) -> Bool {
        UTType(filenameExtension: url.pathExtension)?.conforms(to: .image) ?? false
    }

    /// Copies an image file into the screenshots folder (re-saved as PNG when it's a kind or
    /// size agents may not read).
    static func save(file: URL) -> URL? {
        let ext = file.pathExtension.lowercased()
        let size = (try? file.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? .max
        if kept.contains(ext), size <= keptBytes, let data = try? Data(contentsOf: file) {
            return write(data, ext: ext == "jpeg" ? "jpg" : ext)
        }
        return NSImage(contentsOf: file).flatMap(save)
    }

    /// Saves a picture as PNG, scaled down when it's very large.
    static func save(_ image: NSImage) -> URL? {
        guard let png = png(image) else { return nil }
        return write(png, ext: "png")
    }

    /// The image as JPEG for October's answer (it takes one, up to 4 MB).
    static func jpegForOctober(_ url: URL) -> Data? {
        guard let image = NSImage(contentsOf: url), let cg = scaled(image, longest: 1600) else { return nil }
        return NSBitmapImageRep(cgImage: cg).representation(using: .jpeg, properties: [.compressionFactor: 0.8])
            .flatMap { $0.count <= 4_000_000 ? $0 : nil }
    }

    private static func png(_ image: NSImage) -> Data? {
        guard let cg = scaled(image, longest: longestSide) else { return nil }
        return NSBitmapImageRep(cgImage: cg).representation(using: .png, properties: [:])
    }

    private static func scaled(_ image: NSImage, longest: CGFloat) -> CGImage? {
        guard let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { return nil }
        let side = CGFloat(max(cg.width, cg.height))
        guard side > longest else { return cg }
        let scale = longest / side
        let width = Int(CGFloat(cg.width) * scale), height = Int(CGFloat(cg.height) * scale)
        guard let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return cg }
        context.interpolationQuality = .high
        context.draw(cg, in: CGRect(x: 0, y: 0, width: width, height: height))
        return context.makeImage() ?? cg
    }

    private static func write(_ data: Data, ext: String) -> URL? {
        let fm = FileManager.default
        try? fm.createDirectory(at: ScreenCapture.folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let stamp = DateFormatter()
        stamp.dateFormat = "yyyyMMdd-HHmmss"
        let url = ScreenCapture.folder.appendingPathComponent("added-\(stamp.string(from: Date()))-\(UUID().uuidString.prefix(6)).\(ext)")
        return fm.createFile(atPath: url.path, contents: data, attributes: [.posixPermissions: 0o600]) ? url : nil
    }
}
