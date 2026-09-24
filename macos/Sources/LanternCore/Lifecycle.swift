import Foundation

/// Identifies one attempt at something asynchronous (a sign-in, a dictation authorization). Work
/// that awaited checks `isCurrent` before acting; cancelling or starting again makes older attempts
/// stale, so their late answers change nothing.
public struct Attempts: Equatable {
    public private(set) var current = 0
    public private(set) var active = false

    public init() {}

    public mutating func begin() -> Int {
        current += 1
        active = true
        return current
    }

    /// Cancels whatever attempt is running.
    public mutating func cancel() {
        current += 1
        active = false
    }

    /// The attempt finished; it stays current (its result was accepted) but is no longer running.
    public mutating func finish(_ id: Int) {
        if id == current { active = false }
    }

    public func isCurrent(_ id: Int) -> Bool { active && id == current }
}

/// Decides when the engine process is (re)started. Every launch gets a generation; anything a
/// process reports carries its generation, and only the current one counts. `stop` makes every
/// earlier generation stale, so a restart already waiting out its delay does nothing.
public struct Supervisor: Equatable {
    public private(set) var generation = 0
    public private(set) var wantsRunning = false
    /// Consecutive short-lived runs, for backoff.
    public private(set) var failures = 0

    /// A run at least this long counts as healthy and resets the backoff.
    public static let healthyRun: TimeInterval = 30
    public static let maxDelay: TimeInterval = 30

    public init() {}

    /// Start (or restart) now. Returns the new process's generation.
    public mutating func start() -> Int {
        wantsRunning = true
        failures = 0
        generation += 1
        return generation
    }

    /// Stop, and don't restart.
    public mutating func stop() {
        wantsRunning = false
        generation += 1
    }

    public func isCurrent(_ generation: Int) -> Bool { generation == self.generation }

    /// The process of `generation` exited after running `ranFor` seconds. Returns how long to wait
    /// before restarting, or nil when it shouldn't be restarted (stale, or stopped).
    public mutating func exited(_ generation: Int, ranFor: TimeInterval) -> TimeInterval? {
        guard wantsRunning, isCurrent(generation) else { return nil }
        failures = ranFor >= Self.healthyRun ? 1 : failures + 1
        return min(Self.maxDelay, pow(2, Double(failures)))
    }

    /// After the delay: may the exited process of `generation` be replaced? If so, returns the new
    /// generation.
    public mutating func relaunch(after generation: Int) -> Int? {
        guard wantsRunning, isCurrent(generation) else { return nil }
        self.generation += 1
        return self.generation
    }
}
