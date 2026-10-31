# Backend agent prompt: Lantern account pages

Implement the website side of Lantern's consumer account flow. Work on an isolated worktree, inspect the actual frontend, auth implementation and deployed routing first, and keep the existing October Supabase identity. Do not deploy to production or change production auth settings, email templates or database configuration without Harsh's explicit instruction. Report any required settings changes separately.

## Product outcome

A new Lantern user clicks **Create account** in the desktop app, lands on a Lantern-branded page, creates and verifies an account, and returns to the waiting Lantern app signed in. They should not need to understand October Desktop, coding agents, teams or a canvas. Existing October accounts work without creating another account.

Build these routes on `https://lantern.october.dev`:

- `/signup`: Lantern logo, “Create your Lantern account”, email/password signup, verification/resend and clear existing-account guidance. Match the current Lantern site's visual style. Keep any offered Google, GitHub and Apple options connected to the same native return flow when opened from Lantern.
- `/forgot-password`: request a reset email, complete the reset in a Lantern-branded recovery view, then tell the user to return to Lantern and sign in with the new password. Include working sign-in/signup navigation and recovery errors.
- Any callback or completion routes these flows actually require. Reuse the existing auth implementation where it fits; do not add a second account database or a new token-relay service.

Use one short explanation: “Lantern is made by October. Already have an October account? Use it here.” The phone is still called the **October app**; explain that the user signs in with the same account. Keep October Desktop promotion out of this first-run journey. Do not invent plan entitlements or promise phone access for every account; use the existing authoritative plan checks.

As checked on 30 October 2026, `https://lantern.october.dev/signup` returned 404. `https://www.october.dev/signup` exists, but its copy and default `/download` destination describe October's coding workspace. Locate the actual Lantern website project and Vercel routing; do not assume the Saturday frontend serves both hosts.

## Native signup contract already implemented

The desktop app owns its existing Supabase PKCE verifier. It starts its existing listener on `127.0.0.1:54547` **before** opening this URL:

```text
https://lantern.october.dev/signup?redirect_to=http%3A%2F%2Flocalhost%3A54547%2Fcallback&code_challenge=<base64url SHA-256 of native verifier>&code_challenge_method=s256
```

Parameters:

| Name | Required value in the app-originated flow |
| --- | --- |
| `redirect_to` | Exactly `http://localhost:54547/callback` |
| `code_challenge` | 43-character unpadded base64url SHA-256 challenge |
| `code_challenge_method` | Exactly `s256` |

Support this contract rather than replacing it with browser-owned PKCE. The native verifier never leaves the initiating app's memory. The website must forward the supplied challenge/method into Supabase's existing signup or provider authorization request, and set the fixed email/authorization return URL above. Supabase returns a one-time `code` to the loopback callback after verification/authorization. The app exchanges that code with `grant_type=pkce`, `auth_code` and its verifier, stores the session in its existing OS credential store, and refreshes the plan.

The generic Supabase SDK signup implementation generates its own verifier when configured for PKCE. Calling that unchanged would break the native exchange. Inspect the installed SDK types/source and implement the smallest scoped adapter for forwarding the native challenge through the established Supabase auth fields. Verify the deployed auth server supports this before declaring the flow complete. Do not ask the website to exchange the app's code, return access/refresh tokens to localhost, or write an app verifier into browser storage.

For reference, the installed Saturday SDK's `node_modules/@supabase/auth-js/src/GoTrueClient.ts` `signUp()` sends email/password with `code_challenge` and `code_challenge_method` to `/signup`, using `options.emailRedirectTo` as the redirect. Provider authorization uses the existing `/auth/v1/authorize` endpoint. Confirm exact behavior against the current installed version and [Supabase's PKCE documentation](https://supabase.com/docs/guides/auth/sessions/pkce-flow) and [signup documentation](https://supabase.com/docs/reference/javascript/auth-signup).

The app waits up to five minutes for signup. Cancellation, a new attempt or timeout invalidates the local wait. The callback's browser page says “Finishing sign-in in Lantern”; only successful exchange in the app confirms sign-in. A cancelled/timed-out website signup may still have created the account. Give that user a clear route to sign in again in Lantern. Do not call a website-only success “Lantern is signed in.”

## Validation and recovery

- Validate the native signup context separately from ordinary site navigation. Require all three parameters exactly once; reject malformed, duplicate, partial or unsupported values before starting auth. Do not accept arbitrary redirect hosts/ports/paths, credentials, fragments or extra parameters inside the return URL.
- Preserve the existing strict same-origin `safeReturnPath` behavior for normal web navigation. Do not widen it to accept arbitrary absolute URLs to make localhost work.
- A visitor who opens `/signup` without a native context still gets a normal Lantern signup journey. Its completion should offer Download/Open Lantern and explain the remaining sign-in step. If you cannot resume a native attempt, say so; do not fabricate a handoff.
- Never put passwords, verifiers, access tokens or refresh tokens into URLs, logs, analytics or client error reports. Do not log auth codes or full callback URLs. Apply the site's existing credential-safe error handling. Use public Supabase auth credentials in the browser, never a service-role/admin key.
- Inspect email confirmation/autoconfirm, permitted redirect URLs and email templates. Supabase PKCE signup depends on these settings. List the exact changes needed for Harsh; do not silently change them or turn off verification. Prefer no database migration.
- Keep email verification and resend working with the original native challenge and fixed return destination. Explain that verification should be opened on the same computer with Lantern still waiting. Handle rate limits, delayed mail, expired/reused links, already-existing accounts and opening email on a phone. Do not promise an expired native attempt can resume.
- An existing browser session must not silently complete signup as a different account. Show which account is being used when relevant and provide a clear switch/sign-in path. Do not sign the user out of unrelated October sessions automatically.
- Password recovery is a separate, properly validated recovery session. Complete it on the website and direct the user to native email/password sign-in. Do not feed recovery credentials through the normal native signup callback.
- Use generic recovery-email acknowledgement so the UI does not expose whether an email is registered. Use the actual Supabase password requirements and errors rather than inventing a second password policy.

## Grounded places to inspect

In `harshsaver/saturday`, inspect at least:

- `src/components/SignUpPage.tsx`, `SignInPage.tsx`, `AuthCallback.tsx` and `ResetPasswordPage.tsx`.
- `src/lib/supabase.ts`, `src/lib/returnPath.ts` and `src/lib/authCallback.ts`.
- The existing signup resend behavior, email callback routing and recovery handling; verify they work rather than only restyling their buttons.
- The actual Lantern host's project, routes, design assets and deploy configuration.

The native client implementation is in `october-dev/october-lantern`, branch `feat/connections-redesign`:

- `macos/Sources/OctoberLantern/OctoberAccount.swift`: `browserURL`, `createAccount`, listener and existing PKCE exchange.
- `desktop/src/account.rs`: `BrowserFlow::SignUp`, `browser_url` and `browser_sign_in` for Windows/Linux.
- `protocol/README.md`: “Lantern browser signup”.

The app's permanent page/menu is now **Connections**. Phone instructions must say **Connections → Phone → Pair**, replacing “October panel → Phone → Pair”. If the phone app's instruction copy is owned elsewhere, report that exact text change for its agent. Do not change the phone pairing URL, relay protocol, account binding or `lantern-` core version prefix.

## Acceptance and handoff

Test the real native/browser boundary, not only component renders:

1. Fresh email account: Create account in Lantern → website → verification on this computer → app shows the correct account and real plan. Confirm later email/password sign-in and same-account phone sign-in.
2. Any OAuth buttons offered in the app-originated signup flow return to the waiting app using its challenge. Existing provider login directly from Lantern still works.
3. Ordinary website signup without native parameters, an already-registered email, and a browser already signed in to a different account have clear, truthful outcomes.
4. Forgot password, resend, expired/replayed verification and recovery links, network/plan errors, and email opened on another device have one clear recovery action each.
5. App cancellation, timeout, app closed, listener port unavailable and a newer native attempt do not produce a false sign-in success or attach a code to a different attempt.
6. Malformed/duplicate challenge parameters and arbitrary callback URLs are rejected. No change weakens the general site's return-path validation or leaks credentials through logs/analytics.

Run the repository's appropriate tests and add focused regression tests for native-context validation, challenge forwarding, email verification/resend and recovery separation. Report commits, test evidence, actual routes, exact Supabase/Vercel/email prerequisites and any remaining device acceptance. Do not claim the desktop flow is ready to ship until the required website routes are deployed and one real email verification round trip succeeds.
