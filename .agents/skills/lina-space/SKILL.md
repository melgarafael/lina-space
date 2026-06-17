```markdown
# lina-space Development Patterns

> Auto-generated skill from repository analysis

## Overview

This skill provides guidance for contributing to the `lina-space` Rust codebase. It covers coding conventions, commit patterns, and platform-specific workflows, with a focus on Windows feature and keybinding development. The repository does not use a major framework, and emphasizes clear documentation and maintainable code practices.

## Coding Conventions

- **File Naming:**  
  Use camelCase for file names.  
  _Example:_  
  ```
  myModule.rs
  windowHandler.rs
  ```

- **Import Style:**  
  Use relative imports within modules.  
  _Example:_  
  ```rust
  mod utils;
  use crate::utils::helperFunction;
  ```

- **Export Style:**  
  Use named exports for functions, structs, and modules.  
  _Example:_  
  ```rust
  pub fn performAction() { ... }
  pub struct WindowState { ... }
  ```

- **Commit Messages:**  
  Follow [Conventional Commits](https://www.conventionalcommits.org/) with prefixes like `fix`, `build`, `feat`.  
  _Example:_  
  ```
  feat: add Windows-specific keybinding for fullscreen toggle
  fix: resolve panic on window resize in Windows
  ```

## Workflows

### windows-platform-feature-or-bugfix
**Trigger:** When adding, fixing, or improving Windows-specific functionality or resolving platform-specific bugs.  
**Command:** `/windows-feature-fix`

1. Modify or add Rust source files under `app/lina-gpui/src/` to implement or fix Windows-specific logic.
   ```rust
   #[cfg(windows)]
   fn windows_specific_feature() {
       // Windows-only implementation
   }
   ```
2. Update `WINDOWS-CONTRIB.md` to document the change, rationale, or workaround.
3. Optionally update `docs/CORRECTIONS.md` with error histories and solutions.
4. If build or environment changes are needed, update `.cargo/config.toml` or related build scripts.
5. Commit with a descriptive message:
   ```
   fix: handle Windows clipboard bug in bridge.rs
   ```

**Files Involved:**
- `app/lina-gpui/src/main.rs`
- `app/lina-gpui/src/bridge.rs`
- `WINDOWS-CONTRIB.md`
- `.cargo/config.toml`
- `app/lina-gpui/.cargo/config.toml`
- `docs/CORRECTIONS.md`
- `dev.bat`

---

### windows-keybinding-or-shortcut-update
**Trigger:** When adding or changing keyboard shortcuts or keybindings for Windows users.  
**Command:** `/windows-keybinding`

1. Modify `app/lina-gpui/src/main.rs` or `bridge.rs` to implement or adjust keybinding logic under `cfg(windows)`.
   ```rust
   #[cfg(windows)]
   fn setup_shortcuts() {
       // Example: Ctrl+Q to quit
       register_shortcut("Ctrl+Q", || {
           app.quit();
       });
   }
   ```
2. Document the new or changed shortcuts in `WINDOWS-CONTRIB.md`.
3. Test to ensure the shortcut works as expected on Windows.
4. Commit with a descriptive message:
   ```
   feat: add Ctrl+Shift+N shortcut for new window on Windows
   ```

**Files Involved:**
- `app/lina-gpui/src/main.rs`
- `app/lina-gpui/src/bridge.rs`
- `WINDOWS-CONTRIB.md`

---

## Testing Patterns

- **Test File Naming:**  
  Test files follow the pattern `*.test.*` (e.g., `windowHandler.test.rs`).
- **Framework:**  
  The specific testing framework is unknown, but standard Rust test modules are likely used.
- **Example:**  
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn test_feature_behavior() {
          assert_eq!(some_function(), expected_value);
      }
  }
  ```

## Commands

| Command                 | Purpose                                                      |
|-------------------------|--------------------------------------------------------------|
| /windows-feature-fix    | Start a Windows-specific feature or bugfix workflow          |
| /windows-keybinding     | Begin a Windows keybinding or shortcut update workflow       |
```