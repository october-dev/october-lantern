# Local task features

This describes the source branch, rather than an installer release. Remote devices, SSH, cloud execution, phone expansion, remote routines and task moves are outside this change.

## Cards

October, Claude Code and Codex started by Lantern receive the `lantern` MCP server. They can ask a question, show progress and steps, request a hand-off, suggest a routine, request guidance, and report a result with files. Both native clients render these cards. Cards and launch receipts survive an engine restart and remain after the terminal session ends.

An answer belongs to a particular waiting call and card revision. Creation has an operation ID, so transport retries cannot create a second question. Calls renew a short lease; expired, cancelled and unconfirmed answers are never delivered to a new call. After 25 minutes a question closes without granting permission. Keyboard answers require selecting **Use number keys** on the current question. Typing in a text field, key repeats and stale card revisions do not answer it.

A hand-off is a request to the person. It does not establish a protected sign-in session. Preferences given to an agent are instructions, not enforced restrictions.

Recent completed card history is bounded; live work and unresolved questions/answer receipts stay visible even beyond those limits.

## App access controls

Live tasks expose **Pause app access**, **Continue app access**, and **Stop session**. Pause is global to Lantern's driver: it persists a hold, rejects new launches and controlled input, closes waits/guides, revokes the owned driver's sessions, and stops that daemon. Continue starts a fresh driver; it never replays cancelled input or old approvals. Stop also terminates the specific owned tmux/ConPTY session. Driver Off cannot be undone by an earlier start request.

These controls do **not** prove that arbitrary shell children or other same-user automation stopped. Protected sign-in and the stronger “you're in control” guarantee are outside this beta's scope, by the owner's 27 October decision. A missing discovery scan is not treated as proof that a session ended.

After an engine crash, a saved hold blocks dispatch before card IPC opens. Lantern cancels old waits and checks the previous private driver under its ownership lock. It confirms app-access pause only after shutdown and endpoint release; a failed or unreachable check remains unconfirmed. A second engine cannot stop a live owner's driver.

## Routines

Open **Routines** from Lantern's menu. Create a local schedule with a task, existing project folder, October/Claude Code/Codex, an optional model, an IANA timezone, a daily/weekdays/weekly time and preferences. Review and confirm it explicitly. An agent's suggestion opens the same form and cannot create a schedule itself. Each run uses that agent's account and may use credits. Routines use the harness's one-shot mode, rather than leaving a terminal waiting after its turn. Claude retains native permission rules; Codex exec uses its workspace-write sandbox.

Lantern and the computer must remain available. An occurrence is recorded before starting an agent. Retries cannot launch it twice; an uncertain admission blocks overlap until reconciled. There are no catch-up bursts after sleep or restart. A repeated DST time runs once at its earlier occurrence; a nonexistent time is skipped. The panel shows next run, its cards/results and the last admission/session state. A session that exits without a result is “closed”; an agent's completion report is labelled as such. Pausing/deleting a schedule leaves an admitted run running.

## Models and account routing

New sessions and routines share a searchable model picker. With an October app sign-in, its list comes from October's live account catalogue and selections explicitly use the October provider, including paid Kimi K2.6. A failed or expired app sign-in shows an error and Retry; it does not substitute direct-provider CLI credentials. Without an app sign-in, October lists its CLI's own configured models. **Default** deliberately follows the agent's saved configuration, which can use a different provider. Explicit selections show their full model id.

Unix launches put their environment and first message into a private single-use file, removed before the agent executes. The terminal/login shell and tmux receive its path, keeping the app access token out of their arguments and tmux's retained start command. This is credential handling, not an execution sandbox.

## Point & Ask

Select an area, then ask about its picture or send it to an agent. On Mac the caption and recipient matching use the selected window, rather than the foreground app. Selections spanning or overlapping windows have no guessed caption. Windows/Linux use the picture without automatic app metadata. Point & Ask does not collect a foreground browser URL, document path or selected text. Reselect keeps the recipient the person chose. Interrupted answers retain their received text with an explicit error; failed turns are excluded from follow-up history.

## Show me

Ask Lantern, Point & Ask and a task's **Show me** hand-off use the same guide. Its button (an amber **Show me on screen** under an answer, **Show me** in Point & Ask) says on hover what it does and that each step sends a picture of the screen to October. Each look uploads a picture of the whole selected display and a bounded list of visible controls to October's dedicated guidance endpoint. Lantern's own windows are excluded. Control-list field values are omitted; the picture can still contain visible sensitive content. Pictures are held in memory by the guide and discarded on cancellation.

Show me never clicks, types or moves the pointer. On Mac, Windows and supported X11 desktops, a transparent, borderless, click-through window draws an orange ring around the target, an arrow from the screen-centre side and **Lantern · Step N**. The ring is at least 44 OS points and is kept inside the selected display, including its pulse. It glows and pulses gently; the OS reduce-motion preference makes it still (an unavailable preference is still too). The ring and picture use the exact captured display's origin, size and scale; negative monitor origins and mixed Windows DPI do not use the pill's scale.

The ring stays until **Next**, **Look again**, **Stop**, a click inside it (with a 16-point tolerance), a screen change or its engine-issued lease expires. A ring click passes through to the real app and hides the ring immediately; after about one second Lantern advances once to the next step. Next, another capture or Stop cancels that delayed action. The Show me card stays open when you return to the real app; losing focus does not stop the guide. If the card covers the target, it moves to the opposite side of its display. A window opening, closing, moving, changing stacking order or coming to the foreground hides the ring and requests a fresh look at the same step. On Windows/X11 the app watches while October is answering too, so a changed screen cancels that answer. Window lists contain IDs and bounds only, stay local, and exclude Lantern's own windows. They do not detect content-only changes inside an otherwise unchanged window; use **Look again** for those.

On Mac the coordinator reads/revalidates controls and watches windows through Cua Driver (its observation allowlist contains no input tools); without the driver it cannot watch window changes. Windows uses native window enumeration and a non-consuming mouse observer. X11 uses EWMH/RandR, a transparency compositor and raw button events, with an empty overlay input region. These desktop clients ask October about the picture without a Cua control list. Linux Wayland, including Lantern running through XWayland, uses the marked-picture fallback (the portal picture may include all displays, which the card explains): XWayland cannot reliably observe native Wayland windows and clicks. Missing compositor/window-manager/input capabilities or a failed native overlay also use that fallback, explained in the card. It never advertises a live ring in that case.

The captured picture remains marked as a backup on every platform. A guide stays on its selected monitor; disconnecting that display fails the look rather than quietly pointing on a different display. Pictures are discarded on cancellation. Run/capture/round IDs and cancellation epochs reject late work. Limits remain eight steps, sixteen looks, ten minutes total and two minutes idle (the ring's outer lease).

No endpoint/mode fallback changes a guide into an ordinary answer, and no text-only retry silently drops pictures. An answer that disconnects before its final receipt shows an error while retaining partial text. Native Windows/Linux desktop behavior, the held-out model accuracy evaluation and screen/privacy qualification remain release gates. Compilation and simulated protocol fixtures do not close them. Sign-in hand-offs are ordinary requests to the person; the beta offers no protected sign-in mode.

## Validation

See [the implementation evidence and remaining gates](LOCAL-VALIDATION.md).

Native overlay regression tests run on Windows CI and a disposable Xvfb/Openbox/compositor desktop on Linux CI, with no account, engine or model. They exercise native placement, click-through, focus preservation, click observation and capture; geometry and coordinator/client tests cover clamping, scaling, screen changes, leases and stale events. They do not qualify every physical monitor/compositor or model accuracy.

Run `scripts/check.sh` (Mac/Linux), `scripts/check.ps1` (Windows), and `scripts/check-windows-types.sh` for a cross-target type check. The latter does not execute Windows code. `scripts/smoke-local-cards.py` exercises card transport, cancellation, routine editing and Show me cancellation against a private debug profile without starting a paid model. Native UI/model tests must also be recorded separately.

On Mac/Linux, `python3 scripts/smoke-driver-recovery.py path/to/debug/lantern-engine path/to/cached/cua-driver` additionally exercises an actual engine crash against the pinned driver already verified by Lantern's installer. It uses no models, screenshots or UI input and does not qualify desktop permissions.
