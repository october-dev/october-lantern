import SwiftUI

/// Multiplayer: teammates' agents in Lantern, across Lantern and October Desktop. Not built yet,
/// so the card is locked; its little canvas of drifting teammate cursors shows what it will feel
/// like. No network calls.
struct TeamCard: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            CursorCanvas()
                .frame(height: 140)
            HStack(alignment: .top, spacing: 8) {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        Text("Multiplayer").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                        Label("Coming soon", systemImage: "lock.fill")
                            .font(.system(size: 9.5, weight: .semibold))
                            .foregroundStyle(Theme.amber)
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Capsule().fill(Theme.amber.opacity(0.15)))
                    }
                    Text("See and message your teammates' agents, live, across Lantern and October Desktop.")
                        .font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
            }
            Button("Invite teammates") {}
                .buttonStyle(SecondaryButtonStyle())
                .disabled(true)
                .help("Multiplayer is coming soon")
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Theme.hairline))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Multiplayer, coming soon. See and message your teammates' agents, live, across Lantern and October Desktop.")
    }
}

/// A dotted canvas with teammates' cursors drifting over it, Figma-style, and their avatars in the
/// corner. Motion is deterministic (sine loops), stops when the card is off screen, and holds still
/// when Reduce Motion is on.
private struct CursorCanvas: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var visible = false

    struct Mate {
        let name: String
        let color: Color
        let avatar: Int
        /// Loop shape: centre, radius and speed, in fractions of the canvas.
        let cx, cy, rx, ry, speed, phase: Double
    }

    static let mates: [Mate] = [
        Mate(name: "Maya", color: Color(red: 0.98, green: 0.45, blue: 0.55), avatar: 23, cx: 0.28, cy: 0.38, rx: 0.16, ry: 0.18, speed: 0.55, phase: 0),
        Mate(name: "Leo", color: Color(red: 0.36, green: 0.62, blue: 1.0), avatar: 29, cx: 0.66, cy: 0.30, rx: 0.14, ry: 0.14, speed: 0.42, phase: 2.1),
        Mate(name: "Aru", color: Color(red: 0.35, green: 0.82, blue: 0.55), avatar: 34, cx: 0.52, cy: 0.66, rx: 0.22, ry: 0.12, speed: 0.36, phase: 4.0),
        Mate(name: "Sam", color: Color(red: 0.98, green: 0.72, blue: 0.25), avatar: 41, cx: 0.78, cy: 0.58, rx: 0.10, ry: 0.12, speed: 0.6, phase: 1.2),
    ]

    var body: some View {
        TimelineView(.animation(minimumInterval: 1 / 30, paused: reduceMotion || !visible)) { context in
            let t = reduceMotion ? 1.3 : context.date.timeIntervalSinceReferenceDate
            GeometryReader { geo in
                ZStack(alignment: .topLeading) {
                    Dots().fill(Theme.ink.opacity(0.10))
                    // A few "agents" on the canvas, for the cursors to hover over.
                    node("claude-2", x: 0.18, y: 0.62, geo: geo)
                    node("codex-1", x: 0.46, y: 0.22, geo: geo)
                    ForEach(Self.mates, id: \.name) { m in
                        let angle = t * m.speed + m.phase
                        let x = (m.cx + m.rx * sin(angle)) * geo.size.width
                        let y = (m.cy + m.ry * sin(angle * 1.7 + 0.8)) * geo.size.height
                        CursorBadge(mate: m).position(x: x + 34, y: y + 10)
                    }
                    avatarStack.position(x: geo.size.width - 50, y: 18)
                }
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))
        .background(RoundedRectangle(cornerRadius: 11, style: .continuous).fill(Color.black.opacity(0.18)))
        .overlay(RoundedRectangle(cornerRadius: 11, style: .continuous).strokeBorder(Theme.hairline))
        .onAppear { visible = true }
        .onDisappear { visible = false }
        .accessibilityHidden(true)
    }

    private func node(_ title: String, x: Double, y: Double, geo: GeometryProxy) -> some View {
        Text("@\(title)")
            .font(.system(size: 10, weight: .medium, design: .rounded))
            .foregroundStyle(Theme.ink.opacity(0.7))
            .padding(.horizontal, 8).padding(.vertical, 4)
            .background(RoundedRectangle(cornerRadius: 7).fill(Theme.faint))
            .overlay(RoundedRectangle(cornerRadius: 7).strokeBorder(Theme.hairline))
            .position(x: x * geo.size.width + 30, y: y * geo.size.height)
    }

    private var avatarStack: some View {
        HStack(spacing: -8) {
            ForEach(Self.mates, id: \.name) { m in
                Group {
                    if let img = Assets.avatar(m.avatar) {
                        Image(nsImage: img).resizable().aspectRatio(contentMode: .fill)
                    } else {
                        m.color
                    }
                }
                .frame(width: 22, height: 22)
                .clipShape(Circle())
                .overlay(Circle().strokeBorder(Color.black.opacity(0.55), lineWidth: 1.5))
            }
        }
    }
}

/// A Figma-style cursor: a colored arrow with its owner's name (and face) in a pill.
private struct CursorBadge: View {
    let mate: CursorCanvas.Mate

    var body: some View {
        ZStack(alignment: .topLeading) {
            Arrow().fill(mate.color).frame(width: 13, height: 17)
                .overlay(Arrow().stroke(Color.white.opacity(0.9), lineWidth: 1).frame(width: 13, height: 17))
                .shadow(color: .black.opacity(0.35), radius: 1.5, y: 1)
            HStack(spacing: 4) {
                if let img = Assets.avatar(mate.avatar) {
                    Image(nsImage: img).resizable().aspectRatio(contentMode: .fill).frame(width: 13, height: 13).clipShape(Circle())
                }
                Text(mate.name).font(.system(size: 10, weight: .semibold)).foregroundStyle(.white)
            }
            .padding(.leading, 3).padding(.trailing, 7).padding(.vertical, 2.5)
            .background(Capsule().fill(mate.color))
            .offset(x: 11, y: 14)
        }
        .frame(width: 80, height: 34, alignment: .topLeading)
    }
}

/// The classic pointer arrow.
private struct Arrow: Shape {
    func path(in r: CGRect) -> Path {
        var p = Path()
        p.move(to: CGPoint(x: r.minX, y: r.minY))
        p.addLine(to: CGPoint(x: r.minX, y: r.maxY * 0.92))
        p.addLine(to: CGPoint(x: r.width * 0.30, y: r.height * 0.70))
        p.addLine(to: CGPoint(x: r.width * 0.52, y: r.maxY))
        p.addLine(to: CGPoint(x: r.width * 0.68, y: r.height * 0.93))
        p.addLine(to: CGPoint(x: r.width * 0.47, y: r.height * 0.64))
        p.addLine(to: CGPoint(x: r.maxX, y: r.height * 0.62))
        p.closeSubpath()
        return p
    }
}

/// A dotted grid, like a design canvas.
private struct Dots: Shape {
    func path(in r: CGRect) -> Path {
        var p = Path()
        let step: CGFloat = 12
        var y = step / 2
        while y < r.height {
            var x = step / 2
            while x < r.width {
                p.addEllipse(in: CGRect(x: x - 0.8, y: y - 0.8, width: 1.6, height: 1.6))
                x += step
            }
            y += step
        }
        return p
    }
}
