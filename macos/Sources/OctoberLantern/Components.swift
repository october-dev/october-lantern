import AppKit
import SwiftUI

enum Theme {
    static let amber = Color(red: 0.98, green: 0.72, blue: 0.27)
    static let green = Color(red: 0.36, green: 0.80, blue: 0.51)
    static let red = Color(red: 0.93, green: 0.36, blue: 0.36)
    static let ink = Color.white
    static let muted = Color.white.opacity(0.55)
    static let faint = Color.white.opacity(0.07)
    static let hairline = Color.white.opacity(0.10)
    static let stroke = Color.white.opacity(0.14)

    static let blue = Color(red: 0.30, green: 0.58, blue: 1.0)

    /// Blue is working, amber needs you, green is done (its turn is over).
    static func color(for state: AgentState) -> Color {
        switch state {
        case .needsInput: amber
        case .waiting: green
        case .working: blue
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

    /// October's avatars (Resources/avatars/21.webp … 44.webp).
    static func avatar(_ n: Int) -> NSImage? { image("avatars/\(n).webp") }

    static func harness(_ kind: AgentKind) -> NSImage? { image("harness/\(kind.rawValue).png") }
}

/// The window-level glass that the pill and the panel sit in. On macOS 26+ this is Apple's Liquid
/// Glass (`NSGlassEffectView`); earlier versions get a behind-window blur. It's an AppKit container
/// (rather than a SwiftUI background) because SwiftUI can't clip these views: shaped from SwiftUI,
/// they leak a rectangle at the corners. `tint` darkens the glass so white text stays readable.
final class GlassContainer: NSView {
    private let radius: CGFloat?  // nil = capsule
    private let glass: NSView

    init(content: NSView, cornerRadius: CGFloat?, tint: Double) {
        radius = cornerRadius
        if #available(macOS 26.0, *) {
            let g = NSGlassEffectView()
            g.style = .regular
            g.tintColor = tint > 0 ? NSColor.black.withAlphaComponent(tint) : nil
            g.contentView = content
            glass = g
        } else {
            let blur = NSVisualEffectView()
            blur.material = .hudWindow
            blur.blendingMode = .behindWindow
            blur.state = .active
            blur.wantsLayer = true
            blur.layer?.masksToBounds = true
            blur.layer?.cornerCurve = .continuous
            content.frame = blur.bounds
            content.autoresizingMask = [.width, .height]
            blur.addSubview(content)
            glass = blur
        }
        super.init(frame: .zero)
        glass.appearance = NSAppearance(named: .darkAqua)
        glass.frame = bounds
        glass.autoresizingMask = [.width, .height]
        addSubview(glass)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layout() {
        super.layout()
        let r = radius ?? min(bounds.width, bounds.height) / 2
        if #available(macOS 26.0, *), let g = glass as? NSGlassEffectView {
            g.cornerRadius = r
        } else {
            glass.layer?.cornerRadius = r
        }
    }

    override func setFrameSize(_ size: NSSize) {
        super.setFrameSize(size)
        needsLayout = true
    }
}

/// The glassmorphism finish: a light rim that fades from top to bottom, and a soft sheen along
/// the top edge, over the glass.
struct GlassRim<S: InsettableShape>: View {
    let shape: S

    var body: some View {
        ZStack {
            shape.strokeBorder(
                LinearGradient(colors: [.white.opacity(0.42), .white.opacity(0.10), .white.opacity(0.18)],
                               startPoint: .top, endPoint: .bottom),
                lineWidth: 1
            )
            shape.fill(
                LinearGradient(colors: [.white.opacity(0.10), .clear], startPoint: .top, endPoint: .center)
            )
            .allowsHitTesting(false)
        }
    }
}

extension View {
    /// The glass rim over the window's glass (see `GlassContainer`), plus a dark layer so the
    /// white icons and text stay readable when the glass sits over a white or light window.
    func glassSurface<S: InsettableShape>(_ shape: S) -> some View {
        background(shape.fill(Color.black.opacity(0.35)))
            .clipShape(shape)
            .overlay(GlassRim(shape: shape))
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

/// Whether text can be selected here. Off inside a card that opens its conversation when
/// clicked: selectable text keeps a click for itself (it may start a selection), so a click on the
/// card's words wouldn't open it. Selecting and copying belong to the conversation it opens.
private struct SelectableTextKey: EnvironmentKey {
    static let defaultValue = true
}

extension EnvironmentValues {
    var selectableText: Bool {
        get { self[SelectableTextKey.self] }
        set { self[SelectableTextKey.self] = newValue }
    }
}

private struct Selectable: ViewModifier {
    @Environment(\.selectableText) private var selectable

    @ViewBuilder func body(content: Content) -> some View {
        if selectable { content.textSelection(.enabled) } else { content.textSelection(.disabled) }
    }
}

extension View {
    /// Text selection, unless a clickable card around it turned it off (see `selectableText`).
    func selectable() -> some View { modifier(Selectable()) }
}

/// Agent text with its Markdown: paragraphs (inline Markdown, line breaks kept), headings,
/// bulleted and numbered lists, and fenced code in a monospaced face. It's one text, so a drag
/// selects across all of it (separate texts each keep their own selection, so a drag stopped at
/// the first paragraph or list); each code block has its own Copy below it.
struct MarkdownBlocks: View {
    let text: String
    var ink: Color = Theme.ink

    enum Block: Hashable {
        case paragraph(String)
        case heading(String)
        case item(marker: String, text: String)
        case code(String)
    }

    var body: some View {
        let blocks = Self.parse(text)
        let code = blocks.compactMap { block -> String? in
            if case .code(let c) = block { return c }
            return nil
        }
        VStack(alignment: .leading, spacing: 6) {
            Text(Self.attributed(blocks))
                .font(.system(size: 12.5)).foregroundStyle(ink)
                .selectable()
                .fixedSize(horizontal: false, vertical: true)
            if !code.isEmpty {
                // Wraps onto more lines rather than running past a narrow panel: every block
                // keeps its own Copy.
                FlowRow(spacing: 12, lineSpacing: 4) {
                    ForEach(Array(code.enumerated()), id: \.offset) { i, c in
                        CopyButton(text: c, title: code.count == 1 ? "Copy code" : "Copy code \(i + 1)")
                    }
                }
            }
        }
    }

    /// The blocks as one string: list items on their own lines, other blocks a blank line apart
    /// (close to how it was written; the whole-reply Copy gives the Markdown itself). Code keeps
    /// its indentation.
    static func attributed(_ blocks: [Block]) -> AttributedString {
        var out = AttributedString()
        var previous: Block?
        for block in blocks {
            if let previous {
                let list = { (b: Block) -> Bool in
                    if case .item = b { return true }
                    return false
                }
                out += AttributedString(list(previous) && list(block) ? "\n" : "\n\n")
            }
            switch block {
            case .paragraph(let t):
                out += markdown(t)
            case .heading(let t):
                var h = markdown(t)
                h[AttributeScopes.SwiftUIAttributes.FontAttribute.self] = Font.system(size: 13, weight: .semibold)
                out += h
            case .item(let marker, let t):
                var m = AttributedString(marker + " ")
                m[AttributeScopes.SwiftUIAttributes.ForegroundColorAttribute.self] = Theme.muted
                out += m
                out += markdown(t)
            case .code(let c):
                var code = AttributedString(c)
                code[AttributeScopes.SwiftUIAttributes.FontAttribute.self] = Font.system(size: 11.5, design: .monospaced)
                code[AttributeScopes.SwiftUIAttributes.BackgroundColorAttribute.self] = Color.black.opacity(0.3)
                out += code
            }
            previous = block
        }
        return out
    }

    static func parse(_ text: String) -> [Block] {
        var blocks: [Block] = []
        var paragraph: [String] = []
        var code: [String]?
        // The row just shown, while it may still turn out to be a table's header.
        var tableRow: String?
        func flush() {
            if !paragraph.isEmpty { blocks.append(.paragraph(paragraph.joined(separator: "\n"))) }
            paragraph = []
        }
        for line in text.components(separatedBy: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            let row = code == nil ? tableCells(trimmed) : nil
            defer { if code == nil { tableRow = row.map { $0.joined(separator: " · ") } } }
            if let row {
                // A Markdown table, too wide for the panel as a grid: its header as a line of its
                // own, then a row per line with its cells " · " apart.
                flush()
                if row.allSatisfy({ !$0.isEmpty && $0.allSatisfy { "-:".contains($0) } }) {
                    if let header = tableRow, case .item(_, let last)? = blocks.last, last == header {
                        blocks[blocks.count - 1] = .paragraph(header)
                    }
                } else {
                    blocks.append(.item(marker: "•", text: row.joined(separator: " · ")))
                }
                continue
            }
            if var lines = code {
                if trimmed.hasPrefix("```") {
                    blocks.append(.code(lines.joined(separator: "\n")))
                    code = nil
                } else {
                    lines.append(line)
                    code = lines
                }
                continue
            }
            if trimmed.hasPrefix("```") {
                flush()
                code = []
            } else if trimmed.isEmpty {
                flush()
            } else if trimmed.hasPrefix("#") {
                flush()
                blocks.append(.heading(String(trimmed.drop(while: { $0 == "#" })).trimmingCharacters(in: .whitespaces)))
            } else if trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ") || trimmed.hasPrefix("• ") {
                flush()
                blocks.append(.item(marker: "•", text: String(trimmed.dropFirst(2))))
            } else if let dot = trimmed.firstIndex(of: "."), trimmed[..<dot].allSatisfy(\.isNumber), !trimmed[..<dot].isEmpty,
                      trimmed[trimmed.index(after: dot)...].hasPrefix(" ") {
                flush()
                blocks.append(.item(marker: String(trimmed[...dot]), text: String(trimmed[trimmed.index(dot, offsetBy: 2)...])))
            } else {
                paragraph.append(line)
            }
        }
        // An unclosed fence (a message cut off mid-block) still shows as code.
        if let lines = code { blocks.append(.code(lines.joined(separator: "\n"))) }
        flush()
        return blocks
    }

    /// A table row's cells (`| a | b |`), or nil for any other line.
    static func tableCells(_ line: String) -> [String]? {
        guard line.count > 1, line.hasPrefix("|"), line.hasSuffix("|") else { return nil }
        return line.dropFirst().dropLast().components(separatedBy: "|").map { $0.trimmingCharacters(in: .whitespaces) }
    }
}

/// Copies `text` as it is, and says so for a moment.
struct CopyButton: View {
    let text: String
    var title = "Copy"
    @State private var copied = false

    var body: some View {
        Button {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            copied = true
            Task { try? await Task.sleep(for: .seconds(1.5)); copied = false }
        } label: {
            Label(copied ? "Copied" : title, systemImage: copied ? "checkmark" : "doc.on.doc")
                .font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
        }
        .buttonStyle(.plain)
        .help(title)
    }
}

/// Its views in a row, onto the next line when the row is full.
struct FlowRow: Layout {
    var spacing: CGFloat = 8
    var lineSpacing: CGFloat = 4

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let frames = Self.frames(subviews.map { $0.sizeThatFits(.unspecified) }, width: proposal.width ?? .infinity,
                                 spacing: spacing, lineSpacing: lineSpacing)
        return CGSize(width: frames.map(\.maxX).max() ?? 0, height: frames.map(\.maxY).max() ?? 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let sizes = subviews.map { $0.sizeThatFits(.unspecified) }
        for (view, frame) in zip(subviews, Self.frames(sizes, width: bounds.width, spacing: spacing, lineSpacing: lineSpacing)) {
            view.place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY), proposal: ProposedViewSize(frame.size))
        }
    }

    /// Where each size goes: left to right, a new line when the next one doesn't fit `width`.
    static func frames(_ sizes: [CGSize], width: CGFloat, spacing: CGFloat, lineSpacing: CGFloat) -> [CGRect] {
        var frames: [CGRect] = []
        var x: CGFloat = 0, y: CGFloat = 0, line: CGFloat = 0
        for size in sizes {
            if x > 0, x + size.width > width {
                x = 0
                y += line + lineSpacing
                line = 0
            }
            frames.append(CGRect(origin: CGPoint(x: x, y: y), size: size))
            x += size.width + spacing
            line = max(line, size.height)
        }
        return frames
    }
}

/// A hosting view that takes the first click, so buttons work without focusing the panel first.
final class FirstClickHostingView<Content: View>: NSHostingView<Content> {
    var onSizeChange: (() -> Void)?
    override func invalidateIntrinsicContentSize() {
        super.invalidateIntrinsicContentSize()
        onSizeChange?()
    }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}
