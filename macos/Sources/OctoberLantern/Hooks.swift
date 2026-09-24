import AppKit

/// The optional agent hooks (installed by the engine; see engine/src/hooks.rs).
@MainActor
final class Hooks: ObservableObject {
    static let shared = Hooks()
    @Published private(set) var claude = false
    @Published private(set) var codex = false
    var installed: Bool { claude || codex }

    func refresh() {
        let out = run(["hooks", "status"])
        claude = out.contains("\"claude\":true")
        codex = out.contains("\"codex\":true")
    }

    /// Installs or removes the hooks and returns the engine's last line of output.
    @discardableResult
    func set(_ on: Bool) -> String {
        let out = run(["hooks", on ? "install" : "uninstall"])
        refresh()
        return out.split(separator: "\n").last.map(String.init) ?? ""
    }

    private func run(_ args: [String]) -> String {
        guard let engine = EngineClient.engineURL() else { return "" }
        let p = Process()
        p.executableURL = engine
        p.arguments = args
        let out = Pipe()
        p.standardOutput = out
        p.standardError = out
        do { try p.run() } catch { return "Couldn't run Lantern's engine: \(error.localizedDescription)" }
        // Read before waiting: a process that fills the pipe would otherwise never exit.
        let data = out.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        return String(data: data, encoding: .utf8) ?? ""
    }
}
