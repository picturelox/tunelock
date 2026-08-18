# Bug Tracker

Simple, flat bug tracking. One markdown file per bug, numbered sequentially.

## Organization

```
BUGS/
├── README.md        ← this file (index)
├── TEMPLATE.md      ← copy this when filing a new bug
└── BUG-NNN-slug.md  ← individual bug reports
```

## Workflow

1. **File**: Copy `TEMPLATE.md` → `BUG-NNN-short-slug.md`, fill in the top matter.
2. **Work**: Append entries to the **Updates** section as you investigate. Keep newest-first.
3. **Close**: Set `Status: fixed` (or `wontfix` / `duplicate`) and add a final `## Resolution` section.

## Status values

- `open` — reported, not yet triaged
- `in-progress` — actively being worked
- `blocked` — waiting on external input
- `fixed` — resolved and verified
- `wontfix` — acknowledged, will not be addressed
- `duplicate` — see linked bug

## Severity values

- `critical` — app unusable / data loss
- `high` — major feature broken, workaround difficult
- `medium` — feature impaired, workaround available
- `low` — cosmetic / minor annoyance

## Index

| ID | Title | Status | Severity |
|----|-------|--------|----------|
| [BUG-001](./BUG-001-compile-and-runtime-issues.md) | Compile warnings & runtime analysis errors on first launch | fixed | high |
| [BUG-002](./BUG-002-migration-not-idempotent.md) | SQLite migration fails on second launch — tables already exist | fixed | critical |
| [BUG-003](./BUG-003-analysis-not-completing.md) | Analysis starts but never completes / stalls | fixed (pending verify) | high |
