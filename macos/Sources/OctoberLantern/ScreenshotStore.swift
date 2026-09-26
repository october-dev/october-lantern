import Foundation

/// Project screenshots are written only after Git confirms the destination is ignored. Git
/// resolves worktrees and nested working directories; all failures use private app storage.
enum ScreenshotStore {
    static func save(_ png: Data, cwd: URL?, fallback: URL) -> URL? {
        let fm = FileManager.default
        let name = "point-\(UUID().uuidString).png"
        var directory = fallback
        if let cwd {
            let root = cwd.resolvingSymlinksInPath()
            let proposed = root.appendingPathComponent(".lantern/shots", isDirectory: true)
            if proposed.resolvingSymlinksInPath().path.hasPrefix(root.path + "/"), excludeFromGit(root),
               git(["check-ignore", "--quiet", "--", ".lantern/shots/" + name], at: root)?.status == 0 {
                directory = proposed
            }
        }
        do {
            try fm.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            let url = directory.appendingPathComponent(name)
            if fm.createFile(atPath: url.path, contents: png, attributes: [.posixPermissions: 0o600]) { return url }
            return directory == fallback ? nil : save(png, cwd: nil, fallback: fallback)
        } catch {
            // A protected repository may let us update its exclude but not create a directory.
            if directory != fallback { return save(png, cwd: nil, fallback: fallback) }
            return nil
        }
    }

    @discardableResult
    static func excludeFromGit(_ cwd: URL) -> Bool {
        if git(["check-ignore", "--quiet", "--", ".lantern/shots/" + UUID().uuidString], at: cwd)?.status == 0 { return true }
        guard let result = git(["rev-parse", "--path-format=absolute", "--git-path", "info/exclude"], at: cwd),
              result.status == 0, result.output.hasPrefix("/") else { return false }
        let exclude = URL(fileURLWithPath: result.output)
        do {
            let fm = FileManager.default
            try fm.createDirectory(at: exclude.deletingLastPathComponent(), withIntermediateDirectories: true)
            let current = fm.fileExists(atPath: exclude.path) ? try String(contentsOf: exclude, encoding: .utf8) : ""
            if !current.split(separator: "\n").contains(where: { $0 == ".lantern/" }) {
                let line = (current.isEmpty || current.hasSuffix("\n") ? "" : "\n") + "# October Lantern screenshots\n.lantern/\n"
                if fm.fileExists(atPath: exclude.path) {
                    let handle = try FileHandle(forWritingTo: exclude)
                    defer { try? handle.close() }
                    try handle.seekToEnd()
                    try handle.write(contentsOf: Data(line.utf8))
                } else {
                    try line.write(to: exclude, atomically: true, encoding: .utf8)
                }
            }
            return git(["check-ignore", "--quiet", "--", ".lantern/shots/" + UUID().uuidString], at: cwd)?.status == 0
        } catch { return false }
    }

    private static func git(_ arguments: [String], at cwd: URL) -> (status: Int32, output: String)? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = ["-c", "core.hooksPath=/dev/null", "-C", cwd.path] + arguments
        let output = Pipe()
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        guard (try? process.run()) != nil else { return nil }
        let deadline = Date().addingTimeInterval(2)
        while process.isRunning && Date() < deadline { usleep(10_000) }
        guard !process.isRunning else { process.terminate(); return nil }
        return (process.terminationStatus, String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines))
    }
}
