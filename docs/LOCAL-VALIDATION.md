# Local implementation and validation — 27 October 2026

Work branch: `local-product`. The integration starts at `68289e5`, with native fixes in `69beab8` and `cc58275`, and routine lifecycle/receipt handling in `b3fc82b`. No main merge, installer publication or release was performed. Devices, SSH, cloud execution, phone expansion, remote routines and task moves were excluded.

## Implemented

- Merge local cards and Show me; both UI clients render cards, guide state and local routines.
- Stable card-creation IDs, fenced replies/leases and deliberate keyboard focus; no late answer is sent as a new instruction.
- Durable app-access hold, fresh-driver Continue, exact owned-session Stop and graceful driver cleanup on app quit. These are scoped controls, not protected takeover.
- Local schedule editing/confirmation, timezone/DST and occurrence admission, missed-run/overlap/restart handling, one-shot harness execution and inline cards/results. Closed sessions and agent-reported outcomes are distinguished.
- Show me capture/run/round cancellation fences, read-only Cua observation, native Mac ring with an independent renderer expiry, captured-picture mode for portable clients.
- Windows Core runtime discovery and IPC peer-user verification; remove obsolete AI gateway/mode/text-only fallbacks.
- Keychain restoration off the main thread, with generation fencing and a regression test. Standalone forms remain open on outside clicks to preserve input.
- Routine drafts use the computer's IANA timezone (an undetectable zone requires a choice). Scheduling covers every saved routine. Recent-history limits cannot hide live launches or unresolved cards/receipts.

## Evidence

- `scripts/check.sh`: engine formatting/lints, **137 passed, 1 ignored**; Mac build and **48 tests passed**; script syntax passed.
- `scripts/check-windows-types.sh`: engine/desktop cross-target types and lints passed. This does not execute a Windows desktop.
- `scripts/smoke-local-cards.py`: real private-profile IPC/stdio, card retry/body/revision fencing, cancellation/late-answer refusal, routine create/pause/delete, guide Stop versus late capture, durable app-access hold. No model or user terminal involved.
- Two actual 25-minute MCP waits: approximately 1500 seconds each, question expired without permission and late approval refused. These were MCP protocol fixtures, not Claude/Codex/October model turns.
- Native Cua: built/signed Mac app launched; actual routine create, agent selection, save/edit and schedule pause observed; clipped form found and fixed; native time picker and run cards rendered. Graceful app quit stopped the exact private driver, verified twice before restarting. A waiting fixture card remained visible after an engine restart.
- Cua later refused input to a floating panel with `element_outside_target_window` / `off_space_or_ax_unresolved`. Those attempts are **not** successful keyboard, answer-delivery, Stop/Continue or completed-result UI evidence. Disposable fixture calls/sessions and schedules were cancelled/removed afterwards.
- A subsequent native macOS accessibility test activated the labelled fixture's option: the MCP caller receipted `delivered`, wrote the selected word to a real file and published its result. Native AX Pause/Continue/Stop then verified the durable hold, owned private-driver availability and exact sleeping-session exit; the global Continue banner still worked after the final session ended. The occurrence became `reported_done`. This was a disposable MCP caller, not an October model or physical keyboard/mouse test. Exact QA bundle hashes and checks are in the private `native-ax-validation.json`; they precede the final projection/timezone changes, which have separate unit/protocol checks.
- A real October launch loaded all nine Lantern tools (`tools=true`). Its existing CLI model was `(openrouter) moonshotai/kimi-k2.6`; that provider rejected its stored key as expired. The harness model selection was left unchanged. The signed test app could not restore its paid October login while macOS waited for the login Keychain password. No real October answer/result round trip is claimed.
- All three native CI jobs passed for `bbddeb8`: [run 40476472341](https://github.com/october-dev/october-lantern/actions/runs/40476472341). This proves that revision only. Later commits require their own green run; consult branch CI rather than carrying this result forward.

QA logs, screenshots and fixture data are private under `/private/tmp/lantern-qa` and `/private/tmp/lantern-*.log`. They are not committed. The original `cards` checkout and existing `show-me` worktree were preserved.

## Open execution/release gates

1. **S2 protected takeover and protected sign-in:** unrestricted same-user CLI children can continue outside Cua. The implemented app-access controls do not close this requirement. An enforced execution/observation boundary and route qualification are still needed. Do not display “you're in control” on these routes.
2. **Signed-in default October and packaged journeys:** complete the system Keychain authorization, verify paid-account/model routing, then exercise real question → UI answer → receipt → result, routine occurrence, multi-image/history, Point & Ask and dictation. Keep the existing model; do not silently switch providers/models or use an expired external key.
3. **Complete Show me qualification:** real model/UI runs, held-out accuracy by plan/model, secure-screen privacy, multi-display/scale/full-screen/cancellation matrix, task-bound guidance and A1–A10 closure. Portable source compiles; physical Windows/Linux behavior remains unqualified.
4. **Native input stability:** qualify physical keyboard/focus/shortcut behavior and real-model card/control/result journeys on a stable accessible app window. The native AX fixture passes above do not qualify physical input or model behavior; rejected Cua commands remain failed attempts.
5. **Backend task contract:** isolated Saturday commit `8bac3e71`, branch `lantern-local-contract`, accepts explicit absolute project folders and emits `taskError` when model task details are invalid. Its 93 focused tests passed. Automatic approval review rejected the separate repository push without explicit user authorization. It is committed locally, not pushed, merged or deployed. The deployed absolute-path bug remains until that change is shipped.
6. Complete final revision CI and packaged qualification before merging/releasing. Consumer claims and plan checkboxes must reflect the qualified scope.
