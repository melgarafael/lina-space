---
name: windows-keybinding-or-shortcut-update
description: Workflow command scaffold for windows-keybinding-or-shortcut-update in lina-space.
allowed_tools: ["Bash", "Read", "Write", "Grep", "Glob"]
---

# /windows-keybinding-or-shortcut-update

Use this workflow when working on **windows-keybinding-or-shortcut-update** in `lina-space`.

## Goal

Implements or updates keyboard shortcuts and keybindings specifically for Windows, ensuring platform conventions and user expectations are met.

## Common Files

- `app/lina-gpui/src/main.rs`
- `app/lina-gpui/src/bridge.rs`
- `WINDOWS-CONTRIB.md`

## Suggested Sequence

1. Understand the current state and failure mode before editing.
2. Make the smallest coherent change that satisfies the workflow goal.
3. Run the most relevant verification for touched files.
4. Summarize what changed and what still needs review.

## Typical Commit Signals

- Modify app/lina-gpui/src/main.rs or bridge.rs to implement or adjust keybinding logic under cfg(windows).
- Document the new or changed shortcuts in WINDOWS-CONTRIB.md.
- Test to ensure the shortcut works as expected on Windows.

## Notes

- Treat this as a scaffold, not a hard-coded script.
- Update the command if the workflow evolves materially.