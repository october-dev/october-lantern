import AppKit

/// The optional agent hooks (installed by the engine; see engine/src/hooks.rs). Claude Code and
/// Codex are reported separately: one can be set up while the other isn't, and an older Lantern's
/// Claude hooks (missing events this version needs) show as needing an update, not as on.
@MainActor
final class Hooks: ObservableObject {
    static let shared = Hooks()
    @Published private(set) var claude = false
    @Published private(set) var claudeOutdated = false
    @Published private(set) var codex = false
    @Published private(set) var busy = false

    /// Fully set up for both agents.
    var installed: Bool { claude && codex }
    /// Some of Lantern's hooks are present.
    var anyInstalled: Bool { claude || claudeOutdated || codex }

    private struct Status: Decodable {
        let claude: Bool
        let claudeOutdated: Bool?
        let codex: Bool
    }

    func refresh() {
        Task {
            let out = await Self.run(["hooks", "status"])
            let line = out.split(separator: "\n").last.map(String.init) ?? ""
            guard let status = try? JSONDecoder().decode(Status.self, from: Data(line.utf8)) else { return }
            claude = status.claude
            claudeOutdated = status.claudeOutdated ?? false
            codex = status.codex
        }
    }

    /// Installs or removes the hooks; returns the engine's last line of output (an error message
    /// when it failed).
    func set(_ on: Bool) async -> String {
        busy = true
        defer { busy = false }
        let out = await Self.run(["hooks", on ? "install" : "uninstall"])
        refresh()
        return out.split(separator: "\n").last.map(String.init) ?? ""
    }

    /// Runs the engine off the main thread.
    nonisolated static func run(_ args: [String]) async -> String {
        await Task.detached {
            guard let engine = EngineClient.engineURL() else { return "Lantern's engine is missing" }
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
            let text = String(data: data, encoding: .utf8) ?? ""
            // Callers look for a last line starting with "Error" to tell failure apart.
            if p.terminationStatus != 0 {
                let last = text.split(separator: "\n").last.map(String.init) ?? "the engine failed"
                return text + (last.hasPrefix("Error") ? "" : "\nError: \(last)")
            }
            return text
        }.value
    }
}
