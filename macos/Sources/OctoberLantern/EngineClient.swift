import Foundation

/// Runs `lantern-engine serve` and speaks its JSON-lines protocol. Restarts it if it dies.
@MainActor
final class EngineClient {
    /// The protocol this app speaks (protocol/README.md). An engine that says otherwise isn't used.
    static let protocolVersion = 2

    var onAgents: (([Agent]) -> Void)?
    var onReplyResult: ((String, Bool, String?) -> Void)?
    var onInstalled: ((EngineMessage.Installed) -> Void)?
    var onHistory: ((String, Bool, [ChatMessage]) -> Void)?
    var onOctober: ((OctoberLink) -> Void)?
    var onPhone: ((PhoneModel.State) -> Void)?
    /// Launch and attach results: (requestId, ok, message).
    var onActionResult: ((String, Bool, String?) -> Void)?
    /// A (new) engine process said hello and is ready for requests.
    var onReady: (() -> Void)?
    /// The engine process ended; anything outstanding won't be answered.
    var onStopped: ((String) -> Void)?

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
        buffer = Data()
        guard let url = Self.engineURL() else {
            onStopped?("Lantern's engine is missing. Reinstall October Lantern.")
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
            // At end of file the handler keeps firing with no data until it's removed.
            if data.isEmpty { handle.readabilityHandler = nil; return }
            Task { @MainActor in self?.receive(data) }
        }
        p.terminationHandler = { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                self.stdin = nil
                if self.stopping { return }
                self.onStopped?("Lantern's engine stopped; restarting it.")
                try? await Task.sleep(for: .seconds(2))
                self.start()
            }
        }
        do {
            try p.run()
            process = p
            stdin = input.fileHandleForWriting
        } catch {
            onStopped?("Couldn't start Lantern's engine: \(error.localizedDescription)")
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

    /// "october.pair", "october.cancelPair" or "october.forget".
    func october(_ type: String) {
        send(["type": type])
    }

    func focus(requestId: String, agentId: String) {
        send(["type": "focus", "requestId": requestId, "agentId": agentId])
    }

    // MARK: Phone (see PhoneModel)

    /// The current October access token: the engine starts hosting for the phone app, or hands a
    /// running host the refreshed token.
    func phoneToken(_ accessToken: String) { send(["type": "phone.token", "accessToken": accessToken]) }
    func phonePair() { send(["type": "phone.pair"]) }
    func phoneDecide(allow: Bool) { send(["type": "phone.decide", "allow": allow]) }
    func phoneRevoke(bind: String) { send(["type": "phone.revoke", "bind": bind]) }
    func phoneStop() { send(["type": "phone.stop"]) }

    private func send(_ obj: [String: Any]) {
        guard let stdin, var data = try? JSONSerialization.data(withJSONObject: obj) else { return }
        data.append(0x0A)
        do {
            try stdin.write(contentsOf: data)
        } catch {
            onStopped?("Lantern's engine isn't answering.")
        }
    }

    private func receive(_ data: Data) {
        guard !data.isEmpty else { return }
        buffer.append(data)
        while let newline = buffer.firstIndex(of: 0x0A) {
            let line = buffer[buffer.startIndex..<newline]
            buffer.removeSubrange(buffer.startIndex...newline)
            guard !line.isEmpty else { continue }
            do {
                switch try JSONDecoder().decode(EngineMessage.self, from: line) {
                case .hello(let protocolVersion, let version):
                    if protocolVersion == Self.protocolVersion {
                        onReady?()
                    } else {
                        stop()
                        onStopped?("Lantern's engine (\(version)) doesn't match this app. Reinstall October Lantern.")
                    }
                case .snapshot(let agents): onAgents?(agents)
                case .replyResult(let id, let ok, _, let message): onReplyResult?(id, ok, message)
                case .installed(let installed): onInstalled?(installed)
                case .history(let agentId, let supported, let messages): onHistory?(agentId, supported, messages)
                case .october(let link): onOctober?(link)
                case .phone(let state): onPhone?(state)
                case .launchResult(let id, let ok, let message), .attachResult(let id, let ok, let message):
                    onActionResult?(id, ok, message)
                case .other: break
                }
            } catch {
                NSLog("Lantern: bad engine message: \(error)")
            }
        }
    }
}
