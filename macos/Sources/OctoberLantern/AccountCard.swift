import SwiftUI

/// "Connect to October": sign in with an October account, or show who's signed in.
struct AccountCard: View {
    @ObservedObject var account = OctoberAccount.shared
    @State private var useEmail = false
    @State private var email = ""
    @State private var password = ""
    /// Off where the context already says why (Ask Lantern's sign-in): just the buttons.
    var showsHeader = true

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if showsHeader || account.signedIn {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: account.signedIn ? "person.crop.circle.fill.badge.checkmark" : "person.crop.circle")
                    .font(.system(size: 16)).foregroundStyle(account.signedIn ? Theme.green : Theme.muted).frame(width: 22)
                VStack(alignment: .leading, spacing: 2) {
                    Text(account.signedIn ? "Connected to October" : "Connect to October")
                        .font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                    Text(detail).font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                    ForEach(Self.lanternLines(account.plan?.lantern), id: \.self) { line in
                        Text(line).font(.system(size: 11.5)).foregroundStyle(Theme.ink.opacity(0.85))
                    }
                }
                Spacer(minLength: 8)
                if account.signedIn {
                    Button(account.loadingPlan ? "Refreshing…" : "Refresh") { account.refreshPlan() }
                        .buttonStyle(SecondaryButtonStyle()).disabled(account.loadingPlan)
                        .help("Reads your plan again, after buying or starting the trial in the browser")
                    Button("Sign Out") { account.signOut() }.buttonStyle(SecondaryButtonStyle())
                }
            }
            }
            if account.signedIn, Self.offersUpgrade(account.plan) {
                OfferButtons(from: "account")
            }

            if !account.signedIn {
                if account.signingIn && !useEmail {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Finish signing in in your browser…").font(.system(size: 12)).foregroundStyle(Theme.muted)
                        Spacer()
                        Button("Cancel") { account.cancelSignIn() }.buttonStyle(.plain).font(.system(size: 12)).foregroundStyle(Theme.muted)
                    }
                } else if useEmail {
                    VStack(spacing: 6) {
                        field(TextField("Email", text: $email))
                        field(SecureField("Password", text: $password).onSubmit(signInWithEmail))
                        HStack {
                            Button("Back") { useEmail = false }.buttonStyle(.plain).font(.system(size: 12)).foregroundStyle(Theme.muted)
                            Spacer()
                            Button(account.signingIn ? "Signing in…" : "Sign In", action: signInWithEmail)
                                .buttonStyle(AmberButtonStyle())
                                .disabled(email.isEmpty || password.isEmpty || account.signingIn)
                        }
                    }
                } else {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 6) {
                            ForEach(OctoberAccount.Provider.allCases) { p in
                                Button { account.signIn(with: p) } label: {
                                    HStack(spacing: 6) {
                                        ProviderMark(provider: p)
                                        Text(p.label).lineLimit(1).fixedSize()
                                    }
                                    .frame(maxWidth: .infinity)
                                }
                                .buttonStyle(SecondaryButtonStyle())
                            }
                        }
                        Button { useEmail = true } label: {
                            Label("Use email and password", systemImage: "envelope")
                                .font(.system(size: 11.5, weight: .medium)).foregroundStyle(Theme.muted)
                        }
                        .buttonStyle(.plain)
                    }
                }
                if let error = account.error {
                    Text(error).font(.system(size: 11.5)).foregroundStyle(Theme.red).fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(account.signedIn ? Theme.green.opacity(0.35) : Theme.hairline))
    }

    private var detail: String {
        guard let s = account.session else { return "Sign in with your October account." }
        let plan = account.plan.map { " · \($0.plan.capitalized) plan" } ?? ""
        return (s.email ?? "Signed in") + plan
    }

    /// "Lantern Lifetime" or "Pro trial · ends Nov 1", then the daily questions on October's AI.
    static func lanternLines(_ lantern: OctoberAccount.Plan.Lantern?, now: Date = Date()) -> [String] {
        guard let lantern else { return [] }
        var lines: [String] = []
        if lantern.lifetime {
            lines.append("Lantern Lifetime")
        } else if let end = lantern.trialEnd(now: now) {
            lines.append("Pro trial · ends \(end.formatted(date: .abbreviated, time: .omitted))")
        }
        if lantern.dailyQuestions > 0 {
            lines.append("Up to \(lantern.dailyQuestions) question\(lantern.dailyQuestions == 1 ? "" : "s") a day on October's AI")
        }
        return lines
    }

    /// The trial and Lifetime are offered to a free account that has neither.
    static func offersUpgrade(_ plan: OctoberAccount.Plan?, now: Date = Date()) -> Bool {
        guard let plan, plan.plan == "free" else { return false }
        return !(plan.lantern?.lifetime ?? false) && plan.lantern?.trialEnd(now: now) == nil
    }

    private func signInWithEmail() {
        guard !email.isEmpty, !password.isEmpty else { return }
        Task {
            await account.signIn(email: email, password: password)
            if account.signedIn { password = ""; useEmail = false }
        }
    }

    private func field<F: View>(_ f: F) -> some View {
        f.textFieldStyle(.plain).font(.system(size: 13))
            .padding(.horizontal, 10).padding(.vertical, 7)
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Color.white.opacity(0.06)))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Theme.hairline))
    }
}

/// The sign-in provider's logo: Apple's from SF Symbols, Google's and GitHub's from the bundle.
struct ProviderMark: View {
    let provider: OctoberAccount.Provider

    var body: some View {
        Group {
            switch provider {
            case .apple: Image(systemName: "applelogo").font(.system(size: 12, weight: .medium)).offset(y: -1)
            case .google, .github:
                if let img = Assets.image("brands/\(provider.rawValue).svg") {
                    Image(nsImage: img).resizable().interpolation(.high).aspectRatio(contentMode: .fit)
                }
            }
        }
        .frame(width: 13, height: 13)
    }
}
