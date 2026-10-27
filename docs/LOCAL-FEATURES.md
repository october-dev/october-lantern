# Local task features

This describes the source branch, rather than an installer release. Remote devices, SSH, cloud execution, phone expansion, remote routines and task moves are outside this change.

## Cards

October, Claude Code and Codex started by Lantern receive the `lantern` MCP server. They can ask a question, show progress and steps, request a hand-off, suggest a routine, request guidance, and report a result with files. Both native clients render these cards. Cards and launch receipts survive an engine restart and remain after the terminal session ends.

An answer belongs to a particular waiting call and card revision. Creation has an operation ID, so transport retries cannot create a second question. Calls renew a short lease; expired, cancelled and unconfirmed answers are never delivered to a new call. After 25 minutes a question closes without granting permission. Keyboard answers require selecting **Use number keys** on the current question. Typing in a text field, key repeats and stale card revisions do not answer it.

A hand-off is a request to the person. It does not establish a protected sign-in session. Preferences given to an agent are instructions, not enforced restrictions.

## App access controls

Live tasks expose **Pause app access**, **Continue app access**, and **Stop session**. Pause is global to Lantern's driver: it persists a hold, rejects new launches and controlled input, closes waits/guides, revokes the owned driver's sessions, and stops that daemon. Continue starts a fresh driver; it never replays cancelled input or old approvals. Stop also terminates the specific owned tmux/ConPTY session. Driver Off cannot be undone by an earlier start request.

These controls do **not** prove that arbitrary shell children or other same-user automation stopped. They do not establish protected takeover, observation protection, or the stronger “you're in control” guarantee. Those S2 requirements remain open and need an enforced execution boundary. A missing discovery scan is not treated as proof that a session ended.

## Routines

Open **Routines** from Lantern's menu. Create a local schedule with a task, existing project folder, October/Claude Code/Codex, an IANA timezone, a daily/weekdays/weekly time and preferences. Review and confirm it explicitly. An agent's suggestion opens the same form and cannot create a schedule itself. Each run uses that agent's account and may use credits. Routines use the harness's one-shot mode, rather than leaving a terminal waiting after its turn. Claude retains native permission rules; Codex exec uses its workspace-write sandbox.

Lantern and the computer must remain available. An occurrence is recorded before starting an agent. Retries cannot launch it twice; an uncertain admission blocks overlap until reconciled. There are no catch-up bursts after sleep or restart. A repeated DST time runs once at its earlier occurrence; a nonexistent time is skipped. The panel shows next run, its cards/results and the last admission/session state. A session that exits without a result is “closed”; an agent's completion report is labelled as such. Pausing/deleting a schedule leaves an admitted run running.

## Show me

Ask Lantern, Point & Ask and a task's **Show me** hand-off use the same guide. Each look uploads a picture of the whole selected display and a bounded list of visible controls to October's dedicated guidance endpoint. Lantern's own windows are excluded. Control-list field values are omitted; the picture can still contain visible sensitive content. Pictures are held in memory by the guide and discarded on cancellation.

Show me observes through Cua; its allowlist contains no click, type or pointer-movement tool. Mac uses a short, pass-through native ring and step label only for a revalidated semantic control. The renderer independently expires the cue after 1.5 seconds, leaving a marked captured picture. Pixel-only targets and Windows/Linux use a clearly labelled captured picture. Use **Look again** after changing the screen, **Next** after completing a step, or **Stop**. Capture/run/round IDs and cancellation epochs reject late work. Limits are eight steps, sixteen looks, ten minutes total and two minutes idle.

No endpoint/mode fallback changes a guide into an ordinary answer, and no text-only retry silently drops pictures. Native Windows/Linux desktop behavior, the held-out model accuracy evaluation, secure-screen/privacy qualification and protected hand-off remain release gates. Compilation and simulated protocol fixtures do not close them.

## Validation

See [the implementation evidence and remaining gates](LOCAL-VALIDATION.md).

Run `scripts/check.sh` (Mac/Linux), `scripts/check.ps1` (Windows), and `scripts/check-windows-types.sh` for a cross-target type check. The latter does not execute Windows code. `scripts/smoke-local-cards.py` exercises card transport, cancellation, routine editing and Show me cancellation against a private debug profile without starting a paid model. Native UI/model tests must also be recorded separately.
