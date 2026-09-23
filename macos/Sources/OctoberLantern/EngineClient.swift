import Foundation

/// Runs `lantern-engine serve` and speaks its JSON-lines protocol. Restarts it if it dies.
@MainActor
final class EngineClient {
    var onAgents: (([Agent]) -> Void)?
    var onReplyResult: ((String, Bool, String?) -> Void)?
    var onInstalled: ((EngineMessage.Installed) -> Void)?
    var onHistory: ((String, Bool, [ChatMessage]) -> Void)?
    /// Launch and attach results: (ok, message).
    var onActionResult: ((Bool, String?) -> Void)?

    private var process: Process?
    private var stdin: FileHandle?
    private var buffer = Data()
    private var stopping = false

    static func engineURL() -> URL? {
        let fm = FileManager.default
        if let env = ProcessInfo.processInfo.environment["LANTERN_ENGINE"], fm.isExecutableFile(atPath: env) {
            return URL(fileURLWithPath: env)
        }
        if let bundled = Bundle.main.url(forAuxiliaryExecutable: "lantern-engine"),
           fm.isExecutableFile(atPath: bundled.path) {
            return bundled
        }
        // Development: walk up from the executable to the repo and use the cargo build.
        var dir = URL(fileURLWithPath: CommandLine.arguments[0]).resolvingSymlinksInPath().deletingLastPathComponent()
        for _ in 0..<8 {
            for profile in ["release", "debug"] {
                let candidate = dir.appendingPathComponent("engine/target/\(profile)/lantern-engine")
                if fm.isExecutableFile(atPath: candidate.path) { return candidate }
            }
            dir.deleteLastPathComponent()
        }
        return nil
    }

    func start() {
        stopping = false
        guard let url = Self.engineURL() else {
            NSLog("Lantern: lantern-engine not found; set LANTERN_ENGINE")
            return
        }
        let p = Process()
        p.executableURL = url
        p.arguments = ["serve"]
        let input = Pipe()
        let output = Pipe()
        p.standardInput = input
        p.standardOutput = output
        p.standardError = FileHandle.standardError
        output.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            Task { @MainActor in self?.receive(data) }
        }
        p.terminationHandler = { [weak self] _ in
            Task { @MainActor in
                guard let self, !self.stopping else { return }
                try? await Task.sleep(for: .seconds(2))
                self.start()
            }
        }
        do {
            try p.run()
            process = p
            stdin = input.fileHandleForWriting
        } catch {
            NSLog("Lantern: failed to start engine: \(error)")
        }
    }

    func stop() {
        stopping = true
        process?.terminate()
    }

    func refresh() {
        send(["type": "refresh"])
    }

    func reply(requestId: String, agentId: String, text: String) {
        send(["type": "reply", "requestId": requestId, "agentId": agentId, "text": text])
    }

    func launch(requestId: String, kind: AgentKind, cwd: String, prompt: String, background: Bool) {
        send(["type": "launch", "requestId": requestId, "kind": kind.rawValue, "cwd": cwd, "prompt": prompt, "background": background])
    }

    func history(agentId: String) {
        send(["type": "history", "requestId": "h-\(agentId)", "agentId": agentId])
    }

    func keys(requestId: String, agentId: String, keys: [String]) {
        send(["type": "keys", "requestId": requestId, "agentId": agentId, "keys": keys])
    }

    func focus(requestId: String, agentId: String) {
        send(["type": "focus", "requestId": requestId, "agentId": agentId])
    }

    // MARK: Phone

    /// Phone host state from the engine (`{"type":"phone",...}`), decoded by `PhoneModel`.
    var onPhone: ((Data) -> Void)?

    /// `phone.start` / `phone.token` / `phone.pair` / `phone.decide` / `phone.revoke` / `phone.stop`.
    func phone(_ obj: [String: Any]) {
        send(obj)
    }

    private func send(_ obj: [String: Any]) {
        guard let stdin, var data = try? JSONSerialization.data(withJSONObject: obj) else { return }
        data.append(0x0A)
        try? stdin.write(contentsOf: data)
    }

    private func receive(_ data: Data) {
        guard !data.isEmpty else { return }
        buffer.append(data)
        while let newline = buffer.firstIndex(of: 0x0A) {
            let line = buffer[buffer.startIndex..<newline]
            buffer.removeSubrange(buffer.startIndex...newline)
            guard !line.isEmpty else { continue }
            // MARK: Phone
            if line.range(of: Data(#""type":"phone""#.utf8)) != nil {
                onPhone?(Data(line))
                continue
            }
            do {
                switch try JSONDecoder().decode(EngineMessage.self, from: line) {
                case .snapshot(let agents): onAgents?(agents)
                case .replyResult(let id, let ok, _, let message): onReplyResult?(id, ok, message)
                case .installed(let installed): onInstalled?(installed)
                case .history(let agentId, let supported, let messages): onHistory?(agentId, supported, messages)
                case .launchResult(_, let ok, let message), .attachResult(_, let ok, let message):
                    onActionResult?(ok, message)
                case .hello, .other: break
                }
            } catch {
                NSLog("Lantern: bad engine message: \(error)")
            }
        }
    }
}
