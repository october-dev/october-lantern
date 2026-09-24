import AppKit
import SwiftUI

/// "Connect to October Desktop": pairs Lantern with a running October so replies to October's
/// agents go through October's safe delivery and "Open" shows them on the canvas.
struct DesktopCard: View {
    @ObservedObject var model: AppModel

    var body: some View {
        let link = model.octoberLink
        let paired = link?.paired == true
        VStack(alignment: .leading, spacing: 10) {
            DesktopArtwork(connected: paired, pairing: link?.status == "pairing")
            // Title with its status under it, the action on the right; the explanation below, full width.
            HStack(alignment: .center, spacing: 8) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("October Desktop").font(.system(size: 13.5, weight: .semibold)).foregroundStyle(Theme.ink).lineLimit(1)
                    StatusPill(text: statusText(link), color: statusColor(link))
                }
                Spacer(minLength: 8)
                action(link).fixedSize()
            }
            Text(detail(link)).font(.system(size: 11.5)).foregroundStyle(Theme.muted)
                .frame(maxWidth: .infinity, alignment: .leading).fixedSize(horizontal: false, vertical: true)
            if let code = link?.pairingCode {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Allow Lantern in October").font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.ink)
                        Text("Check October shows this code:").font(.system(size: 11.5)).foregroundStyle(Theme.muted)
                    }
                    Spacer()
                    Text(code).font(.system(size: 20, weight: .bold, design: .monospaced)).tracking(2).foregroundStyle(Theme.amber)
                }
                .padding(10)
                .background(RoundedRectangle(cornerRadius: 10).fill(Theme.amber.opacity(0.1)))
            }
            if let message = link?.message {
                Text(message).font(.system(size: 11.5)).foregroundStyle(Theme.red).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(paired ? Theme.green.opacity(0.35) : Theme.hairline))
    }

    private func statusText(_ link: OctoberLink?) -> String {
        switch link?.status {
        case "connected": "Connected"
        case "readOnly": "Read-only"
        case "pairing": "Pairing"
        case "notRunning": "Not running"
        case "error": "Problem"
        default: "Not installed"
        }
    }

    private func statusColor(_ link: OctoberLink?) -> Color {
        switch link?.status {
        case "connected": Theme.green
        case "pairing", "readOnly": Theme.amber
        case "error": Theme.red
        default: Theme.muted
        }
    }

    private func detail(_ link: OctoberLink?) -> String {
        switch link?.status {
        case "connected":
            return "\(link!.agentCount) agent\(link!.agentCount == 1 ? "" : "s") in October. Replies go through October, and Open shows them on the canvas."
        case "readOnly":
            return "October is running. Lantern lists its terminals; connect to reply through October and see agents' names and questions."
        case "pairing": return "Waiting for you to allow Lantern in October…"
        case "notRunning": return "Open October Desktop to connect."
        case "error": return link?.message.map { "Couldn't talk to October: \($0)" } ?? "Couldn't talk to October."
        default: return "Get October Desktop to run teams of agents on a canvas."
        }
    }

    @ViewBuilder
    private func action(_ link: OctoberLink?) -> some View {
        switch link?.status {
        case "connected":
            Button("Disconnect") { model.october("october.forget") }.buttonStyle(SecondaryButtonStyle())
                .help("Lantern forgets October's permission and goes back to only listing October's terminals. Remove Lantern in October's Connected apps too.")
        case "readOnly", "error":
            Button("Connect") { model.october("october.pair") }.buttonStyle(SecondaryButtonStyle())
        case "pairing":
            Button("Cancel") { model.october("october.cancelPair") }.buttonStyle(SecondaryButtonStyle())
        case "notRunning":
            Button("Open October") {
                NSWorkspace.shared.open(URL(fileURLWithPath: "/Applications/October.app"))
            }
            .buttonStyle(SecondaryButtonStyle())
        default:
            Button("Get October") { NSWorkspace.shared.open(URL(string: "https://www.october.dev")!) }.buttonStyle(SecondaryButtonStyle())
        }
    }
}

/// A small status label, e.g. "Connected".
struct StatusPill: View {
    let text: String
    let color: Color

    var body: some View {
        HStack(spacing: 4) {
            Circle().fill(color).frame(width: 6, height: 6)
            Text(text).font(.system(size: 10.5, weight: .semibold)).foregroundStyle(color)
        }
        .padding(.horizontal, 7).padding(.vertical, 2)
        .background(Capsule().fill(color.opacity(0.14)))
        // Never squeezed into a column: the text stays on one line.
        .fixedSize()
    }
}

/// October's canvas with its agent nodes on the left, Lantern on the right, and the link between
/// them: dashed until they're connected, then a flowing line.
private struct DesktopArtwork: View {
    let connected: Bool
    let pairing: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        TimelineView(.animation(minimumInterval: 1 / 30, paused: !connected || reduceMotion)) { context in
            let phase = context.date.timeIntervalSinceReferenceDate
            GeometryReader { geo in
                let w = geo.size.width, h = geo.size.height
                let october = CGPoint(x: w * 0.22, y: h * 0.5)
                let lantern = CGPoint(x: w * 0.8, y: h * 0.5)
                ZStack {
                    Dots().fill(Theme.ink.opacity(0.08))
                    // Agent nodes on October's canvas, wired to October.
                    ForEach(Array(["@claude-2", "@codex-1"].enumerated()), id: \.offset) { i, name in
                        let p = CGPoint(x: w * 0.47, y: h * (i == 0 ? 0.2 : 0.8))
                        Path { path in
                            path.move(to: october)
                            path.addQuadCurve(to: p, control: CGPoint(x: (october.x + p.x) / 2, y: p.y))
                        }
                        .stroke(Theme.ink.opacity(0.18), lineWidth: 1)
                        Text(name).font(.system(size: 9.5, weight: .medium, design: .monospaced)).foregroundStyle(Theme.ink.opacity(0.75))
                            .padding(.horizontal, 6).padding(.vertical, 3)
                            .background(RoundedRectangle(cornerRadius: 6).fill(Color.black.opacity(0.35)))
                            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.hairline))
                            .position(p)
                    }
                    // The link to Lantern.
                    Path { path in
                        path.move(to: CGPoint(x: october.x + 26, y: october.y))
                        path.addLine(to: CGPoint(x: lantern.x - 24, y: lantern.y))
                    }
                    .stroke(
                        connected ? Theme.green.opacity(0.8) : (pairing ? Theme.amber.opacity(0.7) : Theme.ink.opacity(0.25)),
                        style: StrokeStyle(lineWidth: 1.5, dash: connected ? [6, 5] : [3, 5], dashPhase: connected ? -phase * 24 : 0)
                    )
                    icon(Self.octoberIcon, fallback: "macwindow").position(october)
                    icon(Assets.logo, fallback: "flame.fill").position(lantern)
                }
            }
        }
        .frame(height: 96)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Color.black.opacity(0.22)))
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        .accessibilityHidden(true)
    }

    private func icon(_ image: NSImage?, fallback: String) -> some View {
        Group {
            if let image {
                Image(nsImage: image).resizable().interpolation(.high).aspectRatio(contentMode: .fit)
            } else {
                Image(systemName: fallback).font(.system(size: 20)).foregroundStyle(Theme.ink)
            }
        }
        .frame(width: 40, height: 40)
        .shadow(color: .black.opacity(0.35), radius: 6, y: 2)
    }

    /// October Desktop's own icon when it's installed, else October's mark.
    private static let octoberIcon: NSImage? = {
        let app = "/Applications/October.app"
        if FileManager.default.fileExists(atPath: app) { return NSWorkspace.shared.icon(forFile: app) }
        return Assets.harness(AgentKind(rawValue: "october"))
    }()
}

