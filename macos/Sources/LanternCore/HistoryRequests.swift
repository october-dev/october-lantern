/// One read per conversation at a time. Request ids never repeat, including after closing a
/// conversation or restarting the engine, so a late result cannot populate a reopened chat.
public struct HistoryRequests {
    private var next = 0
    private var pending: [String: String] = [:]

    public init() {}

    public mutating func begin(for agent: String) -> String? {
        guard pending[agent] == nil else { return nil }
        next += 1
        let id = "history-\(next)"
        pending[agent] = id
        return id
    }

    @discardableResult
    public mutating func finish(_ id: String, for agent: String) -> Bool {
        guard pending[agent] == id else { return false }
        pending[agent] = nil
        return true
    }

    public mutating func cancelAll() { pending.removeAll() }
}
