import AppKit
import SwiftUI

enum Theme {
    static let amber = Color(red: 0.98, green: 0.72, blue: 0.27)
    static let green = Color(red: 0.36, green: 0.80, blue: 0.51)
    static let red = Color(red: 0.93, green: 0.36, blue: 0.36)
    static let ink = Color.white
    static let muted = Color.white.opacity(0.55)
    static let faint = Color.white.opacity(0.08)
    static let stroke = Color.white.opacity(0.14)

    static func color(for state: AgentState) -> Color {
        switch state {
        case .needsInput, .waiting: amber
        case .working: green
        case .idle, .unknown: Color.white.opacity(0.35)
        }
    }
}

/// Images shipped in the app bundle (Contents/Resources), or found in the repo during development.
enum Assets {
    private static var cache: [String: NSImage] = [:]

    private static func url(_ path: String) -> URL? {
        if let res = Bundle.main.resourceURL?.appendingPathComponent(path), FileManager.default.fileExists(atPath: res.path) {
            return res
        }
        var dir = URL(fileURLWithPath: CommandLine.arguments[0]).resolvingSymlinksInPath().deletingLastPathComponent()
        for _ in 0..<8 {
            for candidate in [dir.appendingPathComponent("macos/Resources/\(path)"), dir.appendingPathComponent(path)] {
                if FileManager.default.fileExists(atPath: candidate.path) { return candidate }
            }
            dir.deleteLastPathComponent()
        }
        return nil
    }

    static func image(_ path: String) -> NSImage? {
        if let hit = cache[path] { return hit }
        guard let url = url(path), let img = NSImage(contentsOf: url) else { return nil }
        cache[path] = img
        return img
    }

    static var logo: NSImage? { image("logo.png") }

    static func harness(_ kind: AgentKind) -> NSImage? { image("harness/\(kind.rawValue).png") }
}

/// Behind-window blur, the base of both panels.
struct Glass: NSViewRepresentable {
    var material: NSVisualEffectView.Material = .hudWindow

    func makeNSView(context: Context) -> NSVisualEffectView {
        let v = NSVisualEffectView()
        v.material = material
        v.blendingMode = .behindWindow
        v.state = .active
        v.appearance = NSAppearance(named: .darkAqua)
        return v
    }

    func updateNSView(_ v: NSVisualEffectView, context: Context) {
        v.material = material
    }
}

/// An agent's harness logo, with a dot for its state.
struct AgentBadge: View {
    let agent: Agent
    var size: CGFloat = 26

    var body: some View {
        Group {
            if let img = Assets.harness(agent.kind) {
                Image(nsImage: img).resizable().interpolation(.high).aspectRatio(contentMode: .fit)
                    .clipShape(RoundedRectangle(cornerRadius: size * 0.22, style: .continuous))
            } else {
                RoundedRectangle(cornerRadius: size * 0.22, style: .continuous).fill(Theme.faint)
                    .overlay(
                        Text(agent.kind.displayName.prefix(1))
                            .font(.system(size: size * 0.46, weight: .bold, design: .rounded))
                            .foregroundStyle(Theme.ink)
                    )
            }
        }
        .frame(width: size, height: size)
        .overlay(alignment: .bottomTrailing) {
            Circle()
                .fill(Theme.color(for: agent.state))
                .frame(width: size * 0.34, height: size * 0.34)
                .overlay(Circle().stroke(Color.black.opacity(0.7), lineWidth: 1.5))
                .offset(x: 2, y: 2)
        }
        .help(agent.kind.displayName)
    }
}

struct StateChip: View {
    let state: AgentState

    var body: some View {
        HStack(spacing: 5) {
            Circle().fill(Theme.color(for: state)).frame(width: 6, height: 6)
            Text(state.label).font(.system(size: 11, weight: .medium))
        }
        .foregroundStyle(state.wantsYou ? Theme.amber : Theme.muted)
    }
}

func timeAgo(_ date: Date?) -> String {
    guard let date else { return "" }
    let s = max(0, Int(Date().timeIntervalSince(date)))
    switch s {
    case ..<60: return "now"
    case ..<3600: return "\(s / 60)m"
    case ..<86400: return "\(s / 3600)h"
    default: return "\(s / 86400)d"
    }
}

/// Agent text often contains Markdown; render inline styles and keep line breaks.
func markdown(_ text: String) -> AttributedString {
    (try? AttributedString(
        markdown: text,
        options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace, failurePolicy: .returnPartiallyParsedIfPossible)
    )) ?? AttributedString(text)
}

/// A hosting view that takes the first click, so buttons work without focusing the panel first.
final class FirstClickHostingView<Content: View>: NSHostingView<Content> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}
