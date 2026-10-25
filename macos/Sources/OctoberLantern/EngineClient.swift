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
    /// October Harness's status (with `installed`, and in answer to `prepareHarness`).
    var onHarness: ((EngineMessage.HarnessStatus) -> Void)?
    var onComputer: ((EngineMessage.ComputerStatus) -> Void)?
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

    /// The engine's log, for Send Logs: logs/engine.log in Lantern's support folder, the last
    /// run's kept as engine-previous.log so why an engine stopped survives its restart.
    private static func engineLog() -> FileHandle? {
        let logs = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/October Lantern/logs", isDirectory: true)
        let log = logs.appendingPathComponent("engine.log")
        let previous = logs.appendingPathComponent("engine-previous.log")
        try? FileManager.default.createDirectory(at: logs, withIntermediateDirectories: true)
        try? FileManager.default.removeItem(at: previous)
        try? FileManager.default.moveItem(at: log, to: previous)
        guard FileManager.default.createFile(atPath: log.path, contents: nil) else { return nil }
        return try? FileHandle(forWritingTo: log)
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
        p.standardError = Self.engineLog() ?? FileHandle.standardError
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
        requestId: String, kind: AgentKind, cwd: String, prompt: String, screenshot: String?, images: [String] = [], model: String?,
        context: String?, toolkit: Bool, bus: Bool, computer: Bool, askFirst: Bool, background: Bool
    ) -> Bool {
        var request: [String: Any] = [
            "type": "launch", "requestId": requestId, "kind": kind.rawValue, "cwd": cwd, "prompt": prompt, "background": background,
            "toolkit": toolkit, "bus": bus, "computer": computer, "askFirst": askFirst,
            "timeoutMs": Int(AppModel.launchDeadline * 1000),
        ]
        if let context { request["context"] = context }
        if let screenshot { request["screenshot"] = screenshot }
        if !images.isEmpty { request["images"] = images }
        if let model { request["model"] = model }
        return send(request)
    }

    /// Turns Cua Driver on (downloads it if needed and starts Lantern's driver) or off; answered
    /// with `computer` messages.
    @discardableResult
    func prepareComputer(enabled: Bool, restart: Bool = false) -> Bool {
        send(["type": "computer.prepare", "enabled": enabled, "restart": restart])
    }

    /// Downloads October Bus in the background if it isn't here yet.
    @discardableResult
    func prepareBus() -> Bool {
        send(["type": "bus.prepare"])
    }

    /// Installs October Harness in the background if `october` isn't on PATH and it isn't here yet
    /// (answered with `harness`, now and when it's ready or has failed).
    @discardableResult
    func prepareHarness() -> Bool {
        send(["type": "harness.prepare"])
    }

    /// The October sign-in for October launches (memory only in the engine); nil when signed out.
    /// Never the refresh token.
    @discardableResult
    func octoberSession(accessToken: String?, expiresAt: Date?) -> Bool {
        var request: [String: Any] = ["type": "october.session"]
        if let accessToken, let expiresAt {
            request["accessToken"] = accessToken
            request["expiresAt"] = Int(expiresAt.timeIntervalSince1970)
        }
        return send(request)
    }

    /// Looks for installed agents again (answered with `installed`).
    @discardableResult
    func refreshInstalled() -> Bool {
        send(["type": "installed.refresh"])
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

    /// Answers an agent's question; the engine presses the terminal's keys for it.
    @discardableResult
    func answer(requestId: String, agentId: String, promptId: String, answer: AgentAnswer) -> Bool {
        send(["type": "answer", "requestId": requestId, "agentId": agentId, "promptId": promptId, "answer": answer.json])
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
                case .installed(let installed, let harness):
                    onInstalled?(installed)
                    if let harness { onHarness?(harness) }
                case .harness(let status): onHarness?(status)
                case .computer(let status): onComputer?(status)
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
