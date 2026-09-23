// Builds macos/Resources/AppIcon.icns from logo.png: the lantern on a dark rounded tile.
// Run: swift scripts/make-icon.swift   (from the repo root)
import AppKit

let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
guard let logo = NSImage(contentsOf: root.appendingPathComponent("logo.png")) else { fatalError("logo.png not found") }
let iconset = root.appendingPathComponent("build/AppIcon.iconset")
try? FileManager.default.removeItem(at: iconset)
try! FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)

func render(_ px: Int) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = CGFloat(px)
    // Apple's grid: the tile is ~80% of the canvas, centred.
    let tile = NSRect(x: s * 0.1, y: s * 0.1, width: s * 0.8, height: s * 0.8)
    let path = NSBezierPath(roundedRect: tile, xRadius: s * 0.18, yRadius: s * 0.18)
    NSGradient(colors: [NSColor(white: 0.16, alpha: 1), NSColor(white: 0.07, alpha: 1)])!.draw(in: path, angle: -90)
    NSColor(white: 1, alpha: 0.08).setStroke()
    path.lineWidth = max(1, s * 0.004)
    path.stroke()
    logo.draw(in: tile.insetBy(dx: s * 0.1, dy: s * 0.1))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

for size in [16, 32, 128, 256, 512] {
    try! render(size).write(to: iconset.appendingPathComponent("icon_\(size)x\(size).png"))
    try! render(size * 2).write(to: iconset.appendingPathComponent("icon_\(size)x\(size)@2x.png"))
}
let out = root.appendingPathComponent("macos/Resources/AppIcon.icns")
let p = Process()
p.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
p.arguments = ["-c", "icns", iconset.path, "-o", out.path]
try! p.run()
p.waitUntilExit()
print(out.path)
