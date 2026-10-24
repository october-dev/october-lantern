import Foundation

/// The app's own log, for Send Logs: logs/app.log in Lantern's support folder (the engine's is
/// engine.log). A line per event worth knowing about afterwards, never conversations or keys.
/// Over 1 MB it starts again, the old one kept as app-previous.log.
enum AppLog {
    private static let queue = DispatchQueue(label: "dev.october.lantern.applog")
    private static var logs: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/October Lantern/logs", isDirectory: true)
    }

    static func write(_ line: String) {
        let stamp = ISO8601DateFormatter().string(from: Date())
        let text = "\(stamp) \(line)\n"
        queue.async {
            let fm = FileManager.default
            let log = logs.appendingPathComponent("app.log")
            try? fm.createDirectory(at: logs, withIntermediateDirectories: true)
            if let size = (try? fm.attributesOfItem(atPath: log.path))?[.size] as? Int, size > 1_000_000 {
                let previous = logs.appendingPathComponent("app-previous.log")
                try? fm.removeItem(at: previous)
                try? fm.moveItem(at: log, to: previous)
            }
            if !fm.fileExists(atPath: log.path) {
                fm.createFile(atPath: log.path, contents: nil, attributes: [.posixPermissions: 0o600])
            }
            guard let handle = try? FileHandle(forWritingTo: log) else { return }
            defer { try? handle.close() }
            _ = try? handle.seekToEnd()
            try? handle.write(contentsOf: Data(text.utf8))
        }
    }
}
