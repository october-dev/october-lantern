import Foundation

/// Runs `lantern-engine serve` and speaks its JSON-lines protocol. Restarts it if it dies.
@MainActor
final class EngineClient {
    var onAgents: (([Agent]) -> Void)?
    var onReplyResult: ((String, Bool, String?) -> Void)?

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

    private func send(_ obj: [String: String]) {
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
            do {
                switch try JSONDecoder().decode(EngineMessage.self, from: line) {
                case .snapshot(let agents): onAgents?(agents)
                case .replyResult(let id, let ok, _, let message): onReplyResult?(id, ok, message)
                case .hello, .other: break
                }
            } catch {
                NSLog("Lantern: bad engine message: \(error)")
            }
        }
    }
}
