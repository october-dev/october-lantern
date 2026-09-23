import CoreImage.CIFilterBuiltins
import SwiftUI

/// "Connect to October phone app": Lantern acts as an October host computer, so the October
/// phone app pairs with it (QR code, then a 6-digit code to compare) and reaches Lantern's
/// agents through October's relay. The protocol lives in the engine (`engine/src/mobile`).
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
        let status: String
        let message: String?
        let hostId: String
        let devices: [Device]
        let pairing: Pairing?
    }

    @Published private(set) var state: State?
    @Published private(set) var started = false

    /// Set by the app: sends a request to the engine (`EngineClient.phone`).
    var send: (([String: Any]) -> Void)?

    func receive(_ data: Data) {
        if let s = try? JSONDecoder().decode(State.self, from: data) { state = s }
    }

    /// Start (or refresh the token of) the phone host. Call after sign-in and on token refresh.
    func start(accessToken: String) {
        send?(["type": started ? "phone.token" : "phone.start", "accessToken": accessToken])
        started = true
    }

    func stop() {
        send?(["type": "phone.stop"])
        started = false
        state = nil
    }

    func pair() { send?(["type": "phone.pair"]) }
    func decide(_ allow: Bool) { send?(["type": "phone.decide", "allow": allow]) }
    func revoke(_ bind: String) { send?(["type": "phone.revoke", "bind": bind]) }
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
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "iphone").font(.system(size: 16)).foregroundStyle(connected ? Theme.green : Theme.muted).frame(width: 22)
                VStack(alignment: .leading, spacing: 2) {
                    Text("October phone app").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                    Text(detail).font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
                if signedIn && planAllowsPhone != false && model.state?.pairing == nil {
                    Button(model.state?.devices.isEmpty ?? true ? "Pair a Phone" : "Pair Another") { model.pair() }
                        .buttonStyle(SecondaryButtonStyle())
                }
            }

            if let pairing = model.state?.pairing {
                pairingView(pairing)
            }

            ForEach(model.state?.devices ?? []) { device in
                HStack(spacing: 8) {
                    Image(systemName: "iphone.gen3").foregroundStyle(Theme.muted)
                    Text(device.label).font(.system(size: 12.5)).foregroundStyle(Theme.ink)
                    Spacer()
                    Button("Remove") { model.revoke(device.bind) }
                        .buttonStyle(.plain).font(.system(size: 11.5)).foregroundStyle(Theme.muted)
                }
            }

            if let message = model.state?.message {
                Text(message).font(.system(size: 11.5)).foregroundStyle(Theme.amber).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(connected ? Theme.green.opacity(0.35) : Theme.hairline))
    }

    private var connected: Bool { model.state?.status == "connected" && !(model.state?.devices.isEmpty ?? true) }

    private var detail: String {
        guard signedIn else { return "Sign in to October above, then pair your phone to check on and reply to your agents." }
        if planAllowsPhone == false { return "Your October plan doesn't include the phone app." }
        guard let s = model.state else { return "Starting…" }
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
