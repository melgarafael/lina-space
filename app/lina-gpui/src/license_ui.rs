//! `license_ui` — F1-4-6: estado headless e copy CONGELADA da ativação do Lina PRO.
//!
//! Camada gpui-free compartilhada pelas DUAS superfícies da story (vitrine 1b no M9 +
//! Ajustes › Plano T7§P). A autoridade da licença é o crate `lina-license` (F1-4-5:
//! ed25519 100% local, zero rede); aqui vive só a TRADUÇÃO para o leigo — strings da
//! fonte única `tasks/epico-f1/copy-f1-4.md` §§1b/2/3 (congeladas; mudar lá → mudar aqui)
//! e o mapeamento 1:1 `ActivationError` → mensagem acionável (critério 2 da story).
//!
//! Honestidade offline (regra do copy-doc): nenhuma string implica servidor — o app
//! **substitui um arquivo local e destrava quando uma chave válida é colada**.

use lina_license::{ActivationError, BlockReason, EffectiveLicense, LicenseStatus, Verifier};
use std::path::Path;

// ═══════════════════════ copy congelada (copy-f1-4.md §1b — tela do bloqueio) ═══════════════════════

pub const COPY_BLOCK_TITLE: &str = "Seu segundo Espaço vem com o Lina PRO";
pub const COPY_BLOCK_BODY: &str = "O plano Free inclui 1 Espaço — o seu, que já está em uso. \
Para ter um Espaço para cada projeto, cada um com seu próprio time, é só ativar o Lina PRO.";
pub const COPY_BLOCK_HAVE_KEY: &str = "Já tenho uma chave";
pub const COPY_BLOCK_BUY: &str = "Quero o Lina PRO ↗";
pub const COPY_BLOCK_NOT_NOW: &str = "Agora não";
pub const COPY_BLOCK_NOTE: &str =
    "A compra acontece no site. Aqui você só cola a chave que chega por e-mail — sem cadastro.";

// ═══════════════════════ copy congelada (§2 — campo de colar + erros + sucesso) ═══════════════════════

/// Rótulo do campo — também é o que o leitor de tela lê ao focar (a11y, critério 5).
pub const COPY_KEY_LABEL: &str = "Sua Chave do Lina PRO";
pub const COPY_KEY_PLACEHOLDER: &str = "cole aqui a chave do seu e-mail de compra";
pub const COPY_KEY_ACTIVATE: &str = "Ativar";
/// E1 — `Ativar` com o campo vazio (foco volta ao campo).
pub const COPY_ERR_EMPTY: &str = "Falta colar a chave — ela chegou no seu e-mail de compra.";
/// E2 — chave incompleta/malformada (campo mantém o texto para corrigir).
pub const COPY_ERR_MALFORMED: &str =
    "Essa chave está incompleta — confira no e-mail de compra se copiou tudo, do começo ao fim.";
/// E3 — assinatura não confere (campo mantém o texto).
pub const COPY_ERR_SIGNATURE: &str =
    "Essa chave não confere — copie de novo do e-mail de compra, sem mudar nada no texto.";
/// Microcopy fixa sob o E4 (fecha o loop offline da renovação).
pub const COPY_RENEW_NOTE: &str =
    "A renovação chega como uma chave nova no seu e-mail — é só colar aqui de novo.";
/// Falha de disco ao gravar (`ActivationError::Io`) — DERIVADA (não há string congelada
/// para E/S no copy-doc; auditoria de copy: nomeia a causa real + saída, sem jargão).
pub const COPY_ERR_IO: &str =
    "A chave confere, mas não consegui guardá-la neste computador — veja se há espaço em \
disco e tente de novo.";
pub const COPY_SUCCESS_TITLE: &str = "✓ Lina PRO ativo — obrigado!";
pub const COPY_SUCCESS_CHANGED: &str = "O que mudou agora:";
/// Único item garantido pelo MECANISMO (`workspace_limit`); o resto vem dos entitlements
/// assinados na própria chave (gating data-driven de F1-4-5).
pub const COPY_SUCCESS_SPACES: &str = "★ Espaços: crie quantos precisar";

/// E4 — chave com validade vencida (o `expiry` vive assinado dentro da chave).
#[must_use]
pub fn copy_err_expired(expiry_epoch_s: u64) -> String {
    format!("Essa chave venceu em {}.", month_year(expiry_epoch_s))
}

/// «Válida até {mês/ano}» — omitida quando a chave não tem validade (nunca "vitalícia";
/// pricing é ADR 0011).
#[must_use]
pub fn copy_valid_until(expiry_epoch_s: u64) -> String {
    format!("Válida até {}", month_year(expiry_epoch_s))
}

// ═══════════════════════ copy congelada (§3 — painel Ajustes › Plano) ═══════════════════════

pub const COPY_PLAN_FREE_TITLE: &str = "Seu plano: Lina Free";
pub const COPY_PLAN_HAVE_COL: &str = "O que você tem hoje:";
pub const COPY_PLAN_HAVE_ITEMS: &[&str] = &[
    "✓ 1 Espaço com seu time completo",
    "✓ estado e atividade dos Agentes",
];
pub const COPY_PLAN_UNLOCK_COL: &str = "Com o Lina PRO você destrava:";
pub const COPY_PLAN_ASK_KEY: &str = "Já tem uma chave?";
pub const COPY_PLAN_BUY_PREFIX: &str = "Ainda não tem?";
pub const COPY_PLAN_BUY_LINK: &str = "Conhecer o Lina PRO ↗";
pub const COPY_PLAN_PRO_TITLE: &str = "Seu plano: Lina PRO ✓";
pub const COPY_PLAN_SWAP: &str = "Trocar chave…";
pub const COPY_PLAN_SWAP_NOTE: &str = "Cole a nova chave — ela entra no lugar da antiga.";
pub const COPY_PLAN_REMOVE: &str = "Remover chave";
pub const COPY_PLAN_REMOVE_CONFIRM: &str = "Remover a chave volta o app para o plano Free. \
Seus Espaços continuam abrindo. Criar novos fica pausado até colar uma chave de novo.";
pub const COPY_PLAN_REMOVE_CANCEL: &str = "Cancelar";
pub const COPY_PLAN_REMOVE_DO: &str = "Remover";
/// Degradação graciosa (arquivo corrompido/adulterado depois de ativo) — sem alarme.
pub const COPY_PLAN_REPASTE: &str = "Sua chave precisa ser colada de novo. Cole do e-mail \
de compra para reativar o seu plano — nada do seu trabalho se perdeu enquanto isso.";

/// «Espaços: {usados} de {limite} em uso» / variante sem teto (§3, congelada).
#[must_use]
pub fn copy_plan_usage(used: u32, limit: Option<u32>) -> String {
    match limit {
        Some(l) => format!("Espaços: {used} de {l} em uso"),
        None => format!("Espaços: {used} em uso — sem limite"),
    }
}

/// Página de compra — aberta SÓ no navegador padrão por clique explícito (inv#2: o app
/// nunca toca a rede). ⚠ URL final pendente do pricing (ADR 0011) — trocar AQUI quando
/// o fundador decidir o domínio/checkout.
pub const STORE_URL: &str = "https://linaspace.app/pro";

// ═══════════════════════ gate de criação → copy da vitrine (por BlockReason) ═══════════════════════

/// Linha-fato do bloqueio por motivo (vai no `CreateSpaceModal.blocked` e na live-region).
/// `FreeTier` usa a string congelada §1a (a MESMA que o card do M9 já usava); as outras
/// duas são DERIVADAS do padrão (§1a/§4) — registradas para auditoria de copy.
#[must_use]
pub fn blocked_copy(reason: BlockReason, limit: u32, expiry: Option<u64>) -> String {
    match reason {
        BlockReason::FreeTier => "Você já usa o Espaço do plano Free (1 de 1)".to_string(),
        BlockReason::LimitReached => {
            format!("Você já usa todos os Espaços do seu plano ({limit} de {limit})")
        }
        BlockReason::Expired => {
            let when = expiry.map(month_year).unwrap_or_default();
            format!(
                "Sua Chave do Lina PRO venceu em {when} — criar novos Espaços fica pausado \
até você colar a chave nova."
            )
        }
    }
}

/// Relógio dos pontos de gating (epoch s) — o `lina-license` exige o instante INJETADO
/// (critério 7 de F1-4-5: expiry só é avaliado em ponto de gating, nunca num timer).
#[must_use]
pub fn now_epoch_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ═══════════════════════ campo de colar (estado compartilhado M9 + T7) ═══════════════════════

/// Resultado visível da última tentativa de ativação.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyFeedback {
    /// Erro acionável (E1–E4 + E/S). `renewable` ⇒ mostrar `[ Renovar no site ↗ ]` +
    /// a microcopy da renovação (E4).
    Error { message: String, renewable: bool },
    /// Chave instalada: lista do que mudou (mecanismo + entitlements assinados) e a
    /// validade quando existe.
    Success {
        changed: Vec<String>,
        valid_until: Option<String>,
    },
}

/// O campo «cole aqui a chave…» como estado puro: normalização silenciosa no input
/// (colar de e-mail com espaços/quebras NUNCA falha por formatação — §2) + ativação.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KeyEntry {
    pub input: String,
    pub feedback: Option<KeyFeedback>,
}

impl KeyEntry {
    /// Entrada de texto (digitada ou colada): só os caracteres não-brancos entram —
    /// é a normalização silenciosa do §2 (trim/quebras de linha removidos no ato).
    pub fn type_str(&mut self, s: &str) {
        self.input.extend(s.chars().filter(|c| !c.is_whitespace()));
    }

    pub fn backspace(&mut self) {
        self.input.pop();
    }

    /// `[[ Ativar ]]`: valida e instala via `lina_license::activate`. Devolve `true`
    /// quando ativou (o chamador re-roda o gate e destrava SEM restart). Em erro, o
    /// campo MANTÉM o texto para corrigir (saída E2/E3); vazio é E1.
    pub fn activate(&mut self, path: &Path, verifier: &Verifier, now_epoch_s: u64) -> bool {
        if self.input.is_empty() {
            self.feedback = Some(KeyFeedback::Error {
                message: COPY_ERR_EMPTY.to_string(),
                renewable: false,
            });
            return false;
        }
        match lina_license::activate(&self.input, path, verifier, now_epoch_s) {
            Ok(claims) => {
                let mut changed = vec![COPY_SUCCESS_SPACES.to_string()];
                changed.extend(claims.entitlements.iter().map(|e| format!("★ {e}")));
                self.feedback = Some(KeyFeedback::Success {
                    changed,
                    valid_until: claims.expiry.map(copy_valid_until),
                });
                self.input.clear();
                true
            }
            Err(e) => {
                self.feedback = Some(KeyFeedback::Error {
                    renewable: matches!(e, ActivationError::Expired { .. }),
                    message: error_copy(&e),
                });
                false
            }
        }
    }

    /// O que a live-region anuncia após a tentativa (erro OU confirmação — critério 5).
    #[must_use]
    pub fn announcement(&self) -> Option<String> {
        match &self.feedback {
            Some(KeyFeedback::Error { message, .. }) => Some(message.clone()),
            Some(KeyFeedback::Success { valid_until, .. }) => Some(match valid_until {
                Some(v) => format!("{COPY_SUCCESS_TITLE} {v}."),
                None => COPY_SUCCESS_TITLE.to_string(),
            }),
            None => None,
        }
    }
}

/// `ActivationError` → mensagem congelada (E2/E3/E4; E/S derivada) — 1 variante = 1 copy.
#[must_use]
pub fn error_copy(e: &ActivationError) -> String {
    match e {
        ActivationError::MalformedKey(_) => COPY_ERR_MALFORMED.to_string(),
        ActivationError::InvalidSignature => COPY_ERR_SIGNATURE.to_string(),
        ActivationError::Expired { expiry } => copy_err_expired(*expiry),
        ActivationError::Io(_) => COPY_ERR_IO.to_string(),
    }
}

// ═══════════════════════ painel Ajustes › Plano (projeção do estado) ═══════════════════════

/// O que o painel T7§P mostra — projeção PURA de `EffectiveLicense` (critério 3: o
/// painel afirma só o que o `lina-license` garante).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanPanel {
    /// Free (sem licença) — com a linha de vencimento quando uma chave autêntica venceu
    /// (`effective` rebaixa para free; a linha explica o porquê sem alarme).
    Free { expired_note: Option<String> },
    /// Licença válida e vigente.
    Pro {
        valid_until: Option<String>,
        usage: String,
    },
    /// Arquivo presente mas ilegível/adulterado — degradação graciosa: re-colar resolve.
    Repaste,
}

#[must_use]
pub fn plan_panel(eff: &EffectiveLicense, used: u32) -> PlanPanel {
    match eff.status {
        LicenseStatus::DegradedUnreadable | LicenseStatus::DegradedInvalidSignature => {
            PlanPanel::Repaste
        }
        LicenseStatus::NoLicense => PlanPanel::Free { expired_note: None },
        LicenseStatus::Licensed => {
            if eff.expired {
                PlanPanel::Free {
                    expired_note: eff
                        .expiry
                        .map(|e| format!("Sua Chave do Lina PRO venceu em {}.", month_year(e))),
                }
            } else {
                PlanPanel::Pro {
                    valid_until: eff.expiry.map(copy_valid_until),
                    usage: copy_plan_usage(used, eff.workspace_limit),
                }
            }
        }
    }
}

// ═══════════════════════ {mês/ano} de um epoch (UTC, sem dependência nova) ═══════════════════════

/// `epoch s` → «MM/AAAA» (UTC). Algoritmo civil-from-days (Howard Hinnant) — exato para
/// todo o range de interesse; evita puxar `chrono`/`time` só para um formato.
#[must_use]
pub fn month_year(epoch_s: u64) -> String {
    let days = (epoch_s / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{m:02}/{y}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use lina_license::{encode_token, LicenseClaims, LicenseState, WorkspaceGate};
    use rand_core::OsRng;

    const NOW: u64 = 1_750_000_000; // 2025-06-15 UTC

    fn pro_claims(limit: Option<u32>, expiry: Option<u64>) -> LicenseClaims {
        LicenseClaims {
            license_id: "lic-ui-teste".into(),
            tier: "pro".into(),
            workspace_limit: limit,
            entitlements: vec!["Medidor completo ativado".into()],
            expiry,
            machine_id: None,
        }
    }

    fn signed_token(key: &SigningKey, claims: &LicenseClaims) -> String {
        let sig = key.sign(&claims.canonical_bytes());
        encode_token(claims, &sig.to_bytes()).expect("token")
    }

    /// {mês/ano} correto (E4/«Válida até» dependem disto): epochs conhecidos.
    #[test]
    fn month_year_formats_utc() {
        assert_eq!(month_year(1_750_000_000), "06/2025");
        assert_eq!(month_year(0), "01/1970");
        // 2026-06-18 (EoL do Gemini — data dura conhecida) = 1781740800.
        assert_eq!(month_year(1_781_740_800), "06/2026");
    }

    /// §2: colar com espaços/quebras NUNCA falha por formatação — normalização no input.
    #[test]
    fn pasted_key_is_normalized_silently() {
        let mut k = KeyEntry::default();
        k.type_str("  LINA1.\nabc ");
        k.type_str("\tdef\r\n");
        assert_eq!(k.input, "LINA1.abcdef");
        k.backspace();
        assert_eq!(k.input, "LINA1.abcde");
    }

    /// Critério "erro leigo por variante": E1 vazio · E2 malformada · E3 assinatura ·
    /// E4 vencida (com {mês/ano} e saída de renovação). 1 variante = 1 copy, sem jargão.
    #[test]
    fn each_error_variant_has_its_own_lay_copy() {
        let dir = std::env::temp_dir().join(format!("lina-licui-err-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tempdir");
        let path = dir.join("license.json");
        let key = SigningKey::generate(&mut OsRng);
        let verifier = Verifier::with_key(key.verifying_key());

        // E1 — vazio.
        let mut k = KeyEntry::default();
        assert!(!k.activate(&path, &verifier, NOW));
        assert_eq!(
            k.feedback,
            Some(KeyFeedback::Error {
                message: COPY_ERR_EMPTY.into(),
                renewable: false
            })
        );

        // E2 — malformada (campo MANTÉM o texto para corrigir).
        let mut k = KeyEntry::default();
        k.type_str("LINA1.incompleta");
        assert!(!k.activate(&path, &verifier, NOW));
        assert_eq!(k.input, "LINA1.incompleta", "texto preservado p/ corrigir");
        match k.feedback {
            Some(KeyFeedback::Error {
                ref message,
                renewable: false,
            }) => {
                assert_eq!(message, COPY_ERR_MALFORMED);
            }
            ref other => panic!("E2 esperado, veio {other:?}"),
        }

        // E3 — assinatura de terceiro não confere.
        let intruder = SigningKey::generate(&mut OsRng);
        let mut k = KeyEntry::default();
        k.type_str(&signed_token(&intruder, &pro_claims(None, None)));
        assert!(!k.activate(&path, &verifier, NOW));
        match k.feedback {
            Some(KeyFeedback::Error {
                ref message,
                renewable: false,
            }) => {
                assert_eq!(message, COPY_ERR_SIGNATURE);
            }
            ref other => panic!("E3 esperado, veio {other:?}"),
        }

        // E4 — autêntica porém vencida: verbo «venceu» + {mês/ano} + saída de renovação.
        let mut k = KeyEntry::default();
        k.type_str(&signed_token(&key, &pro_claims(None, Some(NOW - 1))));
        assert!(!k.activate(&path, &verifier, NOW));
        match k.feedback {
            Some(KeyFeedback::Error {
                ref message,
                renewable: true,
            }) => {
                assert_eq!(message, &copy_err_expired(NOW - 1));
                assert!(message.contains("venceu em 06/2025"), "{message}");
            }
            ref other => panic!("E4 esperado, veio {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Critérios "gate bloqueia sem chave" e "desbloqueia com chave válida de teste":
    /// sem license.json → 2º Espaço Blocked(FreeTier); colar chave válida → Allowed —
    /// tudo pelo MESMO caminho que o app usa (KeyEntry::activate → LicenseState::load).
    #[test]
    fn gate_blocks_without_key_and_unblocks_after_valid_paste() {
        let dir = std::env::temp_dir().join(format!("lina-licui-gate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tempdir");
        let path = dir.join("license.json");
        let key = SigningKey::generate(&mut OsRng);
        let verifier = Verifier::with_key(key.verifying_key());

        // SEM chave: 1º Espaço passa, 2º bloqueia como FreeTier (a vitrine 1b).
        let state = LicenseState::load(&path, &verifier);
        assert_eq!(state.can_create_workspace(0, NOW), WorkspaceGate::Allowed);
        let gate = state.can_create_workspace(1, NOW);
        let WorkspaceGate::Blocked { limit, reason, .. } = gate else {
            panic!("2º Espaço sem chave deveria bloquear");
        };
        assert_eq!(
            blocked_copy(reason, limit, None),
            "Você já usa o Espaço do plano Free (1 de 1)",
            "copy §1a congelada para o caso Free"
        );

        // Colar a chave válida → feedback de sucesso (com o que mudou) → gate destrava.
        let mut k = KeyEntry::default();
        k.type_str(&signed_token(&key, &pro_claims(None, Some(NOW + 86_400))));
        assert!(k.activate(&path, &verifier, NOW), "chave válida ativa");
        match k.feedback {
            Some(KeyFeedback::Success {
                ref changed,
                ref valid_until,
            }) => {
                assert_eq!(changed[0], COPY_SUCCESS_SPACES);
                assert!(
                    changed[1].contains("Medidor completo"),
                    "entitlement assinado vira item"
                );
                assert_eq!(valid_until.as_deref(), Some("Válida até 06/2025"));
            }
            ref other => panic!("sucesso esperado, veio {other:?}"),
        }
        let state = LicenseState::load(&path, &verifier);
        assert_eq!(
            state.can_create_workspace(1, NOW),
            WorkspaceGate::Allowed,
            "SEM restart: recarregar o estado já destrava o 2º Espaço"
        );
        // O painel T7 reflete o estado real (critério 3).
        assert_eq!(
            plan_panel(&state.effective(NOW), 1),
            PlanPanel::Pro {
                valid_until: Some("Válida até 06/2025".into()),
                usage: "Espaços: 1 em uso — sem limite".into()
            }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §3: variantes do painel — degradado pede re-colagem (gracioso); vencida volta a
    /// free COM a explicação; limite finito mostra «N de M em uso».
    #[test]
    fn plan_panel_variants_match_frozen_copy() {
        let dir = std::env::temp_dir().join(format!("lina-licui-plan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tempdir");
        let path = dir.join("license.json");
        let key = SigningKey::generate(&mut OsRng);
        let verifier = Verifier::with_key(key.verifying_key());

        // Free puro.
        let free = LicenseState::load(&path, &verifier);
        assert_eq!(
            plan_panel(&free.effective(NOW), 1),
            PlanPanel::Free { expired_note: None }
        );

        // PRO com teto finito.
        let mut k = KeyEntry::default();
        k.type_str(&signed_token(&key, &pro_claims(Some(5), None)));
        assert!(k.activate(&path, &verifier, NOW));
        let pro = LicenseState::load(&path, &verifier);
        assert_eq!(
            plan_panel(&pro.effective(NOW), 2),
            PlanPanel::Pro {
                valid_until: None,
                usage: "Espaços: 2 de 5 em uso".into()
            }
        );

        // Vencida DEPOIS de instalada (validade passa): free com a nota do vencimento.
        let mut k = KeyEntry::default();
        k.type_str(&signed_token(&key, &pro_claims(None, Some(NOW + 10))));
        assert!(k.activate(&path, &verifier, NOW));
        let later = NOW + 100_000;
        let expired = LicenseState::load(&path, &verifier);
        match plan_panel(&expired.effective(later), 1) {
            PlanPanel::Free {
                expired_note: Some(note),
            } => {
                assert!(note.contains("venceu em 06/2025"), "{note}");
            }
            other => panic!("free+nota esperado, veio {other:?}"),
        }

        // Adulterado no disco: re-colagem graciosa.
        let raw = std::fs::read_to_string(&path).expect("ler license.json");
        std::fs::write(&path, raw.replace("\"pro\"", "\"deus\"")).expect("adulterar");
        let degraded = LicenseState::load(&path, &verifier);
        assert_eq!(plan_panel(&degraded.effective(NOW), 1), PlanPanel::Repaste);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
