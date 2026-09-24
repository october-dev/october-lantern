@testable import OctoberLantern
import XCTest

/// Holds a token answer until the test releases it.
@MainActor
private final class Gate {
    var waiting: CheckedContinuation<Void, Never>?
    var arrived: CheckedContinuation<Void, Never>?

    func wait() async {
        await withCheckedContinuation { c in
            waiting = c
            arrived?.resume()
            arrived = nil
        }
    }

    /// Returns once the grant is waiting.
    func untilWaiting() async {
        if waiting != nil { return }
        await withCheckedContinuation { arrived = $0 }
    }

    func release() {
        waiting?.resume()
        waiting = nil
    }
}

@MainActor
final class AccountTests: XCTestCase {
    private func session() -> OctoberAccount.Session {
        .init(accessToken: "a", refreshToken: "r", expiresAt: Date().addingTimeInterval(3600), userId: "u", email: nil)
    }

    func testCancelDuringExchangeStaysSignedOut() async {
        let account = OctoberAccount.detached()
        let gate = Gate()
        let s = session()
        account.tokenGrant = { _, _ in
            await gate.wait()
            return s
        }
        let signIn = Task { await account.signIn(email: "x@example.com", password: "pw") }
        await gate.untilWaiting()
        XCTAssertTrue(account.signingIn)
        account.cancelSignIn()
        gate.release()
        await signIn.value
        XCTAssertFalse(account.signedIn)
        XCTAssertFalse(account.signingIn)
        XCTAssertNil(account.error)
    }

    func testCancelledSignInDoesNotChangeANewerOne() async {
        let account = OctoberAccount.detached()
        let gate = Gate()
        let s = session()
        account.tokenGrant = { _, _ in
            await gate.wait()
            throw OctoberAccount.AuthError.rejected("old attempt failed")
        }
        let first = Task { await account.signIn(email: "x@example.com", password: "pw") }
        await gate.untilWaiting()
        account.cancelSignIn()
        // A second sign-in starts and succeeds while the first is still waiting.
        let old = gate.waiting
        gate.waiting = nil
        account.tokenGrant = { _, _ in s }
        await account.signIn(email: "x@example.com", password: "pw")
        XCTAssertTrue(account.signedIn)
        old?.resume()
        await first.value
        XCTAssertTrue(account.signedIn)
        XCTAssertNil(account.error)
    }

    func testSignInSucceeds() async {
        let account = OctoberAccount.detached()
        let s = session()
        account.tokenGrant = { _, _ in s }
        await account.signIn(email: "x@example.com", password: "pw")
        XCTAssertTrue(account.signedIn)
        XCTAssertFalse(account.signingIn)
    }
}
