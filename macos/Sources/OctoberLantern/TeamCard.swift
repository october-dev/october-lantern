import SwiftUI

/// "Team": teammates' agents in Lantern, across Lantern and October Desktop. Not built yet, so the
/// card is locked: it says what's coming and its button is disabled. No network calls.
struct TeamCard: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "person.2").font(.system(size: 16)).foregroundStyle(Theme.muted).frame(width: 22)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text("Team").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                        Label("Coming soon", systemImage: "lock.fill")
                            .font(.system(size: 9.5, weight: .semibold))
                            .foregroundStyle(Theme.amber)
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Capsule().fill(Theme.amber.opacity(0.15)))
                    }
                    Text("See and message your teammates' agents, across Lantern and October Desktop.")
                        .font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
                Button("Invite teammates") {}.buttonStyle(SecondaryButtonStyle()).disabled(true).opacity(0.5)
                    .help("Coming soon")
            }
            VStack(alignment: .leading, spacing: 4) {
                bullet("Your teammates' agents show up next to yours, with what they're doing.")
                bullet("Message a teammate's agent; it's delivered on their Mac, never run from yours.")
                bullet("Works with teammates on October Desktop too.")
            }
            .padding(.leading, 34)
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Theme.hairline))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Team, coming soon. See and message your teammates' agents, across Lantern and October Desktop.")
    }

    private func bullet(_ text: String) -> some View {
        HStack(alignment: .top, spacing: 6) {
            Text("•").foregroundStyle(Theme.muted)
            Text(text).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
        }
        .font(.system(size: 11))
    }
}
