---
name: windows-platform-feature-or-bugfix
description: Workflow command scaffold for windows-platform-feature-or-bugfix in lina-space.
allowed_tools: ["Bash", "Read", "Write", "Grep", "Glob"]
---

# /windows-platform-feature-or-bugfix

Use this workflow when working on **windows-platform-feature-or-bugfix** in `lina-space`.

## Goal

Implements or fixes Windows-specific features, behaviors, or bugs in the application, often with documentation of platform-specific issues and solutions.

## Common Files

- `app/lina-gpui/src/main.rs`
- `app/lina-gpui/src/bridge.rs`
- `WINDOWS-CONTRIB.md`
- `.cargo/config.toml`
- `app/lina-gpui/.cargo/config.toml`
- `docs/CORRECTIONS.md`

## Suggested Sequence

1. Understand the current state and failure mode before editing.
2. Make the smallest coherent change that satisfies the workflow goal.
3. Run the most relevant verification for touched files.
4. Summarize what changed and what still needs review.

## Typical Commit Signals

- Modify or add Rust source files under app/lina-gpui/src/ to implement or fix Windows-specific logic.
- Update WINDOWS-CONTRIB.md to document the change, rationale, or workaround.
- Optionally update docs/CORRECTIONS.md with error histories and solutions.
- If build or environment changes are needed, update .cargo/config.toml or related build scripts.

## Notes

- Treat this as a scaffold, not a hard-coded script.
- Update the command if the workflow evolves materially.