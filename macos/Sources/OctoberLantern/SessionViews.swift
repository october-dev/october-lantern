import AppKit
import SwiftUI

/// Header for the standalone panels (new session, October).
struct PanelTitle: View {
    let title: String
    @ObservedObject var model: AppModel

    var body: some View {
        HStack {
            Text(title.uppercased()).font(.system(size: 11, weight: .semibold)).tracking(0.8).foregroundStyle(Theme.ink)
            Spacer()
            Button { model.panel = nil } label: {
                Image(systemName: "xmark").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.muted)
                    .frame(width: 22, height: 22)
            }
            .buttonStyle(.plain)
            .keyboardShortcut(.cancelAction)
        }
        .padding(.horizontal, 16).padding(.top, 14).padding(.bottom, 10)
    }
}

/// Start a new agent session: pick an agent, a folder, an optional first message, and where it runs.
struct NewSessionView: View {
    @ObservedObject var model: AppModel
    @State private var kind: AgentKind?
    @State private var folder: String?
    @State private var prompt = ""
    @State private var background = false

    private let columns = Array(repeating: GridItem(.flexible(), spacing: 8), count: 4)

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            section("Agent") {
                if model.installedKinds.isEmpty {
                    Text("Looking for installed agents…").font(.system(size: 12)).foregroundStyle(Theme.muted)
                } else {
                    LazyVGrid(columns: columns, spacing: 8) {
                        ForEach(model.installedKinds, id: \.self) { k in
                            AgentTile(kind: k, selected: selectedKind == k) { kind = k }
                        }
                    }
                }
            }

            section("Folder") {
                Menu {
                    ForEach(model.recentFolders, id: \.self) { f in
                        Button(Self.short(f)) { folder = f }
                    }
                    if !model.recentFolders.isEmpty { Divider() }
                    Button("Choose Folder…") { chooseFolder() }
                } label: {
                    HStack {
                        Image(systemName: "folder")
                        Text(selectedFolder.map(Self.short) ?? "Choose a folder…").lineLimit(1).truncationMode(.head)
                        Spacer()
                        Image(systemName: "chevron.up.chevron.down").font(.system(size: 9, weight: .semibold))
                    }
                    .font(.system(size: 12.5))
                    .foregroundStyle(Theme.ink.opacity(0.9))
                    .padding(.horizontal, 12).padding(.vertical, 9)
                    .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.faint))
                    .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Theme.hairline))
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
            }

            section("First message (optional)") {
                TextField("What should it work on?", text: $prompt, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(.system(size: 13))
                    .lineLimit(2...5)
                    .padding(.horizontal, 12).padding(.vertical, 9)
                    .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.faint))
                    .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Theme.hairline))
            }

            section("Open in") {
                HStack(spacing: 8) {
                    Choice(title: "Terminal window", symbol: "macwindow", selected: !background) { background = false }
                    Choice(title: "Background", symbol: "moon", selected: background, disabled: !model.tmuxAvailable) { background = true }
                }
                Text(model.tmuxAvailable
                     ? "Lantern can reply directly to sessions it starts. Background sessions open in Terminal when you click Open."
                     : "Install tmux (brew install tmux) so Lantern can reply directly and run sessions in the background.")
                    .font(.system(size: 11)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
            }

            Button(action: start) {
                HStack {
                    if model.launching { ProgressView().controlSize(.small) }
                    Text(model.launching ? "Starting…" : "Start \(selectedKind?.displayName ?? "session")")
                }
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.black.opacity(0.85))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 9)
                .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.amber.opacity(canStart ? 1 : 0.35)))
            }
            .buttonStyle(.plain)
            .disabled(!canStart)
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 16)
    }

    private var selectedKind: AgentKind? { kind ?? model.installedKinds.first }
    private var selectedFolder: String? { folder ?? model.recentFolders.first }
    private var canStart: Bool { selectedKind != nil && selectedFolder != nil && !model.launching }

    private func start() {
        guard let k = selectedKind, let f = selectedFolder else { return }
        model.launch(kind: k, folder: f, prompt: prompt, background: background)
        prompt = ""
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.level = NSWindow.Level(rawValue: NSWindow.Level.statusBar.rawValue + 1)
        NSApp.activate()
        if panel.runModal() == .OK, let url = panel.url { folder = url.path }
    }

    static func short(_ path: String) -> String {
        let home = NSHomeDirectory()
        return path.hasPrefix(home) ? "~" + path.dropFirst(home.count) : path
    }

    @ViewBuilder
    private func section<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
            content()
        }
    }
}

struct AgentTile: View {
    let kind: AgentKind
    let selected: Bool
    let action: () -> Void
    @State private var hover = false

    var body: some View {
        Button(action: action) {
            VStack(spacing: 5) {
                Group {
                    if let img = Assets.harness(kind) {
                        Image(nsImage: img).resizable().interpolation(.high).aspectRatio(contentMode: .fit)
                            .clipShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
                    } else {
                        Image(systemName: "terminal").font(.system(size: 18))
                    }
                }
                .frame(width: 30, height: 30)
                Text(kind.displayName).font(.system(size: 10.5, weight: .medium)).lineLimit(1)
                    .foregroundStyle(selected ? Theme.ink : Theme.muted)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(selected || hover ? Theme.faint : .clear))
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(selected ? Theme.amber.opacity(0.7) : Theme.hairline, lineWidth: selected ? 1.5 : 1)
            )
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
    }
}

struct Choice: View {
    let title: String
    let symbol: String
    let selected: Bool
    var disabled = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: symbol)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(selected ? Theme.ink : Theme.muted)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(selected ? Theme.faint : .clear))
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .strokeBorder(selected ? Theme.amber.opacity(0.7) : Theme.hairline)
                )
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .opacity(disabled ? 0.4 : 1)
    }
}

/// October: where the connections to the October account and October Desktop will live. Both are
/// locked for now.
struct OctoberView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                OctoberLogo(size: 40)
                VStack(alignment: .leading, spacing: 2) {
                    Text("October").font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.ink)
                    Text("Lantern works on its own. Connecting to October adds more.")
                        .font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
                }
            }
            AccountCard()
            DesktopCard(model: model)
            LockedRow(symbol: "iphone", title: "Connect to October phone app",
                      detail: "Check on your agents and reply to them from your phone.")
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 16)
    }
}

struct OctoberLogo: View {
    var size: CGFloat = 26

    var body: some View {
        Group {
            if let img = Assets.image("october.png") {
                Image(nsImage: img).resizable().interpolation(.high).aspectRatio(contentMode: .fit)
            } else {
                Image(systemName: "leaf.fill").foregroundStyle(.orange)
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: size * 0.24, style: .continuous))
    }
}

struct LockedRow: View {
    let symbol: String
    let title: String
    let detail: String

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: symbol).font(.system(size: 16)).foregroundStyle(Theme.muted).frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.ink.opacity(0.7))
                Text(detail).font(.system(size: 11.5)).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            HStack(spacing: 4) {
                Image(systemName: "lock.fill").font(.system(size: 9))
                Text("Soon").font(.system(size: 10.5, weight: .semibold))
            }
            .foregroundStyle(Theme.muted)
            .padding(.horizontal, 8).padding(.vertical, 3)
            .background(Capsule().fill(Theme.faint))
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.faint))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Theme.hairline))
        .help("Coming soon")
    }
}
