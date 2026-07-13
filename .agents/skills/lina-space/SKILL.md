---
name: lina-space-conventions
description: Development conventions and patterns for lina-space. Rust project with conventional commits.
---

# Lina Space Conventions

> Generated from [melgarafael/lina-space](https://github.com/melgarafael/lina-space) on 2026-06-17

## Overview

This skill teaches Claude the development patterns and conventions used in lina-space.

## Tech Stack

- **Primary Language**: Rust
- **Architecture**: hybrid module organization
- **Test Location**: separate

## When to Use This Skill

Activate this skill when:
- Making changes to this repository
- Adding new features following established patterns
- Writing tests that match project conventions
- Creating commits with proper message format

## Commit Conventions

Follow these commit message conventions based on 6 analyzed commits.

### Commit Style: Conventional Commits

### Prefixes Used

- `fix`
- `build`
- `feat`

### Message Guidelines

- Average message length: ~66 characters
- Keep first line concise and descriptive
- Use imperative mood ("Add feature" not "Added feature")


*Commit message example*

```text
build(windows): ambiente de build Windows — SSL, linker MSVC, dev.bat
```

*Commit message example*

```text
fix(windows/app): corrige crashes no boot e stack overflow
```

*Commit message example*

```text
feat(windows): merge doutrina em settings.json existente + overlay de atalhos
```

*Commit message example*

```text
fix(windows/app): Ctrl+V cola do clipboard e persistência em AppData\Roaming
```

*Commit message example*

```text
fix(windows/app): PowerShell como shell padrão no Windows
```

*Commit message example*

```text
fix(windows/app): Ctrl+Shift+C para copiar seleção no Windows
```

## Architecture

### Project Structure: Single Package

This project uses **hybrid** module organization.

### Guidelines

- This project uses a hybrid organization
- Follow existing patterns when adding new code

## Code Style

### Language: Rust

### Naming Conventions

| Element | Convention |
|---------|------------|
| Files | camelCase |
| Functions | camelCase |
| Classes | PascalCase |
| Constants | SCREAMING_SNAKE_CASE |

### Import Style: Relative Imports

### Export Style: Named Exports


*Preferred import style*

```typescript
// Use relative imports
import { Button } from '../components/Button'
import { useAuth } from './hooks/useAuth'
```

*Preferred export style*

```typescript
// Use named exports
export function calculateTotal() { ... }
export const TAX_RATE = 0.1
export interface Order { ... }
```

## Common Workflows

These workflows were detected from analyzing commit patterns.

### Windows Platform Feature Or Bugfix

Implements or fixes Windows-specific features, behaviors, or bugs in the application, often with documentation of platform-specific issues and solutions.

**Frequency**: ~4 times per month

**Steps**:
1. Modify or add Rust source files under app/lina-gpui/src/ to implement or fix Windows-specific logic.
2. Update WINDOWS-CONTRIB.md to document the change, rationale, or workaround.
3. Optionally update docs/CORRECTIONS.md with error histories and solutions.
4. If build or environment changes are needed, update .cargo/config.toml or related build scripts.

**Files typically involved**:
- `app/lina-gpui/src/main.rs`
- `app/lina-gpui/src/bridge.rs`
- `WINDOWS-CONTRIB.md`
- `.cargo/config.toml`
- `app/lina-gpui/.cargo/config.toml`
- `docs/CORRECTIONS.md`
- `dev.bat`

**Example commit sequence**:
```
Modify or add Rust source files under app/lina-gpui/src/ to implement or fix Windows-specific logic.
Update WINDOWS-CONTRIB.md to document the change, rationale, or workaround.
Optionally update docs/CORRECTIONS.md with error histories and solutions.
If build or environment changes are needed, update .cargo/config.toml or related build scripts.
```

### Windows Keybinding Or Shortcut Update

Implements or updates keyboard shortcuts and keybindings specifically for Windows, ensuring platform conventions and user expectations are met.

**Frequency**: ~3 times per month

**Steps**:
1. Modify app/lina-gpui/src/main.rs or bridge.rs to implement or adjust keybinding logic under cfg(windows).
2. Document the new or changed shortcuts in WINDOWS-CONTRIB.md.
3. Test to ensure the shortcut works as expected on Windows.

**Files typically involved**:
- `app/lina-gpui/src/main.rs`
- `app/lina-gpui/src/bridge.rs`
- `WINDOWS-CONTRIB.md`

**Example commit sequence**:
```
Modify app/lina-gpui/src/main.rs or bridge.rs to implement or adjust keybinding logic under cfg(windows).
Document the new or changed shortcuts in WINDOWS-CONTRIB.md.
Test to ensure the shortcut works as expected on Windows.
```


## Best Practices

Based on analysis of the codebase, follow these practices:

### Do

- Use conventional commit format (feat:, fix:, etc.)
- Use camelCase for file names
- Prefer named exports

### Don't

- Don't write vague commit messages
- Don't deviate from established patterns without discussion

---

*This skill was auto-generated by [ECC Tools](https://ecc.tools). Review and customize as needed for your team.*
