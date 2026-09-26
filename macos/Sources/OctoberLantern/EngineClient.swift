import Foundation
import LanternCore

/// Where the engine process is in its life.
enum EngineHealth: Equatable {
    case starting, ready, restarting
    /// Stopped for good, with the reason (missing or incompatible engine).
    case failed(String)
}

/// Runs `lantern-engine serve` and speaks its JSON-lines protocol. Restarts it if it dies.
@MainActor
final class EngineClient {
    /// The protocol this app speaks (protocol/README.md). An engine that says otherwise isn't used.
    static let protocolVersion = 3

    var onAgents: (([Agent]) -> Void)?
    /// (request id, ok, error code, message). Code "uncertain": typing started but may not have
    /// gone in.
    var onReplyResult: ((String, Bool, String?, String?) -> Void)?
    var onInstalled: ((EngineMessage.Installed) -> Void)?
    var onModels: ((EngineMessage.ModelList) -> Void)?
    var onHistory: ((String, Bool, [ChatMessage]) -> Void)?
    var onOctober: ((OctoberLink) -> Void)?
    var onPhone: ((PhoneModel.State) -> Void)?
    /// Launch and attach results: (requestId, ok, message).
    var onActionResult: ((String, Bool, String?) -> Void)?
    /// A session started on Lantern's tmux server, by tmux session name.
    var onLaunched: ((String) -> Void)?
    /// A (new) engine process said hello and is ready for requests.
    var onReady: (() -> Void)?
    /// The engine process ended; anything outstanding won't be answered.
    var onStopped: ((String) -> Void)?

    /// Where the engine is in its life, for the UI.
    var onHealth: ((EngineHealth) -> Void)?

    private var process: Process?
    private var stdin: FileHandle?
    private var buffer = Data()
    private var supervisor = Supervisor()
    private var historyRequests = HistoryRequests()
    /// The current process said a compatible hello; until then nothing else it says is used and no
    /// request is sent to it.
    private var ready = false
    private var launchedAt = Date()
    private var restartTask: Task<Void, Never>?

    nonisolated static func engineURL() -> URL? {
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

    /// Starts the engine and keeps it running until `stop`.
    func start() {
        restartTask?.cancel()
        launch(supervisor.start())
    }

    /// Stops the engine for good (until `start`). A restart waiting out its delay is dropped, and
    /// nothing the old process still says is used.
    func stop() {
        supervisor.stop()
        restartTask?.cancel()
        restartTask = nil
        shutDown()
    }

    private func launch(_ generation: Int) {
        historyRequests.cancelAll()
        buffer = Data()
        ready = false
        onHealth?(.starting)
        guard let url = Self.engineURL() else {
            supervisor.stop()
            onHealth?(.failed("Lantern's engine is missing. Reinstall October Lantern."))
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
            Task { @MainActor in self?.receive(data, from: generation) }
        }
        p.terminationHandler = { [weak self] _ in
            Task { @MainActor in self?.exited(generation) }
        }
        do {
            try p.run()
            process = p
            stdin = input.fileHandleForWriting
            launchedAt = Date()
        } catch {
            output.fileHandleForReading.readabilityHandler = nil
            process = nil
            exited(generation, message: "Couldn't start Lantern's engine: \(error.localizedDescription)")
        }
    }

    private func exited(_ generation: Int, message: String = "Lantern's engine stopped; restarting it.") {
        guard supervisor.isCurrent(generation) else { return }
        process = nil
        stdin = nil
        ready = false
        guard let delay = supervisor.exited(generation, ranFor: Date().timeIntervalSince(launchedAt)) else { return }
        onStopped?(message)
        onHealth?(.restarting)
        restartTask?.cancel()
        restartTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(delay))
            guard let self, !Task.isCancelled, let next = self.supervisor.relaunch(after: generation) else { return }
            self.launch(next)
        }
    }

    /// Ends the current process without treating it as a crash.
    private func shutDown() {
        historyRequests.cancelAll()
        ready = false
        stdin = nil
        buffer = Data()
        process?.terminate()
        process = nil
    }

    @discardableResult
    func refresh() -> Bool {
        send(["type": "refresh"])
    }

    @discardableResult
    func reply(requestId: String, agentId: String, text: String) -> Bool {
        send(["type": "reply", "requestId": requestId, "agentId": agentId, "text": text])
    }

    @discardableResult
    func launch(
        requestId: String, kind: AgentKind, cwd: String, prompt: String, screenshot: String?, model: String?, context: String?, toolkit: Bool,
        bus: Bool, background: Bool
    ) -> Bool {
        var request: [String: Any] = [
            "type": "launch", "requestId": requestId, "kind": kind.rawValue, "cwd": cwd, "prompt": prompt, "background": background,
            "toolkit": toolkit, "bus": bus,
            "timeoutMs": Int(AppModel.launchDeadline * 1000),
        ]
        if let context { request["context"] = context }
        if let screenshot { request["screenshot"] = screenshot }
        if let model { request["model"] = model }
        return send(request)
    }

    /// Downloads October Bus in the background if it isn't here yet.
    @discardableResult
    func prepareBus() -> Bool {
        send(["type": "bus.prepare"])
    }

    @discardableResult
    func refreshToolkit() -> Bool {
        send(["type": "toolkit.refresh"])
    }

    @discardableResult
    func models(kind: AgentKind) -> Bool {
        send(["type": "models", "kind": kind.rawValue])
    }

    @discardableResult
    func history(agentId: String) -> Bool {
        guard let id = historyRequests.begin(for: agentId) else { return true }
        let sent = send(["type": "history", "requestId": id, "agentId": agentId])
        if !sent { historyRequests.finish(id, for: agentId) }
        return sent
    }

    func cancelHistory() { historyRequests.cancelAll() }

    @discardableResult
    func keys(requestId: String, agentId: String, keys: [String], promptId: String?) -> Bool {
        var request: [String: Any] = ["type": "keys", "requestId": requestId, "agentId": agentId, "keys": keys]
        if let promptId { request["promptId"] = promptId }
        return send(request)
    }

    /// "october.pair", "october.cancelPair" or "october.forget".
    @discardableResult
    func october(_ type: String) -> Bool {
        send(["type": type])
    }

    @discardableResult
    func focus(requestId: String, agentId: String) -> Bool {
        send(["type": "focus", "requestId": requestId, "agentId": agentId])
    }

    // MARK: Phone (see PhoneModel)

    /// The current October access token: the engine starts hosting for the phone app, or hands a
    /// running host the refreshed token.
    @discardableResult func phoneToken(_ accessToken: String) -> Bool { send(["type": "phone.token", "accessToken": accessToken]) }
    @discardableResult func phonePair() -> Bool { send(["type": "phone.pair"]) }
    @discardableResult func phoneCancelPair() -> Bool { send(["type": "phone.cancelPair"]) }
    @discardableResult func phoneDecide(allow: Bool) -> Bool { send(["type": "phone.decide", "allow": allow]) }
    @discardableResult func phoneRevoke(bind: String) -> Bool { send(["type": "phone.revoke", "bind": bind]) }
    @discardableResult func phoneStop() -> Bool { send(["type": "phone.stop"]) }

    /// Writes one request. False when there's no ready engine to take it (the caller fails the
    /// request instead of waiting for an answer that can't come).
    @discardableResult
    private func send(_ obj: [String: Any]) -> Bool {
        guard ready, let stdin, var data = try? JSONSerialization.data(withJSONObject: obj) else { return false }
        data.append(0x0A)
        do {
            try stdin.write(contentsOf: data)
            return true
        } catch {
            // The pipe is broken: end this process; its exit restarts the engine.
            process?.terminate()
            return false
        }
    }

    private func receive(_ data: Data, from generation: Int) {
        guard !data.isEmpty, supervisor.isCurrent(generation) else { return }
        buffer.append(data)
        while supervisor.isCurrent(generation), let newline = buffer.firstIndex(of: 0x0A) {
            let line = buffer[buffer.startIndex..<newline]
            buffer.removeSubrange(buffer.startIndex...newline)
            guard !line.isEmpty else { continue }
            do {
                let message = try JSONDecoder().decode(EngineMessage.self, from: line)
                if case .hello(let protocolVersion, let version) = message {
                    if protocolVersion == Self.protocolVersion {
                        ready = true
                        onHealth?(.ready)
                        onReady?()
                    } else {
                        // Nothing else this engine says is used, and it isn't restarted.
                        let text = "Lantern's engine (\(version)) doesn't match this app. Reinstall October Lantern."
                        stop()
                        onHealth?(.failed(text))
                        onStopped?(text)
                    }
                    continue
                }
                guard ready else { continue }
                switch message {
                case .hello: break
                case .snapshot(let agents): onAgents?(agents)
                case .replyResult(let id, let ok, let error, let message): onReplyResult?(id, ok, error, message)
                case .installed(let installed): onInstalled?(installed)
                case .models(let list): onModels?(list)
                case .history(let id, let agentId, let supported, let messages, let busy):
                    if historyRequests.finish(id, for: agentId), !busy { onHistory?(agentId, supported, messages) }
                case .october(let link): onOctober?(link)
                case .phone(let state): onPhone?(state)
                case .launchResult(let id, let ok, let message, let session):
                    onActionResult?(id, ok, message)
                    if ok, let session { onLaunched?(session) }
                case .attachResult(let id, let ok, let message):
                    onActionResult?(id, ok, message)
                case .other: break
                }
            } catch {
                NSLog("Lantern: bad engine message: \(error)")
            }
        }
    }
}
