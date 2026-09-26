@testable import OctoberLantern
import Combine
import XCTest

@MainActor
final class AuditRegressionTests: XCTestCase {
    func testAuditUnchangedHeartbeatShouldNotInvalidateUI() {
        let m = AppModel()
        m.update([])
        var changes = 0
        let subscription = m.objectWillChange.sink { changes += 1 }
        m.update([])
        withExtendedLifetime(subscription) {
            XCTAssertEqual(changes, 0, "An identical snapshot should not redraw or remeasure either window")
        }
    }

    func testAuditOpeningOctoberShouldNotPublishEmptyChatState() {
        let m = AppModel()
        var changes = 0
        let subscription = m.objectWillChange.sink { changes += 1 }
        m.panel = .october
        withExtendedLifetime(subscription) {
            XCTAssertEqual(changes, 1, "A single panel change should not also publish nil chat ID and empty history")
        }
    }
    func testAuditScreenshotExcludedWhenAgentRunsInSubdirectory() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("lantern-audit-exclude-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        XCTAssertEqual(try git(["init", "--quiet"], at: root), 0)
        let cwd = root.appendingPathComponent("src")
        try FileManager.default.createDirectory(at: cwd, withIntermediateDirectories: true)
        ScreenshotStore.excludeFromGit(cwd)
        try assertScreenshotIgnored(in: cwd)
    }

    func testAuditScreenshotExcludedInWorktree() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("lantern-audit-worktree-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repo = root.appendingPathComponent("repo")
        let worktree = root.appendingPathComponent("worktree")
        try FileManager.default.createDirectory(at: repo, withIntermediateDirectories: true)
        XCTAssertEqual(try git(["init", "--quiet"], at: repo), 0)
        XCTAssertEqual(try git(["commit", "--quiet", "--allow-empty", "-m", "audit fixture"], at: repo), 0)
        XCTAssertEqual(try git(["worktree", "add", "--quiet", "--detach", worktree.path], at: repo), 0)
        ScreenshotStore.excludeFromGit(worktree)
        try assertScreenshotIgnored(in: worktree)
    }

    func testScreenshotFallsBackToPrivateStorageOutsideGit() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("lantern-storage-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let project = root.appendingPathComponent("project")
        let fallback = root.appendingPathComponent("private")
        try FileManager.default.createDirectory(at: project, withIntermediateDirectories: true)
        let path = try XCTUnwrap(ScreenshotStore.save(Data("synthetic image".utf8), cwd: project, fallback: fallback))
        XCTAssertEqual(path.deletingLastPathComponent().path, fallback.path)
        XCTAssertFalse(FileManager.default.fileExists(atPath: project.appendingPathComponent(".lantern").path))
        let attributes = try FileManager.default.attributesOfItem(atPath: path.path)
        XCTAssertEqual((attributes[.posixPermissions] as? NSNumber)?.intValue, 0o600)
    }

    func testScreenshotDoesNotFollowASymlinkOutsideTheProject() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("lantern-symlink-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let project = root.appendingPathComponent("project")
        let outside = root.appendingPathComponent("outside")
        let fallback = root.appendingPathComponent("private")
        try FileManager.default.createDirectory(at: project, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: outside, withIntermediateDirectories: true)
        XCTAssertEqual(try git(["init", "--quiet"], at: project), 0)
        try FileManager.default.createSymbolicLink(at: project.appendingPathComponent(".lantern"), withDestinationURL: outside)
        let path = try XCTUnwrap(ScreenshotStore.save(Data("synthetic image".utf8), cwd: project, fallback: fallback))
        XCTAssertEqual(path.deletingLastPathComponent().path, fallback.path)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: outside.path), [])
    }

    private func assertScreenshotIgnored(in cwd: URL, file: StaticString = #filePath, line: UInt = #line) throws {
        let shot = cwd.appendingPathComponent(".lantern/shots/audit.png")
        try FileManager.default.createDirectory(at: shot.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data("synthetic fixture, not a real screenshot".utf8).write(to: shot)
        XCTAssertEqual(try git(["check-ignore", "--quiet", ".lantern/shots/audit.png"], at: cwd), 0,
                       "A saved screenshot must be ignored by Git", file: file, line: line)
    }

    private func git(_ args: [String], at cwd: URL) throws -> Int32 {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.currentDirectoryURL = cwd
        process.arguments = ["-c", "core.excludesFile=/dev/null", "-c", "core.hooksPath=/dev/null",
                             "-c", "init.templateDir=", "-c", "commit.gpgsign=false",
                             "-c", "user.name=Audit Fixture", "-c", "user.email=audit@example.invalid"] + args
        var env = ProcessInfo.processInfo.environment
        env["GIT_CONFIG_NOSYSTEM"] = "1"
        env["GIT_CONFIG_GLOBAL"] = "/dev/null"
        process.environment = env
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        return process.terminationStatus
    }
}
