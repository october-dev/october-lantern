import SwiftUI

/// October panel: account, phone setup and the optional Desktop connection.
struct OctoberView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var account = OctoberAccount.shared
    @ObservedObject var phone = PhoneModel.shared
    @ObservedObject private var metrics = PanelMetrics.shared

    var body: some View {
        ScrollViewReader { scroll in
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    ConnectionsHeader()
                    AccountCard(account: account)
                    if account.signedIn {
                        PhoneCard(model: phone, account: account).id("phone")
                    } else {
                        Text("Your phone uses the same October account. Sign in above to connect it.")
                            .font(.system(size: 11.5)).foregroundStyle(Theme.muted)
                    }
                    DesktopCard(model: model, compact: true)
                }
                .padding(.horizontal, 16).padding(.bottom, 16)
            }
            .onAppear { if phone.setup { scroll.scrollTo("phone", anchor: .top) } }
            .onChange(of: phone.setup) { _, setup in if setup { scroll.scrollTo("phone", anchor: .top) } }
            .onChange(of: phone.state?.pairing?.intentId) { _, intent in if intent != nil { scroll.scrollTo("phone", anchor: .top) } }
            .onChange(of: phone.state?.pairing?.code) { _, code in if code != nil { scroll.scrollTo("phone", anchor: .top) } }
        }
        .frame(maxHeight: max(200, metrics.maxHeight - 60))
        .fixedSize(horizontal: false, vertical: true)
    }
}

struct ConnectionsHeader: View {
    var body: some View {
        HStack(spacing: 12) {
            ZStack(alignment: .bottomTrailing) {
                if let logo = Assets.logo {
                    Image(nsImage: logo).resizable().interpolation(.high).scaledToFit().frame(width: 48, height: 48)
                }
                OctoberLogo(size: 18).padding(2).background(Color(white: 0.13), in: RoundedRectangle(cornerRadius: 6)).offset(x: 3, y: 1)
            }
            VStack(alignment: .leading, spacing: 5) {
                Text("Keep your agents close.")
                    .font(.system(size: 19, weight: .semibold, design: .rounded)).foregroundStyle(Theme.ink)
                Text("Ask on your Mac. Reply from your phone.")
                    .font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 5)
    }
}

/// Decorative, bundled October app screenshots. Never contains live pairing links or account data.
struct OctoberPhoneArtwork: View {
    var body: some View {
        ZStack {
            Ellipse().fill(Theme.amber.opacity(0.12)).frame(width: 210, height: 110).blur(radius: 22)
            screenshot("phone-dashboard", width: 64).rotationEffect(.degrees(-10)).offset(x: -65, y: 10)
            screenshot("phone-agent-network", width: 64).rotationEffect(.degrees(10)).offset(x: 65, y: 10)
            screenshot("phone-agent-chat", width: 76)
        }
        .frame(maxWidth: .infinity).frame(height: 180)
        .accessibilityHidden(true)
        .allowsHitTesting(false)
    }

    private func screenshot(_ name: String, width: CGFloat) -> some View {
        Group {
            if let image = Assets.image("october/\(name).jpg") {
                Image(nsImage: image).resizable().interpolation(.high).scaledToFit()
            }
        }
        .frame(width: width, height: width * 520 / 240)
        .background(.black, in: RoundedRectangle(cornerRadius: 11))
        .clipShape(RoundedRectangle(cornerRadius: 11))
        .overlay(RoundedRectangle(cornerRadius: 11).strokeBorder(Color.white.opacity(0.2)))
        .shadow(color: .black.opacity(0.3), radius: 8, y: 5)
    }
}
