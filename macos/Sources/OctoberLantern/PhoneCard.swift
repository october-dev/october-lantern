import CoreImage.CIFilterBuiltins
import SwiftUI

/// "Connect to October phone app": Lantern acts as an October host computer, so the October
/// phone app pairs with it (QR code, then a 6-digit code to compare) and reaches Lantern's
/// agents through October's relay. The protocol lives in the engine (`engine/src/mobile`); this
/// is a view of the engine's phone state plus the requests the card can make.
@MainActor
final class PhoneModel: ObservableObject {
    static let shared = PhoneModel()

    struct Device: Decodable, Identifiable, Hashable {
        let bind: String
        let label: String
        let platform: String
        let pairedAt: Double
        var id: String { bind }
    }

    struct Pairing: Decodable, Equatable {
        let qr: String
        let expiresAt: Double
        let code: String?
        let label: String?
        let finishing: Bool
    }

    struct State: Decodable {
        /// offline | connecting | connected | plan-required | signed-out | error
        let status: String
        let message: String?
        let hostId: String?
        let devices: [Device]
        let pairing: Pairing?
    }

    @Published private(set) var state: State?

    /// Set by `AppModel`; the engine hosts the phone connection.
    weak var engine: EngineClient?

    func receive(_ s: State) {
        if s.devices.count > (state?.devices.count ?? s.devices.count) { Analytics.shared.capture("phone_paired") }
        state = s
    }

    /// The engine is gone, and with it the host; a new engine reports fresh state after `token`.
    func reset() { state = nil }

    /// The current October access token. Sent after sign-in, on every refresh, and whenever the
    /// engine (re)starts: the engine starts hosting if it isn't, or takes the new token.
    func token(_ accessToken: String) { engine?.phoneToken(accessToken) }

    func stop() {
        engine?.phoneStop()
        state = nil
    }

    func pair() { engine?.phonePair() }
    func cancelPairing() { engine?.phoneCancelPair() }
    func decide(_ allow: Bool) { engine?.phoneDecide(allow: allow) }
    func revoke(_ bind: String) { engine?.phoneRevoke(bind: bind) }
}

/// The card in the October panel.
struct PhoneCard: View {
    @ObservedObject var model = PhoneModel.shared
    /// Whether the user is signed in to October, and whether their plan includes the phone app
    /// (nil when unknown). Provided by the October panel.
    let signedIn: Bool
    let planAllowsPhone: Bool?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            PhoneArtwork()
            HStack(alignment: .top, spacing: 10) {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        Text("October phone app").font(.system(size: 13.5, weight: .semibold)).foregroundStyle(Theme.ink)
                            .lineLimit(1).fixedSize()
                        StatusPill(text: statusText, color: statusColor)
                    }
                    Text(detail).font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
                if signedIn && planAllowsPhone != false && model.state?.pairing == nil && model.state?.status != "error" {
                    Button(model.state?.devices.isEmpty ?? true ? "Pair a Phone" : "Pair Another") { model.pair() }
                        .buttonStyle(SecondaryButtonStyle())
                }
            }

            if let pairing = model.state?.pairing {
                pairingView(pairing)
            }

            ForEach(model.state?.devices ?? []) { device in
                HStack(spacing: 10) {
                    Image(systemName: "iphone.gen3").font(.system(size: 15)).foregroundStyle(connected ? Theme.green : Theme.muted)
                        .frame(width: 28, height: 28)
                        .background(RoundedRectangle(cornerRadius: 8).fill(Theme.faint))
                    VStack(alignment: .leading, spacing: 1) {
                        Text(device.label).font(.system(size: 12.5, weight: .medium)).foregroundStyle(Theme.ink).lineLimit(1)
                        Text("Paired \(Date(timeIntervalSince1970: device.pairedAt / 1000).formatted(date: .abbreviated, time: .omitted))")
                            .font(.system(size: 10.5)).foregroundStyle(Theme.muted)
                    }
                    Spacer()
                    Button("Remove") { model.revoke(device.bind) }
                        .buttonStyle(.plain).font(.system(size: 11.5)).foregroundStyle(Theme.muted)
                }
                .padding(.horizontal, 10).padding(.vertical, 7)
                .background(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Theme.hairline))
            }

            if let message = model.state?.message {
                Text(message).font(.system(size: 11.5)).foregroundStyle(Theme.amber).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(connected ? Theme.green.opacity(0.35) : Theme.hairline))
    }

    private var statusText: String {
        guard signedIn else { return "Sign in first" }
        if planAllowsPhone == false || model.state?.status == "plan-required" { return "Upgrade" }
        if model.state?.pairing != nil { return "Pairing" }
        if model.state?.status == "error" { return "Problem" }
        if model.state?.devices.isEmpty ?? true { return "Not paired" }
        switch model.state?.status {
        case "connected": return "Connected"
        case "connecting": return "Connecting"
        default: return "Offline"
        }
    }

    private var statusColor: Color {
        switch statusText {
        case "Connected": Theme.green
        case "Pairing", "Connecting", "Upgrade": Theme.amber
        case "Problem": Theme.red
        default: Theme.muted
        }
    }

    private var connected: Bool { model.state?.status == "connected" && !(model.state?.devices.isEmpty ?? true) }

    private var detail: String {
        guard signedIn else { return "Sign in to October above, then pair your phone to check on and reply to your agents." }
        if planAllowsPhone == false { return "Your October plan doesn't include the phone app." }
        guard let s = model.state else { return "Starting…" }
        if s.status == "error" { return "The phone connection couldn't start." }
        if s.devices.isEmpty { return "Check on your agents and reply to them from your phone." }
        switch s.status {
        case "connected": return "Connected. Your phone sees Lantern's agents."
        case "connecting": return "Connecting to October…"
        case "plan-required": return "Your October plan doesn't include the phone app."
        case "signed-out": return "Sign in to October again to reconnect."
        default: return "Offline. Lantern will reconnect automatically."
        }
    }

    @ViewBuilder
    private func pairingView(_ p: PhoneModel.Pairing) -> some View {
        if let code = p.code {
            VStack(alignment: .leading, spacing: 8) {
                Text("Does \(p.label ?? "your phone") show this code?").font(.system(size: 12)).foregroundStyle(Theme.muted)
                Text(code.prefix(3) + " " + code.suffix(3))
                    .font(.system(size: 26, weight: .semibold, design: .monospaced)).foregroundStyle(Theme.ink)
                if p.finishing {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.small)
                        Text("Finishing on your phone…").font(.system(size: 12)).foregroundStyle(Theme.muted)
                    }
                } else {
                    HStack(spacing: 8) {
                        Button("Allow") { model.decide(true) }.buttonStyle(AmberButtonStyle())
                        Button("Deny") { model.decide(false) }.buttonStyle(SecondaryButtonStyle())
                    }
                }
            }
        } else {
            HStack(alignment: .top, spacing: 12) {
                if let image = qrImage(p.qr) {
                    Image(nsImage: image).interpolation(.none).resizable().frame(width: 132, height: 132)
                        .padding(6).background(RoundedRectangle(cornerRadius: 8).fill(.white))
                }
                VStack(alignment: .leading, spacing: 6) {
                    Text("Scan with the October app").font(.system(size: 12.5, weight: .semibold)).foregroundStyle(Theme.ink)
                    Text("In the October app, choose Pair a computer and scan this code. Use the same October account on both.")
                        .font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                    Text("Expires \(Date(timeIntervalSince1970: p.expiresAt / 1000).formatted(date: .omitted, time: .shortened))")
                        .font(.system(size: 11)).foregroundStyle(Theme.muted)
                    Button("Cancel") { model.cancelPairing() }.buttonStyle(SecondaryButtonStyle())
                }
            }
        }
    }

    private func qrImage(_ text: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)),
              let cg = CIContext().createCGImage(output, from: output.extent) else { return nil }
        return NSImage(cgImage: cg, size: NSSize(width: output.extent.width / 2, height: output.extent.height / 2))
    }
}

/// The phone app, as October Desktop shows it: three real screens of the app over October's
/// wallpaper, the outer two tilted.
private struct PhoneArtwork: View {
    var body: some View {
        ZStack {
            if let wall = Assets.image("october/hero-palace.jpg") {
                Image(nsImage: wall).resizable().aspectRatio(contentMode: .fill)
            }
            LinearGradient(colors: [.clear, .black.opacity(0.45)], startPoint: .top, endPoint: .bottom)
            phone("october/phone-agent-network.jpg").rotationEffect(.degrees(-13)).offset(x: -78, y: 30)
            phone("october/phone-agent-chat.jpg").rotationEffect(.degrees(13)).offset(x: 78, y: 30)
            phone("october/phone-dashboard.jpg").offset(y: 14)
        }
        .frame(height: 150)
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        .accessibilityHidden(true)
    }

    private func phone(_ path: String) -> some View {
        Group {
            if let img = Assets.image(path) {
                Image(nsImage: img).resizable().aspectRatio(contentMode: .fill)
            } else {
                Color.black
            }
        }
        .frame(width: 74, height: 160)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Color.black.opacity(0.7), lineWidth: 2.5))
        .shadow(color: .black.opacity(0.45), radius: 8, y: 4)
    }
}

