import SwiftUI

/// The Team button's fan of teammates. Multiplayer isn't built yet, so it shows a few October
/// avatars and "coming soon"; clicking opens the October panel, where the Team card is.
///
/// The fan lives in its own small window beside the pill (the pill's window is only as wide as
/// the pill); WindowController shows it while the mouse is on the Team button or on the fan.
@MainActor
final class TeamStack: ObservableObject {
    static let shared = TeamStack()

    /// Where the Team button sits in the pill's view, top-left origin.
    var buttonFrame: CGRect = .zero
    @Published var shown = false
    /// Fans out to the left (pill on the right edge) or to the right.
    @Published var leftward = true
    /// A few of October's avatars, picked once per launch.
    let avatars: [Int] = Array((21...44).shuffled().prefix(5))
}

struct TeamButtonFrameKey: PreferenceKey {
    static let defaultValue: CGRect = .zero
    static func reduce(value: inout CGRect, nextValue: () -> CGRect) { value = nextValue() }
}

/// The Team button in the pill: two overlapping avatars.
struct TeamButton: View {
    let active: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            ZStack {
                avatar(TeamStack.shared.avatars[1]).offset(x: 5, y: 3)
                avatar(TeamStack.shared.avatars[0]).offset(x: -4, y: -3)
            }
            .frame(width: 32, height: 32)
            .background(Circle().fill(active ? Theme.faint : Color.clear))
        }
        .buttonStyle(.plain)
        .background(GeometryReader { g in Color.clear.preference(key: TeamButtonFrameKey.self, value: g.frame(in: .global)) })
        .onPreferenceChange(TeamButtonFrameKey.self) { frame in
            MainActor.assumeIsolated { TeamStack.shared.buttonFrame = frame }
        }
        .help("Multiplayer · coming soon")
        .accessibilityLabel("Team, coming soon")
    }

    private func avatar(_ n: Int) -> some View {
        Group {
            if let img = Assets.avatar(n) {
                Image(nsImage: img).resizable().aspectRatio(contentMode: .fill)
            } else {
                Circle().fill(Theme.faint)
            }
        }
        .frame(width: 17, height: 17)
        .clipShape(Circle())
        .overlay(Circle().strokeBorder(Color.black.opacity(0.55), lineWidth: 1.5))
    }
}

/// The fan of teammates that opens beside the Team button.
struct TeamStackView: View {
    @ObservedObject var stack: TeamStack
    let onOpen: () -> Void

    var body: some View {
        Button(action: onOpen) {
            HStack(spacing: 8) {
                if !stack.leftward { label }
                HStack(spacing: -9) {
                    // The avatar nearest the pill is on top.
                    let order = stack.leftward ? stack.avatars : stack.avatars.reversed()
                    ForEach(Array(order.enumerated()), id: \.element) { i, n in
                        let fromPill = stack.leftward ? order.count - 1 - i : i
                        face(n)
                            .zIndex(Double(stack.leftward ? i : order.count - i))
                            .offset(x: stack.shown ? 0 : CGFloat(fromPill + 1) * (stack.leftward ? 14 : -14))
                            .opacity(stack.shown ? 1 : 0)
                            .animation(.spring(response: 0.32, dampingFraction: 0.75).delay(Double(fromPill) * 0.035), value: stack.shown)
                    }
                }
                if stack.leftward { label }
            }
            .padding(.horizontal, 10).padding(.vertical, 6)
        }
        .buttonStyle(.plain)
        .environment(\.colorScheme, .dark)
        .fixedSize()
        .help("Multiplayer is coming soon: see and message your teammates' agents. Click to learn more.")
    }

    private var label: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Multiplayer").font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.ink)
            Text("coming soon").font(.system(size: 10)).foregroundStyle(Theme.muted)
        }
        .opacity(stack.shown ? 1 : 0)
        .animation(.easeOut(duration: 0.2).delay(0.12), value: stack.shown)
    }

    private func face(_ n: Int) -> some View {
        Group {
            if let img = Assets.avatar(n) {
                Image(nsImage: img).resizable().aspectRatio(contentMode: .fill)
            } else {
                Circle().fill(Theme.faint)
            }
        }
        .frame(width: 30, height: 30)
        .clipShape(Circle())
        .overlay(Circle().strokeBorder(Color.black.opacity(0.6), lineWidth: 2))
    }
}
