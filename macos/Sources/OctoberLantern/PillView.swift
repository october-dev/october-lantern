import SwiftUI

/// The always-on bar: a lantern that lights up when agents want you, the agents themselves,
/// and quick actions.
struct PillView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var dictation: Dictation
    var onDrag: (DragGesture.Value) -> Void
    var onDragEnd: () -> Void
    var onMenu: () -> Void

    private let maxDots = 6

    var body: some View {
        VStack(spacing: 10) {
            LanternButton(count: model.badgeCount, working: model.anyWorking, open: model.panel == .inbox) {
                model.toggle(.inbox)
            }

            if !model.agents.isEmpty {
                VStack(spacing: 6) {
                    ForEach(model.ranked.prefix(maxDots)) { agent in
                        Button { model.compose(to: agent) } label: { AgentBadge(agent: agent, size: 24) }
                            .buttonStyle(.plain)
                            .help("@\(agent.handle) · \(agent.project ?? "") · \(agent.state.label)")
                    }
                    if model.agents.count > maxDots {
                        Button { model.toggle(.agents) } label: {
                            Text("+\(model.agents.count - maxDots)")
                                .font(.system(size: 10, weight: .semibold, design: .rounded))
                                .foregroundStyle(Theme.muted)
                                .frame(width: 24, height: 18)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }

            Rectangle().fill(Theme.stroke).frame(width: 22, height: 1)

            PillIcon(symbol: dictation.isRecording ? "mic.fill" : "mic", tint: dictation.isRecording ? Theme.red : nil,
                     help: "Dictate (⌃⌥Space opens the composer)") {
                model.toggleDictation()
            }
            .overlay {
                if dictation.isRecording {
                    Circle().stroke(Theme.red.opacity(0.6), lineWidth: 2)
                        .scaleEffect(1 + CGFloat(dictation.level) * 0.35)
                        .animation(.easeOut(duration: 0.1), value: dictation.level)
                }
            }
            PillIcon(symbol: "text.bubble", help: "Message an agent") {
                if model.panel == nil { model.compose(to: nil) } else { model.panel = nil }
            }
            PillIcon(symbol: "square.stack.3d.up", active: model.panel == .agents, help: "All agents") {
                model.toggle(.agents)
            }
            PillIcon(symbol: "ellipsis", help: "Settings") { onMenu() }

            Capsule().fill(Color.white.opacity(0.25)).frame(width: 18, height: 4)
                .padding(.top, 2)
                .contentShape(Rectangle().inset(by: -8))
                .gesture(DragGesture(minimumDistance: 1, coordinateSpace: .global).onChanged(onDrag).onEnded { _ in onDragEnd() })
                .help("Drag to move")
        }
        .padding(.top, 12)
        .padding(.bottom, 10)
        .frame(width: 50)
        .background(ZStack { Glass(); Color.black.opacity(0.35) }.clipShape(Capsule()))
        .overlay(Capsule().stroke(Theme.stroke, lineWidth: 1))
        .environment(\.colorScheme, .dark)
        .fixedSize()
    }
}

/// The lantern: dark when nothing is happening, a slow green breath while agents work, and an
/// amber glow with a count when agents are waiting on you.
struct LanternButton: View {
    let count: Int
    let working: Bool
    let open: Bool
    let action: () -> Void
    @State private var breathe = false

    var body: some View {
        Button(action: action) {
            ZStack {
                if count > 0 {
                    Circle().fill(Theme.amber.opacity(0.45)).blur(radius: 8)
                        .scaleEffect(breathe ? 1.15 : 0.9)
                    Circle().fill(RadialGradient(colors: [Theme.amber, Theme.amber.opacity(0.75)], center: .center, startRadius: 2, endRadius: 18))
                    Text("\(count)")
                        .font(.system(size: 15, weight: .bold, design: .rounded))
                        .foregroundStyle(.black.opacity(0.8))
                } else {
                    Circle().fill(Color.white.opacity(0.06))
                    Circle().stroke(working ? Theme.green.opacity(breathe ? 0.9 : 0.35) : Theme.stroke, lineWidth: 1.5)
                    Image(systemName: "flame")
                        .font(.system(size: 14, weight: .medium))
                        .foregroundStyle(working ? Theme.green : Theme.muted)
                }
            }
            .frame(width: 34, height: 34)
            .overlay(Circle().stroke(Color.white.opacity(open ? 0.6 : 0), lineWidth: 1.5).padding(-3))
        }
        .buttonStyle(.plain)
        .help(count > 0 ? "\(count) waiting on you" : "Inbox")
        .onAppear {
            withAnimation(.easeInOut(duration: 1.6).repeatForever(autoreverses: true)) { breathe = true }
        }
    }
}

struct PillIcon: View {
    let symbol: String
    var tint: Color? = nil
    var active = false
    var help: String
    let action: () -> Void
    @State private var hover = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(tint ?? (active ? Theme.ink : Theme.muted))
                .frame(width: 32, height: 32)
                .background(Circle().fill(Color.white.opacity(hover || active ? 0.12 : 0)))
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .help(help)
    }
}
