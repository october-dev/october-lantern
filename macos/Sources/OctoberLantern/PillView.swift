import SwiftUI

/// The always-on button. Collapsed, it's just the lantern: lit with a count when agents are waiting
/// on you. Hovering over it (or opening a panel) expands the actions; it tucks back in afterwards.
struct PillView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var dictation: Dictation
    var onDrag: () -> Void
    var onDragEnd: () -> Void
    var onMenu: () -> Void

    private var expanded: Bool { model.pillExpanded || model.panel != nil || dictation.isActive }

    /// What's wrong with the engine, when it isn't running normally (nil while starting or ready).
    private var engineProblem: String? {
        switch model.engineHealth {
        case .ready, .starting: nil
        case .restarting: "Lantern's engine stopped and is restarting. Agents may be out of date."
        case .failed(let message): "Lantern's engine isn't running: \(message)"
        }
    }

    var body: some View {
        VStack(spacing: 8) {
            LanternButton(count: model.badgeCount, waiting: !model.inbox.isEmpty, working: model.anyWorking, onTap: {
                model.toggle(.inbox)
            }, onDrag: onDrag, onDragEnd: onDragEnd)
            .overlay(alignment: .topTrailing) {
                if let engineProblem {
                    Image(systemName: "exclamationmark")
                        .font(.system(size: 8, weight: .heavy))
                        .foregroundStyle(.black.opacity(0.85))
                        .frame(width: 14, height: 14)
                        .background(Circle().fill(model.engineHealth == .restarting ? Theme.amber : Theme.red))
                        .help(engineProblem)
                        .accessibilityLabel(engineProblem)
                }
            }

            if expanded {
                VStack(spacing: 6) {
                    Rectangle().fill(Theme.stroke).frame(width: 22, height: 1).padding(.bottom, 2)
                    PillIcon(symbol: "tray", active: model.panel == .inbox, help: "Waiting on you") {
                        model.toggle(.inbox)
                    }
                    PillIcon(symbol: "square.stack.3d.up", active: model.panel == .agents, help: "All agents") {
                        model.toggle(.agents)
                    }
                    PillIcon(symbol: "plus", active: model.panel == .newSession, help: "New session, or a task for the app you're in") {
                        model.startNew()
                    }
                    PillIcon(symbol: dictation.isActive ? "mic.fill" : "mic",
                             tint: dictation.isRecording ? Theme.red : dictation.isActive ? Theme.amber : nil,
                             help: dictation.isAuthorizing ? "Waiting for microphone permission… click to cancel" : dictation.isRecording ? "Stop dictating" : "Dictate") {
                        model.toggleDictation()
                    }
                    .overlay {
                        if dictation.isRecording {
                            Circle().stroke(Theme.red.opacity(0.6), lineWidth: 2)
                                .scaleEffect(1 + CGFloat(dictation.level) * 0.35)
                                .animation(.easeOut(duration: 0.1), value: dictation.level)
                        }
                    }
                    PillIcon(symbol: "ellipsis", help: "Settings") { onMenu() }
                    Rectangle().fill(Theme.stroke).frame(width: 22, height: 1).padding(.vertical, 2)
                    TeamButton(active: model.panel == .october) { model.toggle(.october) }
                    Button { model.toggle(.october) } label: {
                        OctoberLogo(size: 26)
                            .overlay(
                                RoundedRectangle(cornerRadius: 6.5, style: .continuous)
                                    .strokeBorder(Color.white.opacity(model.panel == .october ? 0.7 : 0), lineWidth: 1.5)
                                    .padding(-3)
                            )
                            .frame(width: 32, height: 32)
                    }
                    .buttonStyle(.plain)
                    .help("October")
                    .accessibilityLabel("October")
                }
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .padding(6)
        .glassSurface(Capsule())
        .environment(\.colorScheme, .dark)
        .animation(.spring(response: 0.28, dampingFraction: 0.85), value: expanded)
        .fixedSize()
    }
}

/// The lantern logo. Lit with a count when agents are waiting on you, a soft glow while they work,
/// and dimmed when nothing is happening. Tap to open the inbox; drag to move.
struct LanternButton: View {
    let count: Int
    /// Something is waiting that you've already seen: a softer amber glow, no number.
    let waiting: Bool
    let working: Bool
    let onTap: () -> Void
    let onDrag: () -> Void
    let onDragEnd: () -> Void

    var body: some View {
        ZStack(alignment: .bottom) {
            ZStack {
                if count > 0 || waiting || working {
                    Circle()
                        .fill((count > 0 || waiting ? Theme.amber : Color.orange).opacity(count > 0 ? 0.55 : 0.25))
                        .blur(radius: 9)
                        .scaleEffect(count > 0 ? 1.05 : 0.9)
                }
                if let logo = Assets.logo {
                    Image(nsImage: logo).resizable().interpolation(.high).aspectRatio(contentMode: .fit)
                        .saturation(count > 0 || waiting || working ? 1 : 0.55)
                        .opacity(count > 0 || waiting || working ? 1 : 0.8)
                } else {
                    Image(systemName: "flame.fill").font(.system(size: 20)).foregroundStyle(Theme.amber)
                }
            }
            .frame(width: 34, height: 34)
            .padding(2)

            if count > 0 {
                Text("\(count)")
                    .font(.system(size: 10, weight: .bold, design: .rounded))
                    .foregroundStyle(.black.opacity(0.85))
                    .padding(.horizontal, 5)
                    .frame(minWidth: 17, minHeight: 17)
                    .background(Capsule().fill(Theme.amber))
                    .overlay(Capsule().stroke(Color.black.opacity(0.5), lineWidth: 1))
                    // Inside the pill's circle, which clips anything poking out.
                    .offset(y: 3)
            }
        }
        .contentShape(Circle())
        .help(count > 0 ? "\(count) new, waiting on you" : waiting ? "Agents are waiting on you" : "October Lantern")
        .gesture(
            DragGesture(minimumDistance: 4)
                .onChanged { _ in onDrag() }
                .onEnded { _ in onDragEnd() }
        )
        // A tap gesture rather than a Button, so the same view can be dragged; it still acts as a
        // button for VoiceOver and keyboard users.
        .onTapGesture { onTap() }
        .accessibilityElement(children: .ignore)
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(count > 0 ? "October Lantern, \(count) waiting on you" : "October Lantern")
        .accessibilityHint("Shows who's waiting on you")
        .accessibilityAction { onTap() }
        // No continuous animation: redrawing a blur inside the glass every frame costs ~15% of a
        // CPU core. The glow changes only when the state does.
        .animation(.easeOut(duration: 0.3), value: count)
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
        .accessibilityLabel(help)
    }
}
