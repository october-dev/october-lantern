import AppKit
import SwiftUI

/// "Connect to October Desktop": pairs Lantern with a running October so replies to October's
/// agents go through October's safe delivery and "Open" shows them on the canvas.
struct DesktopCard: View {
    @ObservedObject var model: AppModel

    var body: some View {
        let link = model.octoberLink
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: link?.paired == true ? "macwindow.badge.plus" : "macwindow.on.rectangle")
                    .font(.system(size: 16)).foregroundStyle(link?.paired == true ? Theme.green : Theme.muted).frame(width: 22)
                VStack(alignment: .leading, spacing: 2) {
                    Text(link?.paired == true ? "Connected to October Desktop" : "Connect to October Desktop")
                        .font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                    Text(detail(link)).font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
                action(link)
            }
            if let code = link?.pairingCode {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Allow Lantern in October").font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.ink)
                        Text("Check October shows this code:").font(.system(size: 11.5)).foregroundStyle(Theme.muted)
                    }
                    Spacer()
                    Text(code).font(.system(size: 20, weight: .bold, design: .monospaced)).foregroundStyle(Theme.amber)
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
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(link?.paired == true ? Theme.green.opacity(0.35) : Theme.hairline))
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
