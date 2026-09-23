import SwiftUI

/// "Connect to October": sign in with an October account, or show who's signed in.
struct AccountCard: View {
    @ObservedObject var account = OctoberAccount.shared
    @State private var useEmail = false
    @State private var email = ""
    @State private var password = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: account.signedIn ? "person.crop.circle.fill.badge.checkmark" : "person.crop.circle")
                    .font(.system(size: 16)).foregroundStyle(account.signedIn ? Theme.green : Theme.muted).frame(width: 22)
                VStack(alignment: .leading, spacing: 2) {
                    Text(account.signedIn ? "Connected to October" : "Connect to October")
                        .font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink)
                    Text(detail).font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
                if account.signedIn {
                    Button("Sign Out") { account.signOut() }.buttonStyle(SecondaryButtonStyle())
                }
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
                    HStack(spacing: 6) {
                        ForEach(OctoberAccount.Provider.allCases) { p in
                            Button(p.label) { account.signIn(with: p) }.buttonStyle(SecondaryButtonStyle())
                        }
                        Button("Email") { useEmail = true }.buttonStyle(SecondaryButtonStyle())
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
