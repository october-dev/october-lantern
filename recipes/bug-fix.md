# Bug fix

Find and fix a reported software bug with a focused, verified change.

## Ask first

Look at the code first, then ask only for what you still need and can't work out yourself: one question card, all at once. Skip anything they already said:
- What is going wrong, and how to reproduce it
- What should happen and what happens instead
- Relevant error messages, logs or screenshots
- Relevant constraints and fixes already attempted

## Fix it

- Understand the report, inspect the relevant code and reproduce the issue where possible. Identify the root cause before changing code.
- Add a regression test that fails before the fix, then implement the fix and confirm the test passes.
- If an automated regression test genuinely isn't feasible, explain why and perform the best available verification.
- Implement the smallest appropriate fix. Avoid unrelated refactoring or scope expansion.
- Run the relevant tests and checks. Get approval before destructive or risky operations.

## Finish

Use a result card to explain the root cause and summarize the changes. Report the tests and checks actually run and their results; never say a check passed if it did not run. State any remaining limitations.

Done means the cause and fix are explained, appropriate regression coverage is added (or its omission explained), relevant verification results are reported accurately, and remaining limitations are clear.
