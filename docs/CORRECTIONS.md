# CORRECTIONS — Lina Space (Windows)

---

### Correção #1 — SSL CRYPT_E_NO_REVOCATION_CHECK ao baixar crates (2026-06-14)

**Problema:** `cargo build` falhava com `SSL connect error (schannel: next InitializeSecurityContext failed: CRYPT_E_NO_REVOCATION_CHECK 0x80092012)` ao tentar baixar dependências do crates.io.

**Tentativas que falharam:** —

**Solução:** Criar `.cargo/config.toml` na raiz do projeto com:
```toml
[http]
check-revoke = false
```

---

### Correção #2 — link.exe errado (GNU em vez de MSVC) (2026-06-14)

**Problema:** `cargo build` compilava mas falhava no link com `link: extra operand '...'` porque o PATH resolvia `link.exe` do Git for Windows (GNU linker) em vez do MSVC.

**Tentativas que falharam:** —

**Solução:** Instalar Visual Studio Build Tools 2022 com workload C++:
```
winget install Microsoft.VisualStudio.2022.BuildTools --source winget --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```
E rodar builds sempre via Developer Command Prompt ou ativando `vcvars64.bat` antes do `cargo build`.
