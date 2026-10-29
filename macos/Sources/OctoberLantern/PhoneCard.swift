import CoreImage.CIFilterBuiltins
import SwiftUI

/// Pairing links live only in this view model and the engine's memory. Never encode this model
/// into preferences, analytics, logs or saved screenshots.
@MainActor
final class PhoneModel: ObservableObject {
    static let shared = PhoneModel()

    struct Device: Decodable, Identifiable, Hashable {
        let bind: String
        let label: String
        let platform: String
        let pairedAt: Double
        let lastSeenAt: Double?
        let removalPending: Bool
        var id: String { bind }
    }
    struct Pairing: Decodable, Equatable {
        let intentId: String
        let bind: String?
        let qr: String?
        let expiresAt: Double
        let code: String?
        let label: String?
        let platform: String?
        let finishing: Bool

        func remaining(at date: Date) -> Int { max(0, Int(ceil(expiresAt / 1000 - date.timeIntervalSince1970))) }
    }
    struct State: Decodable {
        let status: String
        let message: String?
        let errorCode: String?
        let outcome: String?
        let hostId: String?
        let devices: [Device]
        let pairing: Pairing?
        let busy: Bool
        let installUrl: URL?
    }

    @Published private(set) var state: State?
    @Published var setup = false
    @Published private(set) var sending = false
    @Published private(set) var localError: String?
    weak var engine: EngineClient?

    var busy: Bool { sending || state?.busy == true }

    /// Returns true only for a new comparison, so AppModel brings this panel forward once.
    @discardableResult func receive(_ s: State) -> Bool {
        let newConfirmation = s.pairing?.code != nil && s.pairing?.finishing == false &&
            (s.pairing?.intentId != state?.pairing?.intentId || s.pairing?.code != state?.pairing?.code)
        if s.outcome == "paired", state?.outcome != "paired" { Analytics.shared.capture("phone_paired") }
        state = s
        if !s.busy { sending = false }
        if s.outcome == "paired" { setup = false }
        if newConfirmation { setup = true }
        return newConfirmation
    }

    func reset() { state = nil; sending = false; localError = nil }
    func token(_ accessToken: String) { engine?.phoneToken(accessToken) }
    func stop() { engine?.phoneStop(); reset(); setup = false }
    private func perform(_ send: () -> Bool) {
        guard !busy else { return }
        localError = nil
        if send() { sending = true } else { localError = "Lantern's connection is restarting. Try again when it is ready." }
    }
    func pair() { perform { engine?.phonePair() ?? false } }
    func retry() { perform { engine?.phoneRetry() ?? false } }
    func cancelPairing() { perform { engine?.phoneCancelPair() ?? false }; setup = false }
    func decide(_ p: Pairing, _ allow: Bool) {
        guard p == state?.pairing else { return }
        perform { engine?.phoneDecide(p, allow: allow) ?? false }
    }
    func revoke(_ bind: String) { perform { engine?.phoneRevoke(bind: bind) ?? false } }

    /// One recovery action for the current host error. Consume failures on the phone are not
    /// sent to this computer; its account guidance is always visible beside the QR.
    static func recovery(for code: String?, outcome: String?) -> String {
        switch code {
        case "plan_required": return "Upgrade"
        case "SESSION_REQUIRED": return "Sign in again"
        case "PAIRING_EXPIRED", "PAIRING_USED", "PAIRING_DISCONNECTED": return "New code"
        case "PAIRING_DENIED": return "Start again"
        case "PAIRING_REJECTED": return "New code"
        default: return outcome == "denied" ? "Start again" : "Retry"
        }
    }
}

/// October panel → Phone → Pair. The same section owns installation, scanning, comparison,
/// completion and device management; a pending request never opens a different dialog.
struct PhoneCard: View {
    @ObservedObject var model = PhoneModel.shared
    @ObservedObject var account = OctoberAccount.shared
    @State private var copied = false
    @State private var copyClear: DispatchWorkItem?
    @State private var confirmingRemoval: PhoneModel.Device?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Label("Phone", systemImage: "iphone").font(.system(size: 14, weight: .semibold)).foregroundStyle(Theme.ink)
                Spacer()
                if model.state?.pairing == nil && !model.setup && model.state?.outcome != "denied" {
                    Button("Pair") { model.setup = true }.buttonStyle(SecondaryButtonStyle()).disabled(model.busy)
                }
            }
            Text("See waiting agents and reply from your phone.").font(.system(size: 12)).foregroundStyle(Theme.muted)
            if model.state?.status == "plan-required" || account.plan?.features?.mobile?.enabled == false {
                Text("Your plan does not currently include mobile access.").foregroundStyle(Theme.amber)
                Button("Upgrade") { NSWorkspace.shared.open(URL(string: "https://www.october.dev/pricing")!) }
                    .buttonStyle(SecondaryButtonStyle())
                Button("Refresh plan") { account.refreshPlan() }.buttonStyle(SecondaryButtonStyle())
            } else if let p = model.state?.pairing {
                if let code = p.code { confirm(p, code: code) } else { scan(p) }
            } else if model.state?.outcome == "paired" {
                Label("iPhone connected", systemImage: "checkmark.circle.fill").foregroundStyle(Theme.green)
                Text("You'll get agent alerts and can reply from your phone. Allow notifications in October on your iPhone.")
                    .foregroundStyle(Theme.muted)
            } else if model.setup && model.state?.errorCode == nil && model.state?.outcome != "denied" {
                install
            }
            if model.busy {
                HStack(spacing: 6) { ProgressView().controlSize(.small); Text("Connecting…").foregroundStyle(Theme.muted) }
            }
            if let message = model.localError ?? model.state?.message {
                Text(message).foregroundStyle(Theme.amber).textSelection(.enabled)
            }
            if model.state?.errorCode != nil || model.state?.outcome == "denied" {
                if model.state?.outcome == "denied" { Text("You denied this phone. It has not been connected.").foregroundStyle(Theme.muted) }
                recovery
            }
            if !(model.state?.devices.isEmpty ?? true) {
                Divider().overlay(Theme.hairline)
                Text("Paired phones").font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.ink)
                ForEach(model.state?.devices ?? []) { device in deviceRow(device) }
            }
            Text("End-to-end encrypted. Only phones you approve can connect.").foregroundStyle(Theme.muted)
            Text("Keep this computer awake with Lantern open for phone access and agent alerts.").foregroundStyle(Theme.muted)
        }
        .font(.system(size: 11.5))
        .fixedSize(horizontal: false, vertical: true)
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(Theme.hairline))
        .onChange(of: account.loadingPlan) { old, loading in
            if old && !loading && account.mobileAllowed && model.state?.status == "plan-required" { model.retry() }
        }
        .onChange(of: model.state?.pairing?.intentId) { _, _ in clearCopiedLink() }
        .onDisappear { clearCopiedLink() }
        .confirmationDialog("Remove \(confirmingRemoval?.label ?? "this phone")?", isPresented: Binding(
            get: { confirmingRemoval != nil }, set: { if !$0 { confirmingRemoval = nil } }
        ), titleVisibility: .visible) {
            if let d = confirmingRemoval {
                Button("Remove", role: .destructive) { model.revoke(d.bind); confirmingRemoval = nil }
            }
            Button("Cancel", role: .cancel) { confirmingRemoval = nil }
        } message: { Text("This phone will lose access to your agents and notifications.") }
    }

    private var email: String { account.session?.email ?? "the same October account as this computer" }
    private var install: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("1 · Get the October app").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
            Text("Install the October iPhone beta with TestFlight, then sign in to the phone with \(email).")
                .foregroundStyle(Theme.muted).textSelection(.enabled)
            if let url = model.state?.installUrl {
                Button("Get the October app") { NSWorkspace.shared.open(url) }.buttonStyle(SecondaryButtonStyle())
            } else {
                Text("Loading the October app's install link…").foregroundStyle(Theme.muted)
            }
            Text("Already have the app? Sign in with the account above, then show your pairing code.")
                .foregroundStyle(Theme.muted)
            Button("Show pairing code") { model.pair() }
                .buttonStyle(AmberButtonStyle()).disabled(model.busy || model.state == nil)
            Button("Cancel") { model.setup = false }.buttonStyle(SecondaryButtonStyle()).disabled(model.busy)
        }
    }

    private func scan(_ p: PhoneModel.Pairing) -> some View {
        TimelineView(.periodic(from: .now, by: 1)) { tick in
            VStack(alignment: .leading, spacing: 10) {
                Text("2 · Scan to pair").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                if p.remaining(at: tick.date) > 0 {
                    Text("Open your iPhone Camera and point it here").font(.system(size: 12.5, weight: .medium)).foregroundStyle(Theme.ink)
                    if let qr = p.qr, let image = Self.qrImage(qr) {
                        Image(nsImage: image).interpolation(.none).resizable().frame(width: 240, height: 240)
                            .padding(10).background(.white, in: RoundedRectangle(cornerRadius: 10))
                            .frame(maxWidth: .infinity).accessibilityLabel("Pair Lantern with the October phone app")
                    }
                    Text("Or use the scanner in the October app, or paste the pairing link.").foregroundStyle(Theme.muted)
                    Text("Expires in \(p.remaining(at: tick.date) / 60):\(String(format: "%02d", p.remaining(at: tick.date) % 60))")
                        .monospacedDigit().foregroundStyle(Theme.muted)
                    Button(copied ? "Pairing link copied" : "Copy pairing link") { copy(p) }
                        .buttonStyle(SecondaryButtonStyle()).disabled(model.busy)
                } else {
                    Text("This pairing code has expired.").foregroundStyle(Theme.amber)
                }
                Text("Sign in to the phone with \(email). A different account cannot pair.")
                    .foregroundStyle(Theme.muted).textSelection(.enabled)
                HStack {
                    Button("New code") { clearCopiedLink(); model.pair() }.buttonStyle(SecondaryButtonStyle())
                    Button("Cancel") { clearCopiedLink(); model.cancelPairing() }.buttonStyle(SecondaryButtonStyle())
                }.disabled(model.busy)
            }
        }
    }

    private func confirm(_ p: PhoneModel.Pairing, code: String) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("3 · Confirm your phone").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
            Text(p.label ?? "Your phone").font(.system(size: 13, weight: .medium)).foregroundStyle(Theme.ink)
            Text(p.platform == "ios" ? "iPhone" : p.platform ?? "Phone").foregroundStyle(Theme.muted)
            Text(code.prefix(3) + " " + code.suffix(3)).font(.system(size: 32, weight: .semibold, design: .monospaced))
                .foregroundStyle(Theme.ink).textSelection(.enabled)
            Text("Make sure this matches your phone").foregroundStyle(Theme.muted)
            if p.finishing {
                Text("Finishing on your phone…").foregroundStyle(Theme.muted)
            } else {
                HStack {
                    Button("Approve") { model.decide(p, true) }.buttonStyle(AmberButtonStyle())
                    Button("Deny") { model.decide(p, false) }.buttonStyle(SecondaryButtonStyle())
                }.disabled(model.busy)
            }
        }
    }

    @ViewBuilder private var recovery: some View {
        let code = model.state?.errorCode
        let title = PhoneModel.recovery(for: code, outcome: model.state?.outcome)
        if code == "plan_required" {
            // The plan gate above owns its single recovery action.
        } else if code == "SESSION_REQUIRED" {
            Button(title) { account.signOut() }.buttonStyle(SecondaryButtonStyle())
        } else {
            Button(title) {
                model.setup = true
                if title == "New code" || title == "Start again" { model.pair() } else { model.retry() }
            }.buttonStyle(SecondaryButtonStyle()).disabled(model.busy)
        }
    }

    private func deviceRow(_ d: PhoneModel.Device) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 3) {
                Text(d.label).foregroundStyle(Theme.ink).lineLimit(1)
                if d.removalPending {
                    Text("Blocked here. Retry to finish removing it from October.").foregroundStyle(Theme.amber)
                } else if let seen = d.lastSeenAt {
                    Text("Last seen \(Date(timeIntervalSince1970: seen / 1000).formatted(.relative(presentation: .named)))").foregroundStyle(Theme.muted)
                } else {
                    Text("Waiting for the phone to connect").foregroundStyle(Theme.muted)
                }
            }
            Spacer()
            Button(d.removalPending ? "Retry removal" : "Remove") {
                if d.removalPending { model.revoke(d.bind) } else { confirmingRemoval = d }
            }.buttonStyle(SecondaryButtonStyle()).disabled(model.busy)
        }
    }

    private func copy(_ p: PhoneModel.Pairing) {
        guard p.remaining(at: Date()) > 0, let qr = p.qr else { return }
        copyClear?.cancel()
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(qr, forType: .string)
        NSPasteboard.general.setData(Data(), forType: NSPasteboard.PasteboardType("org.nspasteboard.TransientType"))
        copied = true
        // Clear only our own still-current clipboard value. Do not retain the secret in a closure.
        let change = NSPasteboard.general.changeCount
        let work = DispatchWorkItem {
            if NSPasteboard.general.changeCount == change { NSPasteboard.general.clearContents() }
        }
        copyClear = work
        DispatchQueue.main.asyncAfter(deadline: .now() + min(60, Double(p.remaining(at: Date()))), execute: work)
    }
    private func clearCopiedLink() {
        // Scheduled clearing survives closing this panel, but never clears newer clipboard content.
        copied = false
    }
    static func qrImage(_ text: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8); filter.correctionLevel = "M"
        guard let code = filter.outputImage else { return nil }
        let padded = code.composited(over: CIImage(color: .white).cropped(to: code.extent.insetBy(dx: -4, dy: -4)))
            .transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        guard let cg = CIContext().createCGImage(padded, from: padded.extent) else { return nil }
        return NSImage(cgImage: cg, size: NSSize(width: padded.extent.width, height: padded.extent.height))
    }
}
