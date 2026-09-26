@testable import OctoberLantern
import XCTest

final class ArtworkTests: XCTestCase {
    /// The 2026 connected-state regression sent a ~19-billion-point offset to the renderer.
    /// Exercise the actual StrokeStyle the view uses, including a paused frame's old date.
    func testConnectedStrokeIsBoundedForCurrentAndDistantDates() {
        let dates = [Date(timeIntervalSince1970: 1_800_000_000), Date(), Date.distantPast, Date.distantFuture]
        for date in dates {
            let style = DesktopArtwork.linkStroke(connected: true, at: date)
            XCTAssertEqual(style.dash, [6, 5])
            XCTAssertTrue(style.dashPhase.isFinite)
            XCTAssertLessThan(abs(style.dashPhase), style.dash.reduce(0, +), "Unbounded dash phase at \(date)")
        }
    }

    func testStrokeKeepsTheSameAppearanceAfterOneDashCycle() {
        let date = Date(timeIntervalSince1970: 1_800_000_000)
        let first = DesktopArtwork.linkStroke(connected: true, at: date)
        let next = DesktopArtwork.linkStroke(connected: true, at: date.addingTimeInterval(11.0 / 24.0))
        XCTAssertEqual(first.dashPhase, next.dashPhase, accuracy: 0.0001)
        let moving = DesktopArtwork.linkStroke(connected: true, at: date.addingTimeInterval(0.01))
        XCTAssertNotEqual(first.dashPhase, moving.dashPhase, "The brief connection animation should still move")
    }

    func testDisconnectedStrokeStaysStill() {
        for date in [Date(), Date.distantFuture] {
            let style = DesktopArtwork.linkStroke(connected: false, at: date)
            XCTAssertEqual(style.dash, [3, 5])
            XCTAssertEqual(style.dashPhase, 0)
        }
    }
}
