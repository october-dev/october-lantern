import SwiftUI

/// Your own AI key (OpenAI, Anthropic or OpenRouter), for Ask Lantern, Point & Ask and Show me.
///
/// It lives only in the login Keychain, under its own item (never the October session's), as one
/// JSON value: provider, key and optional model. Each question to October carries it in
/// `X-Lantern-Byok-*` headers; October passes the question on to the provider with it and keeps
/// nothing (October's prompts, pictures and task rules stay the same). It's never in Preferences,
/// logs, Send Logs, problem reports or usage counts. Signing out keeps it (it's yours, not the
/// October account's); Remove key and Uninstall delete it.
@MainActor
final class OwnKey: ObservableObject {
    static let shared = OwnKey()
    nonisolated static let item = Keychain.Item(service: "dev.october.lantern.ai-key", account: "own-key")

    enum Provider: String, CaseIterable, Identifiable, Codable {
        case openai, anthropic, openrouter
        var id: String { rawValue }
        var label: String {
            switch self {
            case .openai: "OpenAI"
            case .anthropic: "Anthropic"
            case .openrouter: "OpenRouter"
            }
        }
    }

    struct Saved: Codable, Equatable, Sendable, CustomStringConvertible, CustomDebugStringConvertible {
        var provider: Provider
        var key: String
        var model: String?

        /// The key's last four characters, to recognise it by.
        var hint: String { "…" + String(key.suffix(4)) }
        var description: String { "OwnKey(\(provider.rawValue), key: <redacted>, model: \(model ?? "default"))" }
        var debugDescription: String { description }
    }

    enum State: Equatable {
        case loading, none, saved(Saved)
        /// The Keychain is locked or refused: nothing is saved or used, and nothing falls back to a file.
        case unavailable
    }

    @Published private(set) var state: State
    @Published private(set) var busy = false
    /// Why the last Save or Remove didn't work.
    @Published var problem: String?

    var saved: Saved? {
        if case .saved(let s) = state { s } else { nil }
    }

    init(load: Bool = true) {
        state = load ? .loading : .none
        guard load else { return }
        Task { await reload() }
    }

    /// For tests and snapshots: a key that's already there, without the Keychain.
    static func detached(_ saved: Saved?) -> OwnKey {
        let k = OwnKey(load: false)
        k.state = saved.map(State.saved) ?? .none
        return k
    }

    /// Reads the Keychain off the main thread: an unlock dialog can wait indefinitely.
    func reload() async {
        let result = await Task.detached { () -> State in
            do {
                guard let data = try Keychain.read(Self.item) else { return .none }
                return (try? JSONDecoder().decode(Saved.self, from: data)).map(State.saved) ?? .none
            } catch {
                return .unavailable
            }
        }.value
        state = result
    }

    func save(provider: Provider, key: String, model: String) async {
        let key = key.trimmingCharacters(in: .whitespacesAndNewlines)
        let model = model.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty, !busy else { return }
        let saved = Saved(provider: provider, key: key, model: model.isEmpty ? nil : model)
        busy = true
        defer { busy = false }
        problem = nil
        let ok = await Task.detached { () -> Bool in
            guard let data = try? JSONEncoder().encode(saved) else { return false }
            return (try? Keychain.write(data, to: Self.item)) != nil
        }.value
        if ok {
            state = .saved(saved)
            Analytics.shared.capture("own_key_saved", ["provider": provider.rawValue])
        } else {
            state = .unavailable
            problem = Self.unavailableMessage
        }
    }

    func remove() async {
        guard !busy else { return }
        busy = true
        defer { busy = false }
        problem = nil
        let ok = await Task.detached { (try? Keychain.remove(Self.item)) != nil }.value
        if ok {
            state = .none
            Analytics.shared.capture("own_key_removed", [:])
        } else {
            problem = "Lantern couldn't remove your key from the Keychain. Unlock it and try again."
        }
    }

    static let unavailableMessage = "The Keychain is locked or unavailable, so Lantern can't save or use your key. Unlock it and try again."

    /// The key that answers, unless the plan says your own key isn't included (then October's AI
    /// answers, and Settings says why). An unknown plan still sends it: October decides.
    nonisolated static func answering(_ saved: Saved?, plan: OctoberAccount.Plan.Lantern?) -> Saved? {
        plan?.byok == false ? nil : saved
    }

    var answering: Saved? { Self.answering(saved, plan: OctoberAccount.shared.plan?.lantern) }

    /// The headers for `POST /api/lantern/ask`: all of them, or none.
    nonisolated static func headers(_ key: Saved?) -> [String: String] {
        guard let key else { return [:] }
        var h = ["X-Lantern-Byok-Provider": key.provider.rawValue, "X-Lantern-Byok-Key": key.key]
        if let model = key.model, !model.isEmpty { h["X-Lantern-Byok-Model"] = model }
        return h
    }
}

/// Settings › October › Your own AI key.
struct OwnKeyCard: View {
    @ObservedObject var ownKey = OwnKey.shared
    @ObservedObject var account = OctoberAccount.shared
    @State private var provider: OwnKey.Provider = .openai
    @State private var key = ""
    @State private var model = ""
    @State private var editing = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "key.fill").font(.system(size: 14))
                    .foregroundStyle(ownKey.saved != nil ? Theme.green : Theme.muted).frame(width: 22)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Your own AI key").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                    Text("Your provider bills you for these questions. Lantern sends your key to October's server only to pass each question on; it's never stored.")
                        .font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                }
            }
            switch ownKey.state {
            case .loading:
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Reading the Keychain…").font(.system(size: 12)).foregroundStyle(Theme.muted)
                }
            case .unavailable:
                Label(OwnKey.unavailableMessage, systemImage: "lock.fill")
                    .font(.system(size: 11.5)).foregroundStyle(Theme.amber).fixedSize(horizontal: false, vertical: true)
                Button("Try Again") { Task { await ownKey.reload() } }.buttonStyle(SecondaryButtonStyle())
            case .saved(let saved) where !editing:
                HStack(spacing: 8) {
                    Text("\(saved.provider.label) key \(saved.hint)").font(.system(size: 12.5, weight: .medium)).foregroundStyle(Theme.ink)
                    if let m = saved.model { Text(m).font(.system(size: 11.5)).foregroundStyle(Theme.muted).lineLimit(1) }
                    Spacer()
                    Button("Edit") { begin(saved) }.buttonStyle(SecondaryButtonStyle())
                    Button("Remove key") { Task { await ownKey.remove() } }.buttonStyle(SecondaryButtonStyle()).disabled(ownKey.busy)
                }
                if account.signedIn, account.plan?.lantern?.byok == false {
                    Text("Your own key needs Lantern Lifetime or the Pro trial; until then October's AI answers.")
                        .font(.system(size: 11.5)).foregroundStyle(Theme.amber).fixedSize(horizontal: false, vertical: true)
                    OfferButtons(from: "own_key")
                }
            default:
                form
            }
            if let problem = ownKey.problem {
                Text(problem).font(.system(size: 11.5)).foregroundStyle(Theme.red).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Theme.hairline))
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 6) {
            Picker("Provider", selection: $provider) {
                ForEach(OwnKey.Provider.allCases) { Text($0.label).tag($0) }
            }
            .pickerStyle(.segmented).labelsHidden()
            field(SecureField("\(provider.label) API key", text: $key).onSubmit(save))
            field(TextField("Model (optional; empty uses October's default)", text: $model).onSubmit(save))
            HStack {
                if editing {
                    Button("Cancel") { editing = false; key = "" }.buttonStyle(.plain).font(.system(size: 12)).foregroundStyle(Theme.muted)
                }
                Spacer()
                Button(ownKey.busy ? "Saving…" : "Save Key", action: save)
                    .buttonStyle(AmberButtonStyle())
                    .disabled(key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || ownKey.busy)
            }
        }
    }

    private func begin(_ saved: OwnKey.Saved) {
        provider = saved.provider
        model = saved.model ?? ""
        key = ""
        editing = true
    }

    private func save() {
        Task {
            await ownKey.save(provider: provider, key: key, model: model)
            if ownKey.saved != nil { key = ""; editing = false }
        }
    }

    private func field<F: View>(_ f: F) -> some View {
        f.textFieldStyle(.plain).font(.system(size: 13))
            .padding(.horizontal, 10).padding(.vertical, 7)
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Color.white.opacity(0.06)))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Theme.hairline))
    }
}
