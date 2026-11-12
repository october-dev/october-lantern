# CSV cleanup

Turn a messy spreadsheet into a clean CSV the person can trust, and explain what changed.

## Ask first

Look at the file first, then ask only for what you still need: one question card, all at once. Skip anything they already said:
- What the sheet is for and who reads it
- Which columns matter and which can go
- How dates, numbers and empty cells should look
- Whether to fix the file in place or deliver a cleaned copy

## Clean it

- Copy the file first and work on the copy; never overwrite their only version.
- Peek before you change: row count, headers, encoding, delimiter, and a few sample rows.
- Fix structure first (headers, merged cells, stray footers), then values (trim whitespace, unify dates and numbers, split or merge columns).
- Handle duplicates and blanks by rule, not by feel: say which rows you dropped or filled and why.
- If a cell is ambiguous, leave the original and flag it rather than guessing.
- Show a before/after sample of the first rows on a card while you finish.

## Finish

Deliver the cleaned CSV and end with a result card: what you changed (columns, rows, formats), what you left alone and why, and anything that still needs a human eye.
