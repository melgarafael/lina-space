//! `agent_modal` — **F1-2-2 (M6/M6-E): o modal de criar/editar Agente** (evolui o M2 da Fase 0).
//!
//! A tese da story: superar o Maestri por **menos** — 4 controles visíveis (motor·nome·papel·
//! pasta), zero YAML/jargão na superfície, co-piloto de papel pelo nome (via o contrato
//! [`crate::role_suggester::RoleSuggester`] — fronteira F1-2-2↔F1-2-3) e quick-start **somente
//! dos motores detectados** (W4-1), nunca lista fixa.
//!
//! Split sedimentado do shell: [`AgentModal`] é **gpui-free e testável** (estados, debounce por
//! tick, validação, dedup de nome, plano de criação); [`render`] é a casca fina gpui. Toda
//! mutação vira evento **só no Criar/Salvar** (kill -9 no meio do preenchimento não deixa nó
//! fantasma — estado do modal é RAM). Wireframe/copy: `ux-flows.md` fluxo (a), autoritativo.
//!
//! **Critério 7 (neutralidade)**: o comando do motor vem do **CLI Profile TOML lido do disco em
//! runtime** ([`load_profiles`] — os profiles embutidos (claude-code, antigravity, gemini) são
//! semeados uma vez e depois SEMPRE lidos do arquivo: trocar o TOML muda o comando sem
//! recompilar). Descoberta de motores roda
//! **fora da thread de UI** e entra por [`AgentModal::set_engines`] (lição W4-1: injetável).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, rgb, text, transparent_black, AnyElement, ClickEvent, Context, FontWeight,
    Pixels, Role, ScrollHandle, Size,
};
use lina_bootstrap::Autonomy;
use lina_cli_profiles::ProfileRegistry;
use lina_core::{CliProfile, DiscoveredCli, Effort};
use lina_host::NodeId;

use crate::ui::{Button, Frame, Modal, Panel, RadiusExt, Space};

use crate::bridge::AgentEngine;
use crate::role_suggester::{humanize, role_registry, RoleSuggester, RoleSuggestion};
use crate::theme;
use crate::WorkspaceView;

// ═══════════════════════════ copy (auditável — critério 2: zero jargão) ═══════════════════════════
// Toda string visível do modal mora AQUI e passa pelo teste `copy_has_no_jargon` (proíbe
// YAML/TOML/profile/terminal em QUALQUER estado — anti-padrões 1/2/5 do fluxo (a)).

pub const COPY_TITLE_CREATE: &str = "Novo Agente";
pub const COPY_TITLE_EDIT: &str = "Editar Agente";
pub const COPY_QUICKSTART: &str = "Início rápido — motores prontos no seu computador:";
pub const COPY_SCANNING: &str = "Procurando motores no seu computador…";
pub const COPY_EMPTY_TITLE: &str = "Vamos preparar tudo para você";
pub const COPY_EMPTY_CTA: &str = "Instalar o motor recomendado";
pub const COPY_INSTALL_OTHER: &str = "+ Instalar outro motor…";
pub const COPY_NAME_LABEL: &str = "Nome";
pub const COPY_NAME_PLACEHOLDER: &str = "Ex.: Revisor, Designer de Landing…";
pub const COPY_ROLE_LABEL: &str = "Papel";
pub const COPY_ROLE_PLACEHOLDER: &str =
    "O papel nasce do nome: digite o nome acima que eu sugiro aqui. Para escolher você mesmo, use o Trocar.";
/// Fix de tela: nome sem padrão conhecido (ex.: "Teles") mostra fallback HONESTO no campo —
/// nunca silêncio (o fundador digitou e "nada aconteceu").
pub const COPY_ROLE_NO_MATCH: &str =
    "Não reconheci um papel nesse nome — escolha um com o Trocar, ou crie assim mesmo (dá para definir depois).";
/// Hint do preview da sugestão DENTRO do campo Papel (aceita por Tab ou clique).
pub const COPY_ROLE_SUGGESTED_HINT: &str = "sugestão — aceite com Tab ou clique";
// ── seletor de papéis (F1-2-3 + requisito do fundador: dropdown, não ciclar) ──
pub const COPY_GALLERY_TITLE: &str = "Escolha o papel";
pub const COPY_GALLERY_SEARCH_PLACEHOLDER: &str = "busque por nome ou pelo que ele faz…";
pub const COPY_GALLERY_EMPTY: &str =
    "Nenhum papel casa com a busca — apague para ver todos, ou comece pelo Nome que eu sugiro.";
pub const COPY_GALLERY_HINT: &str = "↑↓ navega · Enter escolhe · Esc fecha";
pub const COPY_ROLE_SWAP: &str = "Trocar";
// ── editor leve de papel custom (F1-2-3 p2: duplicar + modificar, persistente) ──
pub const COPY_GALLERY_DUPLICATE: &str = "Duplicar";
pub const COPY_TPL_TITLE: &str = "Criar o seu papel a partir de";
pub const COPY_TPL_NAME_LABEL: &str = "Nome";
pub const COPY_TPL_DESC_LABEL: &str = "O que ele faz";
pub const COPY_TPL_DETAILS: &str = "▸ ver detalhes";
pub const COPY_TPL_DETAILS_OPEN: &str = "▾ ver detalhes";
pub const COPY_TPL_DOCTRINE_LABEL: &str = "Instruções completas (modo técnico — opcional)";
pub const COPY_TPL_SAVE: &str = "Salvar papel";
pub const COPY_TPL_BACK: &str = "Voltar";
pub const COPY_TPL_HINT: &str = "↑↓ muda o campo · Esc volta";
pub const COPY_TPL_BADGE: &str = "seu papel";
pub const COPY_TPL_WHY: &str = "Você criou este papel na biblioteca.";
pub const COPY_TPL_ERR_EMPTY_NAME: &str = "Dê um nome ao papel antes de salvar.";
pub const COPY_TPL_ERR_SAVE: &str =
    "Não consegui salvar o papel agora — tente de novo em instantes.";

/// Nome pré-preenchido ao duplicar ("Revisor" → "Revisor (cópia)") — o fundador edita.
#[must_use]
pub fn copy_tpl_dup_name(label: &str) -> String {
    format!("{label} (cópia)")
}
pub const COPY_CWD_LABEL: &str = "Pasta de trabalho";
pub const COPY_CWD_DEFAULT: &str = "a pasta deste Espaço";
pub const COPY_CWD_PICK: &str = "Trocar…";
// ADR 0037 — "reiniciar agora na nova pasta": só aparece no Editar quando a pasta MUDOU. A pasta
// é salva de qualquer forma (vale na próxima abertura); ligar isto re-ergue o Agente já na pasta
// nova, encerrando o que estiver rodando (opt-in explícito — nada se encerra por inércia).
pub const COPY_RESTART_LABEL: &str = "Reiniciar este Agente na nova pasta agora";
pub const COPY_RESTART_DETAIL_ON: &str =
    "Ao salvar, o Agente fecha e abre de novo já na pasta nova. O que estiver rodando agora é \
     encerrado.";
pub const COPY_RESTART_DETAIL_OFF: &str =
    "A pasta nova passa a valer na próxima vez que você abrir o Lina. Nada é encerrado agora.";
// Rodada 360 (ADR 0022 §4) — consentimento do kit em pasta própria. RASCUNHO: o texto
// final passa pelo fundador no gate (escrever na pasta do usuário é decisão de produto).
pub const COPY_CONSENT_LABEL: &str = "Deixar a Lina preparar esta pasta para o time";
pub const COPY_CONSENT_DETAIL: &str =
    "Cria arquivos de orientação (como o CLAUDE.md) que apresentam os colegas ao agente e \
     ligam o acompanhamento de atividade. Um arquivo que já for seu nunca é alterado.";
pub const COPY_CONSENT_OFF_WARN: &str =
    "Sem isso, o agente não conhece o time e não aparece no acompanhamento — o card dele \
     mostra um aviso.";
// ── autonomia (FIX-3, dogfooding 2026-06-10): o fundador levou uma tempestade de sim/não sem
//    saber que vinham dos gates do PRÓPRIO Lina (nível "assistido", default). A seção torna o
//    nível uma escolha CONSCIENTE: 1 linha leiga por opção, sem mudar o default do produto.
//    ("terminal" é jargão proibido aqui — a superfície fala "janela dele".)
pub const COPY_AUTONOMY_LABEL: &str = "Autonomia";

/// F3-0-6: rótulo curto da seção do nível de raciocínio (coluna estreita, par de "Autonomia").
pub const COPY_EFFORT_LABEL: &str = "Raciocínio";
/// F3-0-6: o controle NOMEADO em pt-br (o "porquê" do leigo) — nunca a chave técnica
/// low/medium/high. Mostra também o trade-off de custo, que é o que o empreendedor sente.
pub const COPY_EFFORT_CAPTION: &str =
    "Quanto a IA pensa antes de agir — quanto mais caprichoso, mais caro.";

/// Ordem de exibição das opções (da rédea mais curta à mais solta).
pub const AUTONOMY_OPTIONS: [Autonomy; 3] =
    [Autonomy::Manual, Autonomy::Assisted, Autonomy::Autonomous];

/// Rótulo de SUPERFÍCIE (leigo, com acento) — não confundir com `Autonomy::label()`, que é o
/// vocabulário interno da doutrina/env (`autonomo`, sem acento).
#[must_use]
pub fn autonomy_surface_label(a: Autonomy) -> &'static str {
    match a {
        Autonomy::Manual => "manual",
        Autonomy::Assisted => "assistido",
        Autonomy::Autonomous => "autônomo",
    }
}

/// A descrição leiga de 1 linha por opção (entrega 1 do FIX-3) — diz o que o nível FAZ no dia a
/// dia, fiel à matriz real do guard (`lina_core::guard::decide`).
#[must_use]
pub fn autonomy_desc(a: Autonomy) -> &'static str {
    match a {
        Autonomy::Manual => "só faz o que você mandar, passo a passo.",
        Autonomy::Assisted => {
            "trabalha sozinho, mas pede sua confirmação para ações sensíveis \
             (você verá pedidos de sim/não na janela dele)."
        }
        Autonomy::Autonomous => {
            "trabalha sozinho; só te chama para ações perigosas (apagar, publicar, gastar)."
        }
    }
}

/// Texto do badge do CARD (entrega 2): discreto; no nível assistido carrega o mini-aviso que
/// responde a dúvida do fundador ("de onde vêm esses sim/não?") sem crescer a barra do card.
#[must_use]
pub fn card_autonomy_badge(a: Autonomy) -> String {
    match a {
        Autonomy::Assisted => format!("autonomia: {} · pede sim/não", autonomy_surface_label(a)),
        _ => format!("autonomia: {}", autonomy_surface_label(a)),
    }
}

/// Explicação do sim/não (entrega 3) — SÓ no nível assistido (é quando os pedidos aparecem).
/// Vai no rótulo anunciável do badge; o clique no badge abre o Editar (onde se ajusta).
pub const COPY_CARD_AUTONOMY_EXPLAIN: &str =
    "os pedidos de sim/não na janela são o Agente pedindo sua permissão — ajuste a autonomia em Editar";

#[must_use]
pub fn card_autonomy_explain(a: Autonomy) -> Option<&'static str> {
    matches!(a, Autonomy::Assisted).then_some(COPY_CARD_AUTONOMY_EXPLAIN)
}

/// Rótulo anunciável (AccessKit) do badge do card — explica o sim/não quando assistido.
#[must_use]
pub fn card_autonomy_aria(a: Autonomy) -> String {
    match card_autonomy_explain(a) {
        Some(explain) => format!("autonomia {} — {explain}", autonomy_surface_label(a)),
        None => format!(
            "autonomia {} — {}",
            autonomy_surface_label(a),
            autonomy_desc(a)
        ),
    }
}

/// Par (texto, fundo) do badge de autonomia — FONTE ÚNICA consumida pelo render do card E pelo
/// gate WCAG (teste `autonomy_badge_meets_wcag_in_all_themes`): mudar o par aqui re-gateia.
#[must_use]
pub fn autonomy_badge_colors(th: &crate::theme::Theme) -> (u32, u32) {
    (th.text.primary, th.surface.raised)
}

pub const COPY_ADVANCED: &str = "▸ Avançado";
pub const COPY_ADVANCED_OPEN: &str = "▾ Avançado";
pub const COPY_CMD_LABEL: &str = "Comando do motor (modo técnico)";
pub const COPY_CREATE: &str = "Criar Agente";
pub const COPY_SAVE: &str = "Salvar";
pub const COPY_CANCEL: &str = "Cancelar";
pub const COPY_ERR_EMPTY_NAME: &str = "Dê um nome ao agente antes de criar.";
pub const COPY_ERR_SCANNING: &str = "Um instante — ainda estou procurando os motores.";
pub const COPY_ERR_NO_ENGINE: &str =
    "Nenhum motor instalado ainda — use o botão acima e eu instalo para você.";
pub const COPY_DISCARD_ARMED: &str = "Descartar este Agente? (Esc de novo descarta)";
/// Falha ao criar/salvar (ex.: motor sumiu entre a detecção e o clique) — leiga, sem stack trace;
/// o detalhe técnico vai ao stderr.
pub const COPY_ERR_COMMIT: &str =
    "Não consegui criar agora — o motor pode ter mudado. Tente de novo ou escolha outro.";
pub const COPY_EDIT_ENGINE_RESTART_WARN: &str =
    "⚠ Trocar o motor reinicia este Agente com o novo CLI. O que estiver rodando agora é encerrado.";
pub const COPY_EDIT_APPLY_NOTE: &str = "Mudanças de nome e papel valem agora.";
pub const COPY_WHY_PREFIX: &str = "ⓘ por quê?";

// ── F3-5-3 (doc-fonte 67): seção "Continuar uma conversa" — retomar uma conversa salva (`--resume`)
// sem digitar nada técnico. Zero jargão: o leigo pensa "continuar de onde parei", nunca "sessão"/
// "id"/"motor cru". A copy passa pelo guardião `copy_has_no_jargon` como toda string da tela.
pub const COPY_RESUME_HEADER_CLOSED: &str = "▸ Continuar uma conversa";
pub const COPY_RESUME_HEADER_OPEN: &str = "▾ Continuar uma conversa";
pub const COPY_RESUME_HINT: &str =
    "retome de onde você parou — a conversa volta com tudo o que já foi conversado";
pub const COPY_RESUME_EMPTY: &str =
    "Nenhuma conversa salva ainda. Elas aparecem aqui assim que um agente começa a trabalhar.";
pub const COPY_RESUME_UNAVAILABLE: &str = "indisponível agora";

/// Linha-fantasma do co-piloto (M6-E2): nunca popup, nunca rouba foco.
#[must_use]
pub fn copy_ghost(label: &str) -> String {
    format!("Parece um {label} — usar este papel? (Tab)")
}

/// Aviso leve de nome duplicado (tabela de erros do fluxo (a): sufixo automático, não bloqueia).
#[must_use]
pub fn copy_dup_note(unique: &str) -> String {
    format!("Já existia um com esse nome — este será \u{201c}{unique}\u{201d}.")
}

// ═══════════════════════════ motores (W4-1 → quick-start) ═══════════════════════════

/// Um motor pronto para o quick-start: rótulo leigo + comando (do CLI Profile TOML quando há;
/// senão o binário descoberto, cru).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Engine {
    /// id da descoberta (`"claude"`, `"codex"`, …).
    pub id: String,
    /// Rótulo leigo do chip ("Claude Code").
    pub label: String,
    pub version: Option<String>,
    pub program: String,
    pub args: Vec<String>,
    /// id do CLI Profile TOML quando existe (`"claude-code"`) → vira `CliProfileSet` no log.
    pub profile_id: Option<String>,
}

/// Rótulo leigo de um CLI descoberto (ids do `KNOWN_CLIS` do core).
#[must_use]
pub fn engine_label(id: &str) -> String {
    match id {
        "claude" => "Claude Code".to_string(),
        "codex" => "Codex".to_string(),
        "gemini" => "Gemini CLI".to_string(),
        "opencode" => "OpenCode".to_string(),
        "copilot" => "Copilot CLI".to_string(),
        "agy" => "Antigravity".to_string(),
        other => {
            let mut s = other.to_string();
            if let Some(f) = s.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            s
        }
    }
}

/// Copy LEIGA do aviso de aposentadoria do Gemini CLI (F1-1-9 critério 2 → render no gate de
/// saída F1). A VERDADE vive no profile (`profiles/gemini.toml`: TRANSITIONAL / fim 2026-06-18;
/// sucessor em `profiles/antigravity.toml`) — aqui só a tradução aprovada para a tela. Zero
/// jargão (sem "EoL"/"deprecated"); informa, NUNCA bloqueia a escolha.
pub const COPY_GEMINI_TRANSITION: &str = "O Google está aposentando o Gemini CLI: ele deixa de \
ser atualizado em 18/jun/2026. O sucessor é o Antigravity — quando puder, prefira ele. Dá para \
criar este agente mesmo assim; o aviso é só para você não ser pego de surpresa.";

/// Aviso de transição/aposentadoria de um motor, por id da descoberta — mesmo padrão de
/// conhecimento-por-id do [`engine_label`]. `None` = nada a avisar.
#[must_use]
pub fn engine_transition_notice(id: &str) -> Option<&'static str> {
    match id {
        "gemini" => Some(COPY_GEMINI_TRANSITION),
        _ => None,
    }
}

/// Monta a lista de motores do quick-start a partir da DESCOBERTA (W4-1) + os profiles TOML do
/// disco. **Recomendado primeiro** (claude), demais na ordem da descoberta. Puro e testável.
#[must_use]
pub fn engines_from(found: &[DiscoveredCli], profiles: &ProfileRegistry) -> Vec<Engine> {
    let mut out: Vec<Engine> = found
        .iter()
        .map(|d| {
            // O profile cujo `program` é o binário descoberto dá comando/args/identidade.
            let profile = profiles.ids().find_map(|id| {
                let p = profiles.get(id)?;
                (p.program == d.id).then_some(p)
            });
            Engine {
                id: d.id.clone(),
                label: engine_label(&d.id),
                version: d.version.clone(),
                program: profile.map_or_else(|| d.id.clone(), |p| p.program.clone()),
                args: profile.map(|p| p.args.clone()).unwrap_or_default(),
                profile_id: profile.map(|p| p.id.clone()),
            }
        })
        .collect();
    // Recomendado (claude) primeiro — estabilidade no resto (sem reordenar surpresa).
    out.sort_by_key(|e| if e.id == "claude" { 0 } else { 1 });
    out
}

/// Estado da varredura de motores (a descoberta roda FORA da thread de UI e entra por
/// [`AgentModal::set_engines`] — lição W4-1: off-thread + injetável p/ teste determinístico).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineScan {
    Scanning,
    Ready(Vec<Engine>),
}

/// **F3-5-3 — uma linha da seção "Continuar uma conversa".** O que a tela MOSTRA (nome amigável +
/// idade humanizada) + o comando de religação JÁ montado (via `lina_core::resume_session::
/// resume_command`, com o verbo de resume do CLI + o `session_id` exato). Nenhum dado técnico
/// (`session_id`/`cli`) aparece na tela: ele viaja só dentro de `command`, consumido pelo funil de
/// admissão. `resumable == false` ⇒ o motor da conversa sumiu/não retoma → linha apagada e inerte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSessionRow {
    pub display_name: String,
    /// Idade humanizada ("ontem", "há 3 dias") — calculada na montagem (gpui-free no render).
    pub age: String,
    /// `[program, args…, resume_args…, session_id]`; vazio quando `resumable == false`.
    pub command: Vec<String>,
    pub profile_id: Option<String>,
    pub role: Option<String>,
    pub cwd: Option<String>,
    pub resumable: bool,
}

/// **F3-5-3 — idade humanizada de uma conversa salva** (para o leigo reconhecer "a de ontem").
/// PURA e em buckets (sem precisão falsa de minutos): a tela quer significado, não cronômetro.
/// `now`/`then` em ms epoch; `now < then` (relógio não-monotônico) cai em "agora há pouco".
#[must_use]
pub fn humanize_age(now_ms: u64, then_ms: u64) -> String {
    let mins = now_ms.saturating_sub(then_ms) / 60_000;
    let hours = mins / 60;
    let days = hours / 24;
    if mins < 1 {
        "agora há pouco".to_string()
    } else if hours < 1 {
        format!("há {mins} min")
    } else if days < 1 {
        format!("há {hours}h")
    } else if days == 1 {
        "ontem".to_string()
    } else if days < 7 {
        format!("há {days} dias")
    } else if days < 14 {
        "há 1 semana".to_string()
    } else {
        format!("há {} semanas", days / 7)
    }
}

// ═══════════════════════════ CLI Profiles em RUNTIME (critério 7) ═══════════════════════════

/// CLI Profiles embutidos — escritos UMA vez no dir de profiles do usuário; depois disso o
/// arquivo do DISCO manda (editar/trocar o TOML muda o comando sem recompilar — invariante #3).
/// Antigravity + Gemini entram por F1-1-9: sem este seeding, um TOML novo no repo NÃO chega ao
/// dir de runtime do usuário (o modal só lê o disco).
const CLAUDE_PROFILE_SEED: &str = include_str!("../../../profiles/claude-code.toml");
const ANTIGRAVITY_PROFILE_SEED: &str = include_str!("../../../profiles/antigravity.toml");
const GEMINI_PROFILE_SEED: &str = include_str!("../../../profiles/gemini.toml");
/// A2A UNIVERSAL: o profile do Codex (`prompt_ready`/`busy_markers` calibrados p/ a TUI dele)
/// também precisa chegar ao dir de runtime do usuário, senão a entrega A2A ao Codex usaria o
/// fallback do Claude e voltaria a dar timeout (o modal/registry só lê o disco).
const CODEX_PROFILE_SEED: &str = include_str!("../../../profiles/codex.toml");

/// `(nome de arquivo, conteúdo)` de cada profile embutido, semeado **write-if-absent**: o
/// arquivo do disco, uma vez escrito, é a fonte da verdade (um custom de mesmo `id` no dir
/// do usuário continua sobrescrevendo o default por `id`, via `ProfileRegistry`).
const SEEDED_PROFILES: &[(&str, &str)] = &[
    ("claude-code.toml", CLAUDE_PROFILE_SEED),
    ("antigravity.toml", ANTIGRAVITY_PROFILE_SEED),
    ("gemini.toml", GEMINI_PROFILE_SEED),
    ("codex.toml", CODEX_PROFILE_SEED),
];

/// Onde os CLI Profiles do usuário moram: `LINA_PROFILES` (override de dev) ou
/// `<ws_root>/.lina/profiles` (local-first, por Espaço).
#[must_use]
pub fn profiles_dir(mailbox_dir: &Path) -> PathBuf {
    std::env::var_os("LINA_PROFILES")
        .map(PathBuf::from)
        .unwrap_or_else(|| mailbox_dir.join("profiles"))
}

/// Carrega os CLI Profiles TOML **do disco**, semeando os profiles embutidos
/// ([`SEEDED_PROFILES`]) que ainda não existem. Best-effort: erro de I/O/parse → registry
/// vazio + stderr (o modal degrada para o binário descoberto; criação nunca bloqueia).
#[must_use]
pub fn load_profiles(dir: &Path) -> ProfileRegistry {
    // Semeia cada profile embutido write-if-absent: o arquivo do disco, uma vez presente,
    // manda (o usuário pode editá-lo). Sem isso, Antigravity/Gemini (F1-1-9) nunca chegariam
    // ao dir de runtime — o modal só lê o disco.
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("lina-gpui: M6 — não criei {} ({e})", dir.display());
    } else {
        for &(name, seed) in SEEDED_PROFILES {
            let path = dir.join(name);
            if !path.exists() {
                if let Err(e) = std::fs::write(&path, seed) {
                    eprintln!("lina-gpui: M6 — não semeei {} ({e})", path.display());
                }
            }
        }
    }
    match ProfileRegistry::load_dir(dir) {
        Ok(reg) => reg,
        Err(e) => {
            eprintln!(
                "lina-gpui: M6 — profiles de {} inválidos ({e})",
                dir.display()
            );
            ProfileRegistry::new()
        }
    }
}

// ═══════════════════════════ o modelo (gpui-free) ═══════════════════════════

/// Ticks do loop do modal (~50ms cada) até o co-piloto disparar após a pausa de digitação
/// (M6-E2: ~600ms). 12 × 50ms = 600ms.
pub const SUGGEST_DEBOUNCE_TICKS: u32 = 12;

// ═══════════ contenção do seletor (fix de tela bugB2 — matemática PURA, testável) ═══════════
// O dropdown da galeria ESTOURAVA o modal e a janela (lista inteira inline, sem viewport; caixa
// de 640px fixa em janela estreita). gpui não roda headless → o clamp vive AQUI, em f32 puro,
// com teste unitário; o render só APLICA os números. As constantes são FONTE ÚNICA: a mesma
// tabela alimenta a conta e os `.h(px(..))`/`.line_height(px(..))` do render — se divergirem, a
// matemática mente (teste `gallery_row_math_matches_render_constants` guarda a amarração).

/// Largura de projeto da caixa do modal (a de antes do fix — preservada quando há espaço).
pub const MODAL_W: f32 = 640.0;
/// Piso da caixa em janela minúscula (abaixo disso o conteúdo vira ruído).
pub const MODAL_W_MIN: f32 = 320.0;
/// Respiro mínimo entre a caixa e CADA borda da janela (requisito 3: nunca colar/estourar).
pub const WINDOW_MARGIN: f32 = 16.0;
/// Padding horizontal da caixa (px_6) — a largura útil do modal é `w - 2*MODAL_PAD_X`.
pub const MODAL_PAD_X: f32 = 24.0;
/// Padding interno do dropdown (px_3/py_3).
pub const GALLERY_PAD: f32 = 12.0;
/// gap_2 entre cabeçalho · busca · lista.
pub const GALLERY_GAP: f32 = 8.0;
/// border_1 do container do dropdown — conta no chrome (taffy é border-box: a borda come o
/// espaço interno; sem ela na soma, a lista clipa 2px).
pub const GALLERY_BORDER: f32 = 1.0;
/// line_height EXPLÍCITA de cada linha de texto (lição do projeto: a natural da fonte clipa).
pub const GALLERY_LINE_H: f32 = 20.0;
/// Cabeçalho do dropdown (1 linha: título + hint).
pub const GALLERY_HEADER_H: f32 = 20.0;
/// Caixa de busca (py_2 + 1 linha) — FIXA no topo (requisito 4); só a lista rola.
pub const GALLERY_SEARCH_H: f32 = 36.0;
/// Linha da lista: 2 linhas de texto + py_2 + border_2 dos dois lados (geometria UNIFORME — a
/// inativa usa borda transparente, então navegar nunca reflui o layout).
pub const GALLERY_ROW_H: f32 = 60.0;
/// Espaço entre linhas da lista (gap_1, via mb por linha — o viewport de scroll é bloco).
pub const GALLERY_ROW_GAP: f32 = 4.0;
/// Teto de conforto da viewport (~6,4 linhas): em tela grande a lista NÃO vira um totem — o
/// corte não-inteiro de linha sinaliza visualmente que rola.
pub const GALLERY_VIEWPORT_MAX: f32 = 384.0;
/// Orçamento vertical do modal SEM a lista (título + chips + nome + papel + pasta + avançado +
/// rodapé + gaps + paddings), estimado por cima de propósito: sobrar é seguro, faltar estoura.
pub const MODAL_BASE_RESERVED: f32 = 500.0;

/// F3-5-3: teto da lista de conversas salvas (~4 linhas). Em const NOMEADA (não `px(<literal>)`)
/// para a viewport rolar sem pesar a catraca de tokens — o corte não-inteiro sinaliza que rola.
pub const RESUME_VIEWPORT_MAX: f32 = 232.0;

/// Tamanho lógico em px — `f32` cru (gpui-free) para o teste rodar headless.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WinSize {
    pub w: f32,
    pub h: f32,
}

/// O quadro da caixa do modal DENTRO da janela: largura clampada + teto de altura.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModalFrame {
    /// Largura final da caixa (`MODAL_W` quando cabe; encolhe em janela estreita).
    pub w: f32,
    /// Teto de altura da caixa (janela − 2×margem) — o render aplica como `max_h`.
    pub max_h: f32,
}

/// Quadro do modal para uma janela: NUNCA mais largo/alto que a janela menos as margens
/// (era o estouro horizontal: `w(px(640))` fixo em janela estreita).
#[must_use]
pub fn modal_frame(window: WinSize) -> ModalFrame {
    ModalFrame {
        w: MODAL_W.min(window.w - 2.0 * WINDOW_MARGIN).max(MODAL_W_MIN),
        max_h: (window.h - 2.0 * WINDOW_MARGIN).max(160.0),
    }
}

/// Altura do "chrome" do dropdown (tudo menos a viewport da lista): bordas + paddings + gaps +
/// cabeçalho + busca.
#[must_use]
pub fn gallery_chrome_h() -> f32 {
    2.0 * GALLERY_BORDER
        + 2.0 * GALLERY_PAD
        + 2.0 * GALLERY_GAP
        + GALLERY_HEADER_H
        + GALLERY_SEARCH_H
}

/// Bounds finais do dropdown + viewport da lista (o contrato do fix).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GalleryLayout {
    /// Largura do dropdown == largura ÚTIL do modal (requisito 1: nunca mais largo).
    pub w: f32,
    /// Altura total do dropdown (chrome + viewport).
    pub h: f32,
    /// Altura da VIEWPORT da lista — o conteúdo rola POR DENTRO quando excede (requisito 2).
    pub list_viewport_h: f32,
    /// O conteúdo excede a viewport? (o render usa a viewport igual; a flag é dos testes/UX.)
    pub scrolls: bool,
}

/// **A conta do fix**: dado o quadro do modal + a janela + nº de itens filtrados → bounds do
/// dropdown e viewport da lista. Clamp em 3 camadas: conteúdo justo (sem buraco morto) ≤ espaço
/// vertical disponível (janela − orçamento base − chrome) ≤ teto de conforto — com PISO de 1
/// linha (janela degenerada mantém o seletor usável; o residual é absorvido pelo scroll do
/// corpo do modal, fallback declarativo do render — nunca pintado por cima do canvas).
#[must_use]
pub fn gallery_layout(modal: ModalFrame, window: WinSize, n_items: usize) -> GalleryLayout {
    let w = modal.w - 2.0 * MODAL_PAD_X;
    let content = if n_items == 0 {
        GALLERY_ROW_H // a linha única da mensagem de "nenhum papel casa".
    } else {
        n_items as f32 * GALLERY_ROW_H + (n_items as f32 - 1.0) * GALLERY_ROW_GAP
    };
    // Teto REAL: o menor entre o quadro recebido e a própria janela (robusto a um frame que
    // não veio de `modal_frame`).
    let ceiling = (window.h - 2.0 * WINDOW_MARGIN).min(modal.max_h);
    let space = ceiling - MODAL_BASE_RESERVED - gallery_chrome_h();
    // clamp seguro: ROW_H (60) ≤ VIEWPORT_MAX (384) por construção — nunca panica.
    let cap = space.clamp(GALLERY_ROW_H, GALLERY_VIEWPORT_MAX);
    let list_viewport_h = content.min(cap);
    GalleryLayout {
        w,
        h: gallery_chrome_h() + list_viewport_h,
        list_viewport_h,
        scrolls: content > list_viewport_h + 0.5,
    }
}

// ── papéis customizados (F1-2-3 p2 — duplicar + modificar, persistente) ──

/// Um papel CUSTOM do usuário, como REAPARECE do log (payload de `RoleTemplateSaved`, sem o
/// `ts` — a ordem de log basta para o last-wins). Contrato do seam com o bridge:
/// `NodeManager::role_templates()` devolve estas linhas EM ORDEM DE LOG, SEM dedup (o modal
/// aplica último-`name`-vence, padrão `CostLedger` — doutrina do próprio evento no core);
/// `NodeManager::save_role_template(name, description, doctrine)` faz o append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleTemplate {
    /// Nome leigo ("Revisora de Landing") — também a CHAVE do last-wins.
    pub name: String,
    /// Descrição curta leiga (1 frase "o que ele faz").
    pub description: String,
    /// Instruções completas (texto técnico do bootstrap turno-0) — atrás de "ver detalhes",
    /// nunca obrigatória (progressive disclosure).
    pub doctrine: String,
}

// ── linhas texto+ação (fix bugB3 — o botão é INEGOCIÁVEL, o texto cede) ──
// Regressão da rodada bugB2: o min-content de um texto gpui é a LINHA INTEIRA quando alguma
// medição roda sem largura Definite (text.rs: `wrap_width` exige Definite; e o cache de
// TextLayout aceita hit com wrap_width=None → o min-content que o taffy enxerga depende da
// ORDEM das medições — inserir o wrapper de scroll mudou a ordem). Célula sem `min_w(0)` herda
// esse min-content gigante, não encolhe e EXPULSA o botão da linha (o scroll container clipa →
// [Trocar] sumiu; a galeria ficou inalcançável). Doutrina de TODA linha texto+ação do modal:
// label `flex_none` · célula de texto `flex_1 + min_w(0) + overflow_hidden` (quem cede) ·
// ação `flex_none` (nunca encolhe, nunca sai). A matemática abaixo espelha essa resolução.

/// Largura da label esquerda das linhas do formulário (Nome/Papel/Pasta) — fonte única.
pub const ROW_LABEL_W: f32 = 120.0;
/// Gap entre label · célula · ação nas linhas do formulário — o render usa `gap(px(ROW_GAP))`
/// (fonte única com a varredura de contenção, não coincidência com um token de espaçamento).
pub const ROW_GAP: f32 = 12.0;

/// Geometria resolvida de uma linha texto+ação (label fixa à esquerda, texto que cede no
/// meio, ação inegociável à direita). Espelho-de-spec da doutrina acima: existe PARA o teste
/// guardião (`cfg(test)` — gpui não roda headless; o render real usa flex com a doutrina, e a
/// varredura prova aqui que a geometria é viável em toda largura do `modal_frame`).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActionRowLayout {
    /// Largura da célula de texto (o que CEDE; nunca negativa).
    pub text_w: f32,
    /// x da borda esquerda da ação, relativo à linha.
    pub action_x: f32,
    /// Largura da ação (clampada à linha — jamais maior que ela).
    pub action_w: f32,
}

/// Layout PURO da linha texto+ação sob a doutrina do fix: espelha a resolução flex do taffy
/// para `label flex_none · célula min_w(0) · ação flex_none` — a ação cola à direita SEMPRE
/// dentro da linha; o texto fica com o resto (zero no aperto). O caso degenerado (partes fixas
/// maiores que a linha) clampa a ação para dentro — o teste de varredura prova que a geometria
/// real do modal nunca o atinge.
#[cfg(test)]
#[must_use]
pub fn action_row_layout(row_w: f32, label_w: f32, gap: f32, action_w: f32) -> ActionRowLayout {
    let action_w = action_w.min(row_w);
    let action_x = row_w - action_w;
    let text_w = (row_w - label_w - action_w - 2.0 * gap).max(0.0);
    ActionRowLayout {
        text_w,
        action_x,
        action_w,
    }
}

/// **Seletor de papéis** ([Trocar] — F1-2-3 + feedback de tela do fundador: "tinha que ser um
/// seletor ou dropdown", não ciclar de um em um). TODOS os papéis da biblioteca visíveis de uma
/// vez (a biblioteca É o registry W3-1 humanizado — fonte única, offline, zero LLM), com busca
/// por texto (nome leigo OU descrição) e navegação por teclado (↑↓ + Enter; Esc fecha).
/// gpui-free e testável — espelha o idioma da paleta M1. (Exceção consciente: o `scroll` é um
/// `gpui::ScrollHandle` — um Rc-cell INERTE, sem janela/plataforma — para o ↑↓ manter a linha
/// destacada visível dentro da viewport; construí-lo em teste headless é seguro.)
pub struct RoleGallery {
    /// Embutidos (registry W3-1 humanizado) + customs (last-wins JÁ aplicado), nesta ordem.
    items: Vec<GalleryItem>,
    query: String,
    /// Índice selecionado DENTRO da lista filtrada (teclado ↑↓; clamp ao filtrar).
    selected: usize,
    /// F1-2-3 p2: editor leve aberto por 'Duplicar' (None = lista normal). Enquanto aberto, o
    /// funil de teclado do main.rs (fixo) é REINTERPRETADO pelos métodos deste modelo.
    editor: Option<TemplateEditor>,
    /// Scroll da viewport da lista (one-shot `scroll_to_item` ao navegar por teclado — não
    /// briga com o wheel: fora da navegação o offset é do usuário).
    scroll: ScrollHandle,
}

/// Item da galeria: a sugestão (o contrato que o pick devolve ao modal — INALTERADO) +
/// metadados da biblioteca (é custom? instruções por trás de "ver detalhes").
pub struct GalleryItem {
    pub suggestion: RoleSuggestion,
    /// `true` = papel salvo pelo usuário (badge "seu papel"; duplicar copia a doutrina).
    pub custom: bool,
    /// Instruções completas — vazia nos embutidos (o registry W3-1 não carrega doutrina; o
    /// canal de bootstrap deles segue o existente da W3-2).
    pub doctrine: String,
}

/// Campo focado do editor leve (↑↓ circula; Doutrina só entra com detalhes abertos).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorField {
    Name,
    Description,
    Doctrine,
}

/// Editor LEVE de papel custom (modelo duplicar+modificar — F1-2-3 p2). gpui-free e testável;
/// nada persiste até o clique em [Salvar papel] (kill -9 no meio não deixa lixo — estado RAM,
/// mesmo princípio do modal).
pub struct TemplateEditor {
    /// Rótulo do template de ORIGEM (título "Criar o seu papel a partir de — X").
    pub source: String,
    pub name: String,
    pub description: String,
    pub doctrine: String,
    pub focus: EditorField,
    /// Progressive disclosure: instruções completas só com detalhes ABERTOS (nunca obrigatória).
    pub details: bool,
    /// Erro leigo do último salvar (limpo ao digitar).
    pub error: Option<String>,
}

/// last-wins por `name` (doutrina do evento `RoleTemplateSaved`: "último name vence", padrão
/// `CostLedger`) — PURO e testável; o seam do bridge entrega ordem de log SEM dedup.
#[must_use]
fn dedupe_last_wins(rows: Vec<RoleTemplate>) -> Vec<RoleTemplate> {
    let mut out: Vec<RoleTemplate> = Vec::new();
    for row in rows {
        match out.iter_mut().find(|t| t.name == row.name) {
            Some(slot) => *slot = row, // último vence; posição da 1ª aparição preservada.
            None => out.push(row),
        }
    }
    out
}

impl RoleGallery {
    /// Monta a biblioteca a partir do registry W3-1 (humanizado): nome leigo + 1 frase por papel.
    #[must_use]
    pub fn from_library() -> Self {
        Self {
            items: Self::builtin_items(),
            query: String::new(),
            selected: 0,
            editor: None,
            scroll: ScrollHandle::new(),
        }
    }

    /// Os embutidos (registry W3-1 humanizado) — a base sobre a qual os customs entram.
    fn builtin_items() -> Vec<GalleryItem> {
        role_registry()
            .map(|reg| {
                reg.role_names()
                    .map(|canon| {
                        let (label, blurb) = humanize(canon);
                        GalleryItem {
                            suggestion: RoleSuggestion {
                                role: canon.to_string(),
                                label,
                                blurb,
                                why: "Você escolheu este papel na biblioteca.".to_string(),
                            },
                            custom: false,
                            doctrine: String::new(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// F1-2-3 p2: injeta os papéis CUSTOM (linhas do log, ordem de log) — aplica o last-wins e
    /// reconstrói a lista (embutidos primeiro, customs depois, com badge). Query/seleção
    /// sobrevivem (clamp no `selected()`).
    pub fn set_customs(&mut self, rows: Vec<RoleTemplate>) {
        let mut items = Self::builtin_items();
        items.extend(dedupe_last_wins(rows).into_iter().map(|t| GalleryItem {
            suggestion: RoleSuggestion {
                role: t.name.clone(), // canônico do custom = o próprio nome (roster/VIBE_ROLE).
                label: t.name,
                blurb: t.description,
                why: COPY_TPL_WHY.to_string(),
            },
            custom: true,
            doctrine: t.doctrine,
        }));
        self.items = items;
    }

    /// 'Duplicar' no item `idx` da lista FILTRADA → abre o editor leve pré-preenchido (nome
    /// "(cópia)", descrição igual, instruções copiadas — vazias num embutido). Foco no Nome;
    /// detalhes fechados (progressive disclosure).
    pub fn duplicate(&mut self, idx: usize) {
        // extrai os campos ANTES de tocar self.editor (filtered() empresta self).
        let Some((source, description, doctrine)) = self.filtered().get(idx).map(|it| {
            (
                it.suggestion.label.clone(),
                it.suggestion.blurb.clone(),
                it.doctrine.clone(),
            )
        }) else {
            return;
        };
        self.editor = Some(TemplateEditor {
            name: copy_tpl_dup_name(&source),
            source,
            description,
            doctrine,
            focus: EditorField::Name,
            details: false,
            error: None,
        });
    }

    /// O editor leve aberto (None = lista normal).
    #[must_use]
    pub fn editor(&self) -> Option<&TemplateEditor> {
        self.editor.as_ref()
    }

    pub fn editor_mut(&mut self) -> Option<&mut TemplateEditor> {
        self.editor.as_mut()
    }

    /// Fecha SÓ o editor (volta à lista) — o Esc do funil usa o downgrade em dois passos.
    pub fn editor_close(&mut self) {
        self.editor = None;
    }

    /// Handle do scroll da lista — o render amarra na viewport via `track_scroll`.
    #[must_use]
    pub fn scroll_handle(&self) -> &ScrollHandle {
        &self.scroll
    }

    /// Os papéis que casam com a busca (case-insensitive, nome leigo OU descrição — vale
    /// também pros customs). Sem busca → TODOS de uma vez (o requisito do fundador).
    #[must_use]
    pub fn filtered(&self) -> Vec<&GalleryItem> {
        let q = self.query.trim().to_lowercase();
        self.items
            .iter()
            .filter(|it| {
                q.is_empty()
                    || it.suggestion.label.to_lowercase().contains(&q)
                    || it.suggestion.blurb.to_lowercase().contains(&q)
            })
            .collect()
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Índice selecionado na lista FILTRADA (p/ o render destacar com o focus ring).
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected.min(self.filtered().len().saturating_sub(1))
    }

    /// Digitar: com o editor aberto vai pro CAMPO focado (e limpa o erro); senão é a busca.
    pub fn type_char(&mut self, s: &str) {
        if let Some(ed) = self.editor.as_mut() {
            ed.error = None;
            ed.field_mut().push_str(s);
            return;
        }
        self.query.push_str(s);
        self.selected = 0;
        self.scroll.scroll_to_item(0); // filtro novo → lista volta ao topo.
    }

    pub fn backspace(&mut self) {
        if let Some(ed) = self.editor.as_mut() {
            ed.error = None;
            ed.field_mut().pop();
            return;
        }
        self.query.pop();
        self.selected = 0;
        self.scroll.scroll_to_item(0);
    }

    /// ↑↓ com clamp nas bordas (sem wrap — previsível pro leitor de tela). Com o editor aberto,
    /// ↑↓ troca o CAMPO focado (Doutrina só entra com detalhes abertos — progressive
    /// disclosure); na lista, a viewport SEGUE a seleção (scroll mínimo one-shot).
    pub fn move_selection(&mut self, delta: i32) {
        if let Some(ed) = self.editor.as_mut() {
            ed.move_focus(delta);
            return;
        }
        let len = self.filtered().len();
        if len == 0 {
            return;
        }
        let cur = self.selected() as i32;
        self.selected = (cur + delta).clamp(0, len as i32 - 1) as usize;
        self.scroll.scroll_to_item(self.selected);
    }

    /// O papel sob o cursor (Enter) — `None` com a busca vazia de resultados, e `None` com o
    /// editor aberto (Enter NÃO escolhe nem salva no meio da edição — salvar é o botão).
    #[must_use]
    pub fn pick(&self) -> Option<RoleSuggestion> {
        if self.editor.is_some() {
            return None;
        }
        self.filtered()
            .get(self.selected())
            .map(|it| it.suggestion.clone())
    }

    /// O papel na posição `idx` da lista FILTRADA (clique do mouse).
    #[must_use]
    pub fn pick_at(&self, idx: usize) -> Option<RoleSuggestion> {
        if self.editor.is_some() {
            return None;
        }
        self.filtered().get(idx).map(|it| it.suggestion.clone())
    }
}

impl TemplateEditor {
    /// O buffer do campo focado (o funil de digitação escreve aqui).
    fn field_mut(&mut self) -> &mut String {
        match self.focus {
            EditorField::Name => &mut self.name,
            EditorField::Description => &mut self.description,
            EditorField::Doctrine => &mut self.doctrine,
        }
    }

    /// ↑↓ entre os campos, com clamp (sem wrap) — Doutrina só com detalhes abertos.
    fn move_focus(&mut self, delta: i32) {
        let order: &[EditorField] = if self.details {
            &[
                EditorField::Name,
                EditorField::Description,
                EditorField::Doctrine,
            ]
        } else {
            &[EditorField::Name, EditorField::Description]
        };
        let cur = order.iter().position(|f| *f == self.focus).unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, order.len() as i32 - 1) as usize;
        self.focus = order[next];
    }
}

/// Criar do zero ou editar um nó vivo (M6-E).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalMode {
    Create,
    Edit { node: NodeId },
}

/// Qual buffer de texto recebe o teclado (Nome é o foco inicial; Comando só no Avançado).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusField {
    Name,
    Command,
}

/// O plano de CRIAÇÃO commitado pelo `[[ Criar Agente ]]` — só aqui nascem eventos (critério 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePlan {
    /// Nome final (dedup com sufixo "(2)" aplicado).
    pub name: String,
    /// Aviso leve quando o dedup agiu (tabela de erros: não bloqueia).
    pub dup_note: Option<String>,
    pub engine: AgentEngine,
    /// Pasta escolhida (None = pasta gerenciada do Espaço).
    pub cwd: Option<PathBuf>,
    /// Papel CANÔNICO escolhido/aceito (None = inferir do nome, comportamento M2).
    pub role: Option<String>,
    /// Rodada 360 (ADR 0022 §4): o usuário AUTORIZOU a Lina a criar os arquivos de
    /// orquestração na pasta própria escolhida. Só tem efeito com `cwd: Some` — sem
    /// consentimento o agente nasce com o badge "sem doutrina/observabilidade".
    pub kit_consent: bool,
    /// FIX-3: nível de autonomia ESCOLHIDO no modal (default do produto: assistido, intocado).
    /// COSTURA (bridge, dono Dev 01): `create_agent_with`/`NodeAdmission` ainda não recebem o
    /// campo — o funil precisa carimbar `.env("LINA_AUTONOMY", label)` p/ o guard por-agente.
    pub autonomy: Autonomy,
    /// F3-0-3 (ADR 0031): modelo/effort escolhidos para o novo terminal (contrato de dados do
    /// modal). `None` = usar o default (o CLI escolhe o modelo; effort efetivo Medium). A SELEÇÃO
    /// interativa (controles visíveis + badge) é F3-0-6 (Frontend, gate de tela do fundador); o
    /// consumo no PTY (`LINA_EFFORT` + `EffortAssigned`) é F3-0-4. Aqui só existe a CASA aditiva.
    pub model: Option<String>,
    pub effort: Option<Effort>,
}

/// O plano de SALVAR do modo edição (M6-E): aplica nome/papel agora; motor fica p/ F1-2-4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavePlan {
    pub node: NodeId,
    pub name: String,
    /// Papel canônico a gravar (None = não mudou).
    pub role: Option<String>,
    /// FIX-3: autonomia a aplicar (None = não mudou) — mesma semântica do `role`.
    pub autonomy: Option<Autonomy>,
    /// ADR 0037: nova Pasta de Trabalho a gravar (`None` = não mudou — mesma semântica de
    /// `role`/`autonomy`). Persiste via `NodeCwdSet`; o restore re-entra nela. `Some` só
    /// quando o caminho escolhido DIFERE do cwd atual do nó.
    pub cwd: Option<PathBuf>,
    /// ADR 0037: o usuário pediu para RE-ERGUER o nó na nova pasta AGORA (`false` = só na
    /// próxima abertura, default conservador). Só tem efeito com `cwd: Some`.
    pub restart_now: bool,
    /// ADR 0039: novo MOTOR a aplicar (`None` = não mudou). Trocar o motor re-ergue o Agente com
    /// o novo CLI (o cérebro de um terminal vivo não troca sem reiniciar o processo).
    pub engine: Option<AgentEngine>,
}

/// O modal M6/M6-E — estado puro (nenhum gpui; nenhum evento até Criar/Salvar).
pub struct AgentModal {
    pub mode: ModalMode,
    name: String,
    /// Ticks desde a última tecla no Nome (None = sem digitação pendente de sugestão).
    typing_ticks: Option<u32>,
    /// Linha-fantasma corrente (não aplicada — some se ignorada).
    suggestion: Option<RoleSuggestion>,
    /// Cartão de Papel APLICADO (Tab/clique/Trocar). Nunca aplicado silenciosamente.
    role: Option<RoleSuggestion>,
    /// `true` → o papel veio de escolha manual ([Trocar]) e NÃO deve ser re-sugerido por cima.
    role_pinned: bool,
    engines: EngineScan,
    selected_engine: usize,
    /// ADR 0039: o `profile_id` do motor ATUAL do nó (modo EDITAR) — referência para detectar a
    /// troca de motor no Salvar e para pré-selecionar o chip certo quando a descoberta chega.
    /// `None` no Create ou em nó sem CLI (shell puro).
    original_profile_id: Option<String>,
    /// ADR 0039: o usuário CLICOU num chip de motor (no Editar). Sem isto, o reset de
    /// `selected_engine` para 0 quando a descoberta chega marcaria uma "troca" que ninguém pediu.
    engine_touched: bool,
    /// Pasta escolhida (None = default leigo "a pasta deste Espaço"). No modo EDITAR nasce com
    /// o cwd REAL do nó (ADR 0037) — o campo mostra a pasta de verdade, não o default genérico.
    cwd: Option<PathBuf>,
    /// ADR 0037: o cwd ORIGINAL do nó ao abrir o Editar — para detectar mudança real no Salvar
    /// (mesmo papel do `original` para nome/papel). `None` no Create.
    original_cwd: Option<PathBuf>,
    /// ADR 0037: o usuário marcou "reiniciar agora na nova pasta" (toggle que só aparece no
    /// Editar quando a pasta mudou). Default `false` = só na próxima abertura (não encerra um
    /// processo vivo por inércia).
    restart_now: bool,
    /// Rodada 360 (ADR 0022 §4): consentimento p/ a Lina criar os arquivos de orquestração
    /// na pasta própria. Opt-in EXPLÍCITO (nasce `false`); irrelevante com `cwd: None`.
    kit_consent: bool,
    advanced: bool,
    /// Override do comando (Avançado). None = derivado do motor selecionado.
    command: Option<String>,
    focus: FocusField,
    error: Option<String>,
    /// Esc com conteúdo arma a confirmação de descarte (2º Esc fecha).
    discard_armed: bool,
    /// Tooltip "por quê" aberto (transparência da sugestão).
    why_open: bool,
    /// O co-piloto RODOU (3+ chars, pausa) e não reconheceu papel — o campo mostra o fallback
    /// honesto em vez de silêncio (fix de tela). Limpa ao sugerir/aceitar/trocar.
    no_match: bool,
    /// Nomes já existentes no Espaço (dedup "(2)").
    existing: Vec<String>,
    /// [Trocar] aberto: o seletor de papéis (dropdown — F1-2-3; fundador: "não ciclar").
    gallery: Option<RoleGallery>,
    suggester: Arc<dyn RoleSuggester>,
    /// (M6-E) nome/papel/autonomia originais — p/ detectar mudança real no Salvar.
    original: Option<(String, Option<String>, Autonomy)>,
    /// FIX-3: nível de autonomia selecionado na seção AUTONOMIA (criar E editar).
    autonomy: Autonomy,
    /// F3-0-6: nível de raciocínio selecionado na seção RACIOCÍNIO (só CRIAR — vai ao spawn via
    /// `CreatePlan.effort`). `None` = casa não-tocada → o controle exibe o default [`Effort::Medium`].
    effort: Option<Effort>,
    /// F3-5-3: a seção "Continuar uma conversa" está aberta (só no modo CRIAR). Default fechado —
    /// o caminho primário é criar; retomar é um atalho que o usuário expande.
    saved_open: bool,
    /// F3-5-3: as conversas salvas já foram carregadas (lazy: na 1ª abertura da seção). Evita
    /// reler o log a cada toggle e distingue "ainda não carregou" de "carregou e está vazio".
    saved_loaded: bool,
    /// F3-5-3: as conversas salvas a listar (montadas pela view a partir da projeção + perfis).
    saved_rows: Vec<SavedSessionRow>,
    /// F3-5-3: scroll da lista de conversas (a viewport rola por dentro, como a galeria de papéis).
    saved_scroll: ScrollHandle,
}

impl AgentModal {
    /// Modal em modo CRIAR. `existing` = nomes vivos do Espaço (dedup).
    #[must_use]
    pub fn new_create(suggester: Arc<dyn RoleSuggester>, existing: Vec<String>) -> Self {
        Self {
            mode: ModalMode::Create,
            name: String::new(),
            typing_ticks: None,
            suggestion: None,
            role: None,
            role_pinned: false,
            engines: EngineScan::Scanning,
            selected_engine: 0,
            original_profile_id: None,
            engine_touched: false,
            cwd: None,
            original_cwd: None,
            restart_now: false,
            kit_consent: false,
            advanced: false,
            command: None,
            focus: FocusField::Name,
            error: None,
            discard_armed: false,
            why_open: false,
            no_match: false,
            existing,
            gallery: None,
            suggester,
            original: None,
            // FIX-3: nasce no DEFAULT do produto (decisão do fundador — assistido). O caller
            // (main.rs) sobrepõe com o nível efetivo do Espaço via `set_autonomy`.
            autonomy: Autonomy::Assisted,
            // F3-0-6: casa não-tocada → o controle mostra o default neutro (Medium/equilibrado).
            effort: None,
            // F3-5-3: seção de conversas salvas fechada e ainda não carregada (lazy on-open).
            saved_open: false,
            saved_loaded: false,
            saved_rows: Vec::new(),
            saved_scroll: ScrollHandle::new(),
        }
    }

    /// Modal em modo EDITAR (M6-E): pré-preenchido com o estado atual do nó.
    /// `autonomy` = o nível EFETIVO do Agente hoje (fonte: workspace, até a costura por-agente).
    // Construtor que ESPELHA o estado do nó (nome/papel/autonomia/cwd/motor) — cada parâmetro é um
    // campo distinto pré-preenchido; agrupá-los num struct só p/ o lint seria cerimônia sem ganho.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new_edit(
        suggester: Arc<dyn RoleSuggester>,
        node: NodeId,
        name: &str,
        role_canon: Option<&str>,
        autonomy: Autonomy,
        cwd: Option<&Path>,
        current_profile_id: Option<&str>,
        existing: Vec<String>,
    ) -> Self {
        let mut m = Self::new_create(suggester, existing);
        m.mode = ModalMode::Edit { node };
        m.name = name.to_string();
        // ADR 0037: o campo "Pasta de trabalho" mostra o cwd REAL do nó (não o default leigo);
        // `original_cwd` é a referência para detectar a troca no Salvar.
        m.cwd = cwd.map(Path::to_path_buf);
        m.original_cwd = cwd.map(Path::to_path_buf);
        // ADR 0039: o motor ATUAL do nó — referência da troca + pré-seleção do chip quando a
        // descoberta chega (`set_engines`).
        m.original_profile_id = current_profile_id.map(str::to_string);
        m.role = role_canon.map(|r| {
            let (label, blurb) = humanize(r);
            RoleSuggestion {
                role: r.to_string(),
                label,
                blurb,
                why: "Papel atual deste Agente.".to_string(),
            }
        });
        // editar nunca re-sugere por cima do papel vigente sem novo nome (M6-E: pergunta antes).
        m.role_pinned = m.role.is_some();
        m.autonomy = autonomy;
        m.original = Some((name.to_string(), role_canon.map(str::to_string), autonomy));
        m
    }

    // ── leitura (render/testes) ──

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn suggestion(&self) -> Option<&RoleSuggestion> {
        self.suggestion.as_ref()
    }
    #[must_use]
    pub fn role(&self) -> Option<&RoleSuggestion> {
        self.role.as_ref()
    }
    #[must_use]
    pub fn engines(&self) -> &EngineScan {
        &self.engines
    }
    #[must_use]
    pub fn selected_engine(&self) -> Option<&Engine> {
        match &self.engines {
            EngineScan::Ready(v) => v.get(self.selected_engine),
            EngineScan::Scanning => None,
        }
    }
    #[must_use]
    pub fn selected_engine_idx(&self) -> usize {
        self.selected_engine
    }
    /// Aviso de aposentadoria do motor SELECIONADO (F1-1-9 critério 2): `Some` ⇒ o render
    /// mostra o banner leigo abaixo dos chips. É o ESTADO que a casca fia — testável headless.
    #[must_use]
    pub fn eol_notice(&self) -> Option<&'static str> {
        self.selected_engine()
            .and_then(|e| engine_transition_notice(&e.id))
    }
    #[must_use]
    pub fn autonomy(&self) -> Autonomy {
        self.autonomy
    }
    /// FIX-3: seleção da seção AUTONOMIA (clique numa opção) e prefill do nível efetivo do
    /// Espaço ao abrir (criar) — o default do produto continua sendo o do `new_create`.
    pub fn set_autonomy(&mut self, a: Autonomy) {
        self.autonomy = a;
    }
    /// F3-0-6: nível EFETIVO mostrado no controle nomeado. `None` (casa não-tocada) exibe o
    /// default do produto ([`Effort::Medium`]) como selecionado — honesto: é o que o novo Agente
    /// terá. A APLICAÇÃO no PTY é por-spawn (F3-0-4); aqui só se escolhe a preferência.
    #[must_use]
    pub fn effort(&self) -> Effort {
        self.effort.unwrap_or(Effort::Medium)
    }
    /// F3-0-6: seleção da seção RACIOCÍNIO (clique numa opção) — carimba a casa `effort` que o
    /// `create_plan` leva ao spawn.
    pub fn set_effort(&mut self, e: Effort) {
        self.effort = Some(e);
    }
    #[must_use]
    pub fn cwd_display(&self) -> String {
        self.cwd
            .as_ref()
            .map_or_else(|| COPY_CWD_DEFAULT.to_string(), |p| p.display().to_string())
    }
    #[must_use]
    pub fn advanced_open(&self) -> bool {
        self.advanced
    }
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    #[must_use]
    pub fn discard_armed(&self) -> bool {
        self.discard_armed
    }
    #[must_use]
    pub fn why_open(&self) -> bool {
        self.why_open
    }
    #[must_use]
    pub fn focus_field(&self) -> FocusField {
        self.focus
    }

    /// O comando EXIBIDO no Avançado: override do usuário, senão o do motor (profile TOML).
    #[must_use]
    pub fn effective_command(&self) -> String {
        if let Some(c) = &self.command {
            return c.clone();
        }
        match self.selected_engine() {
            Some(e) => {
                let mut s = e.program.clone();
                for a in &e.args {
                    s.push(' ');
                    s.push_str(a);
                }
                s
            }
            None => String::new(),
        }
    }

    // ── mutação (teclado/cliques) ──

    /// Caractere imprimível no campo focado. Digitar no Nome re-arma o debounce do co-piloto e
    /// desarma o descarte; digitar dispensa a linha-fantasma corrente sem fricção (M6-E2).
    pub fn type_char(&mut self, s: &str) {
        self.discard_armed = false;
        self.error = None;
        match self.focus {
            FocusField::Name => {
                self.name.push_str(s);
                self.typing_ticks = Some(0);
                self.suggestion = None;
            }
            FocusField::Command => {
                let mut c = self.effective_command();
                c.push_str(s);
                self.command = Some(c);
            }
        }
    }

    pub fn backspace(&mut self) {
        self.discard_armed = false;
        self.error = None;
        match self.focus {
            FocusField::Name => {
                self.name.pop();
                self.typing_ticks = Some(0);
                self.suggestion = None;
            }
            FocusField::Command => {
                let mut c = self.effective_command();
                c.pop();
                self.command = Some(c);
            }
        }
    }

    pub fn set_focus(&mut self, f: FocusField) {
        self.focus = f;
    }

    /// Um tick do loop do modal (~50ms). Devolve `true` se algo visível mudou (→ `cx.notify`).
    /// O co-piloto dispara na PAUSA (SUGGEST_DEBOUNCE_TICKS), com 3+ chars, e **re-sugere só em
    /// mudança substancial**: papel diferente do já aplicado/sugerido (não a cada tecla).
    pub fn tick(&mut self) -> bool {
        let Some(t) = self.typing_ticks else {
            return false;
        };
        if t + 1 < SUGGEST_DEBOUNCE_TICKS {
            self.typing_ticks = Some(t + 1);
            return false;
        }
        self.typing_ticks = None;
        let fresh = self.suggester.suggest(&self.name);
        // Fix de tela: rodou e NÃO reconheceu (nome com 3+ chars) → fallback honesto no campo.
        self.no_match = fresh.is_none() && self.name.trim().len() >= 3;
        match fresh {
            Some(s) => {
                // papel manual ([Trocar]) não é atropelado; igual ao aplicado = sem ghost (ruído).
                let same_as_applied = self.role.as_ref().is_some_and(|r| r.role == s.role);
                if same_as_applied || (self.role_pinned && self.role.is_some()) {
                    // M6-E: renomear COM papel pinado → a pergunta explícita fica visível na
                    // linha-fantasma mesmo assim quando o papel sugerido difere.
                    if self.role_pinned && self.role.as_ref().is_some_and(|r| r.role != s.role) {
                        self.suggestion = Some(s);
                        return true;
                    }
                    self.suggestion = None;
                    return true;
                }
                self.suggestion = Some(s);
                true
            }
            None => self.suggestion.take().is_some(),
        }
    }

    /// Aceita a sugestão (Tab/clique) — ÚNICO caminho que aplica papel sugerido (nunca silencioso).
    pub fn accept_suggestion(&mut self) -> bool {
        match self.suggestion.take() {
            Some(s) => {
                self.role = Some(s);
                self.role_pinned = false;
                self.why_open = false;
                self.no_match = false;
                true
            }
            None => false,
        }
    }

    /// O co-piloto rodou e não reconheceu papel no nome atual (fallback honesto no campo).
    #[must_use]
    pub fn copilot_no_match(&self) -> bool {
        self.no_match
    }

    /// [Trocar]: abre o SELETOR de papéis (F1-2-3 + feedback do fundador — todos os papéis de
    /// uma vez, com busca; nada de ciclar de um em um).
    pub fn open_gallery(&mut self) {
        self.gallery = Some(RoleGallery::from_library());
    }

    #[must_use]
    pub fn gallery(&self) -> Option<&RoleGallery> {
        self.gallery.as_ref()
    }

    #[must_use]
    pub fn gallery_open(&self) -> bool {
        self.gallery.is_some()
    }

    /// Esc do funil (main.rs fixo): com o editor leve aberto, fecha SÓ o editor (volta à
    /// lista); senão fecha a galeria. Dois Esc = fecha tudo (downgrade previsível).
    pub fn gallery_close(&mut self) {
        if let Some(g) = self.gallery.as_mut() {
            if g.editor().is_some() {
                g.editor_close();
                return;
            }
        }
        self.gallery = None;
    }

    // ── F1-2-3 p2: editor leve de papel custom (duplicar + modificar) ──

    /// 'Duplicar' no item `idx` da lista filtrada → abre o editor leve pré-preenchido.
    pub fn gallery_duplicate(&mut self, idx: usize) {
        if let Some(g) = self.gallery.as_mut() {
            g.duplicate(idx);
        }
    }

    /// Injeta os papéis CUSTOM do log (ordem de log; last-wins aplicado aqui no modelo).
    pub fn gallery_set_customs(&mut self, rows: Vec<RoleTemplate>) {
        if let Some(g) = self.gallery.as_mut() {
            g.set_customs(rows);
        }
    }

    pub fn gallery_editor_toggle_details(&mut self) {
        if let Some(ed) = self.gallery.as_mut().and_then(RoleGallery::editor_mut) {
            ed.details = !ed.details;
            // Fechar os detalhes com a Doutrina focada NÃO pode deixar o foco num campo
            // invisível — rebaixa pra Descrição.
            if !ed.details && ed.focus == EditorField::Doctrine {
                ed.focus = EditorField::Description;
            }
        }
    }

    /// Clique num campo do editor foca-o (a Doutrina só existe com detalhes abertos).
    pub fn gallery_editor_focus(&mut self, field: EditorField) {
        if let Some(ed) = self.gallery.as_mut().and_then(RoleGallery::editor_mut) {
            if field != EditorField::Doctrine || ed.details {
                ed.focus = field;
            }
        }
    }

    /// Fecha só o editor (botão [Voltar] / após salvar com sucesso).
    pub fn gallery_editor_cancel(&mut self) {
        if let Some(g) = self.gallery.as_mut() {
            g.editor_close();
        }
    }

    /// Erro leigo no editor (ex.: o seam de persistência falhou) — some ao digitar.
    pub fn gallery_editor_set_error(&mut self, msg: String) {
        if let Some(ed) = self.gallery.as_mut().and_then(RoleGallery::editor_mut) {
            ed.error = Some(msg);
        }
    }

    /// O plano de SALVAR do editor: valida o nome (vazio → erro leigo, nunca evento) e devolve
    /// o [`RoleTemplate`] exato (trim) que o seam `save_role_template` persistirá. Nenhum
    /// estado muda aqui — o handler fecha o editor SÓ depois do append confirmado.
    pub fn gallery_editor_plan(&self) -> Result<RoleTemplate, String> {
        let Some(ed) = self.gallery.as_ref().and_then(RoleGallery::editor) else {
            return Err(COPY_TPL_ERR_EMPTY_NAME.to_string());
        };
        let name = ed.name.trim();
        if name.is_empty() {
            return Err(COPY_TPL_ERR_EMPTY_NAME.to_string());
        }
        Ok(RoleTemplate {
            name: name.to_string(),
            description: ed.description.trim().to_string(),
            doctrine: ed.doctrine.trim().to_string(),
        })
    }

    pub fn gallery_type(&mut self, s: &str) {
        if let Some(g) = self.gallery.as_mut() {
            g.type_char(s);
        }
    }

    pub fn gallery_backspace(&mut self) {
        if let Some(g) = self.gallery.as_mut() {
            g.backspace();
        }
    }

    pub fn gallery_move(&mut self, delta: i32) {
        if let Some(g) = self.gallery.as_mut() {
            g.move_selection(delta);
        }
    }

    /// Aplica o papel escolhido (Enter / clique em `idx`) — PINA (escolha manual vence a
    /// re-sugestão, mesma regra do antigo Trocar) e fecha o seletor. `false` = nada escolhido.
    pub fn gallery_pick(&mut self, idx: Option<usize>) -> bool {
        let Some(g) = self.gallery.as_ref() else {
            return false;
        };
        let picked = match idx {
            Some(i) => g.pick_at(i),
            None => g.pick(),
        };
        match picked {
            Some(role) => {
                self.role = Some(role);
                self.role_pinned = true;
                self.suggestion = None;
                self.no_match = false;
                self.gallery = None;
                true
            }
            None => false,
        }
    }

    pub fn toggle_why(&mut self) {
        self.why_open = !self.why_open;
    }

    pub fn toggle_advanced(&mut self) {
        self.advanced = !self.advanced;
        if !self.advanced {
            self.focus = FocusField::Name;
        }
    }

    pub fn select_engine(&mut self, idx: usize) {
        if let EngineScan::Ready(v) = &self.engines {
            if idx < v.len() {
                self.selected_engine = idx;
                self.command = None; // o comando derivado segue o motor novo
                self.engine_touched = true; // ADR 0039: clique consciente — habilita a troca no Salvar
            }
        }
    }

    /// Resultado da descoberta off-thread (ou snapshot injetado em teste). No modo EDITAR,
    /// pré-seleciona o motor ATUAL do nó (ADR 0039) — senão o reset para 0 (1º da lista, claude)
    /// marcaria uma troca falsa para um nó que já é codex/gemini.
    pub fn set_engines(&mut self, engines: Vec<Engine>) {
        self.selected_engine = self
            .original_profile_id
            .as_deref()
            .and_then(|pid| {
                engines
                    .iter()
                    .position(|e| e.profile_id.as_deref() == Some(pid))
            })
            .unwrap_or(0);
        self.engines = EngineScan::Ready(engines);
    }

    pub fn set_cwd(&mut self, p: PathBuf) {
        self.cwd = Some(p);
    }

    /// ADR 0039: o motor foi trocado em relação ao do nó? (gate do aviso de reinício). Só conta
    /// se o usuário CLICOU num chip (`engine_touched`) E o profile selecionado difere do atual.
    #[must_use]
    pub fn engine_changed(&self) -> bool {
        self.engine_touched
            && self
                .build_engine()
                .is_some_and(|e| e.profile_id != self.original_profile_id)
    }

    /// ADR 0037: a Pasta de Trabalho foi trocada em relação ao estado ORIGINAL do nó? Gate do
    /// toggle "reiniciar agora" — ele só aparece (no Editar) quando há de fato pasta nova a aplicar.
    #[must_use]
    pub fn cwd_changed(&self) -> bool {
        self.cwd != self.original_cwd
    }

    /// ADR 0037: o usuário marcou "reiniciar este Agente na nova pasta agora"? (só efetivo com
    /// a pasta trocada — ver `save_plan`).
    #[must_use]
    pub fn restart_now(&self) -> bool {
        self.restart_now
    }

    /// ADR 0037: liga/desliga o "reiniciar agora" (clique na linha do modal de edição).
    pub fn toggle_restart_now(&mut self) {
        self.restart_now = !self.restart_now;
    }

    /// Rodada 360 (ADR 0022 §4): há pasta PRÓPRIA escolhida? (gate da linha de consentimento —
    /// a pasta gerenciada do Espaço sempre recebe o kit, sem pergunta).
    #[must_use]
    pub fn user_cwd_chosen(&self) -> bool {
        self.cwd.is_some()
    }

    /// Consentimento corrente do kit de integração (opt-in explícito; nasce desligado).
    #[must_use]
    pub fn kit_consent(&self) -> bool {
        self.kit_consent
    }

    /// Liga/desliga o consentimento do kit (clique na linha do modal).
    pub fn toggle_kit_consent(&mut self) {
        self.kit_consent = !self.kit_consent;
    }

    /// Esc: com conteúdo digitado, ARMA a confirmação («Descartar este Agente?»); sem conteúdo
    /// (ou já armado), devolve `true` = fechar agora. (Mapa de cliques #11.)
    pub fn escape(&mut self) -> bool {
        if self.discard_armed || self.name.trim().is_empty() {
            return true;
        }
        self.discard_armed = true;
        false
    }

    /// Monta o [`AgentEngine`] do motor SELECIONADO, respeitando o override de comando do
    /// Avançado. `None` se a descoberta ainda não terminou ou não há motor. Fonte ÚNICA da
    /// montagem do motor — `create_plan` (criar) e `save_plan` (trocar motor, ADR 0039) usam-na.
    #[must_use]
    fn build_engine(&self) -> Option<AgentEngine> {
        let engine = match &self.engines {
            EngineScan::Ready(v) if !v.is_empty() => {
                v[self.selected_engine.min(v.len() - 1)].clone()
            }
            _ => return None,
        };
        // Comando final: override do Avançado (split simples; modo técnico) ou o do profile.
        let (program, args) = match &self.command {
            Some(c) if !c.trim().is_empty() => {
                let mut it = c.split_whitespace().map(str::to_string);
                let program = it.next().unwrap_or_else(|| engine.program.clone());
                (program, it.collect())
            }
            _ => (engine.program.clone(), engine.args.clone()),
        };
        Some(AgentEngine {
            program,
            args,
            profile_id: engine.profile_id.clone(),
            label: engine.label.clone(),
        })
    }

    /// Valida e monta o plano de CRIAÇÃO (critérios 1/3/4/5): nome leigo obrigatório, motor
    /// detectado obrigatório (estado vazio aponta o caminho de instalar — nunca beco), dedup de
    /// nome com sufixo. **Nenhum evento aqui** — quem commita é o caller com o plano.
    pub fn create_plan(&mut self) -> Result<CreatePlan, String> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(COPY_ERR_EMPTY_NAME.to_string());
        }
        match &self.engines {
            EngineScan::Scanning => return Err(COPY_ERR_SCANNING.to_string()),
            EngineScan::Ready(v) if v.is_empty() => return Err(COPY_ERR_NO_ENGINE.to_string()),
            EngineScan::Ready(_) => {}
        }
        let engine = self
            .build_engine()
            .ok_or_else(|| COPY_ERR_NO_ENGINE.to_string())?;
        let unique = unique_name(&name, &self.existing);
        let dup_note = (unique != name).then(|| copy_dup_note(&unique));
        Ok(CreatePlan {
            name: unique,
            dup_note,
            engine,
            cwd: self.cwd.clone(),
            role: self.role.as_ref().map(|r| r.role.clone()),
            kit_consent: self.kit_consent,
            autonomy: self.autonomy,
            // F3-0-3 deu a CASA; F3-0-6 liga a SELEÇÃO: o nível escolhido no controle nomeado vai
            // ao spawn (vira `LINA_EFFORT` + `EffortAssigned` em F3-0-4). `model` segue do CLI (a
            // escolha de modelo não é desta story; o badge só LÊ o modelo correlacionado).
            model: None,
            effort: self.effort,
        })
    }

    /// Valida e monta o plano de SALVAR (M6-E): nome/papel valem agora; motor é F1-2-4.
    pub fn save_plan(&mut self) -> Result<SavePlan, String> {
        let ModalMode::Edit { node } = self.mode else {
            return Err("não está em modo edição".to_string());
        };
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(COPY_ERR_EMPTY_NAME.to_string());
        }
        // sem original (estado impossível em Edit) → trata tudo como mudança, nunca panica.
        let (orig_name, orig_role, orig_autonomy) =
            self.original
                .clone()
                .unwrap_or((String::new(), None, self.autonomy));
        let role_now = self.role.as_ref().map(|r| r.role.clone());
        // dedup ignora o PRÓPRIO nome original (renomear para si mesmo não ganha sufixo).
        let others: Vec<String> = self
            .existing
            .iter()
            .filter(|n| **n != orig_name)
            .cloned()
            .collect();
        // ADR 0037: a Pasta de Trabalho mudou? `cwd` só entra no plano quando DIFERE do cwd
        // original do nó (renomear para a mesma pasta é no-op). `restart_now` só vale se mudou.
        let cwd_changed = self.cwd != self.original_cwd;
        let cwd = cwd_changed.then(|| self.cwd.clone()).flatten();
        // ADR 0039: o MOTOR mudou? Só quando o usuário CLICOU num chip (engine_touched) E o
        // profile selecionado difere do atual do nó. O novo motor RE-ERGUE o Agente (restart) no
        // commit — trocar o cérebro de um terminal vivo é encerrá-lo e recriá-lo com o novo CLI.
        let engine = if self.engine_touched {
            self.build_engine()
                .filter(|e| e.profile_id != self.original_profile_id)
        } else {
            None
        };
        Ok(SavePlan {
            node,
            name: unique_name(&name, &others),
            role: (role_now != orig_role).then_some(role_now).flatten(),
            autonomy: (self.autonomy != orig_autonomy).then_some(self.autonomy),
            cwd,
            restart_now: cwd_changed && self.restart_now,
            engine,
        })
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
    }

    // ── F3-5-3: conversas salvas ("Continuar uma conversa") ──

    #[must_use]
    pub fn saved_open(&self) -> bool {
        self.saved_open
    }

    #[must_use]
    pub fn saved_loaded(&self) -> bool {
        self.saved_loaded
    }

    #[must_use]
    pub fn saved_rows(&self) -> &[SavedSessionRow] {
        &self.saved_rows
    }

    /// Handle do scroll da lista — o render amarra na viewport via `track_scroll` (como a galeria).
    #[must_use]
    pub fn saved_scroll(&self) -> &ScrollHandle {
        &self.saved_scroll
    }

    /// Abre/fecha a seção. O caller carrega as conversas na 1ª abertura (lazy) — ver
    /// [`WorkspaceView::modal_toggle_saved_sessions`].
    pub fn toggle_saved(&mut self) {
        self.saved_open = !self.saved_open;
    }

    /// Injeta as conversas salvas montadas pela view (projeção + perfis) e marca como carregadas.
    pub fn set_saved_sessions(&mut self, rows: Vec<SavedSessionRow>) {
        self.saved_rows = rows;
        self.saved_loaded = true;
    }
}

/// Dedup leigo: nome já existe → sufixo " (2)"/" (3)"… (tabela de erros: avisa, não bloqueia).
#[must_use]
fn unique_name(name: &str, existing: &[String]) -> String {
    let taken = |n: &str| existing.iter().any(|e| e.trim() == n);
    if !taken(name) {
        return name.to_string();
    }
    for i in 2..100 {
        let candidate = format!("{name} ({i})");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{name} (∞)")
}

// ═══════════════════════════ a casca gpui (render fina) ═══════════════════════════

/// Renderiza o modal sobre o canvas. Cliques roteiam por métodos do [`WorkspaceView`]
/// (`modal_*`) — o modelo continua gpui-free.
pub fn render(
    modal: &AgentModal,
    viewport: Size<Pixels>,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let th = theme::active();
    let is_edit = matches!(modal.mode, ModalMode::Edit { .. });
    // Contenção (fix bugB2): TODA dimensão vem da matemática pura — o render só aplica.
    let win = WinSize {
        w: f32::from(viewport.width),
        h: f32::from(viewport.height),
    };
    let frame = modal_frame(win);

    let mut col = div().flex().flex_col().gap_3();

    // ── título ──
    let title = if is_edit {
        format!("{COPY_TITLE_EDIT} — {}", modal.name())
    } else {
        COPY_TITLE_CREATE.to_string()
    };
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .child(
                // bugB3: o título cede (nome de Edit pode ser longo) — o ✕ nunca sai.
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_size(px(20.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(th.text.bright))
                    .child(text!(title)),
            )
            .child(
                // F2-2-1: consolidado no Button (ghost/icon) — ganha role(Button)+aria("Fechar"),
                // que o ✕ inline não tinha (era invisível a leitor de tela).
                Button::new("m6-close", "✕")
                    .icon()
                    .ghost()
                    .aria("Fechar")
                    .no_shrink()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_cancel(cx);
                    })),
            ),
    );

    // ── F3-5-3: "Continuar uma conversa" (só no CRIAR; doc-fonte 67) ──
    // Atalho leigo para retomar uma conversa salva sem digitar nada técnico. Colapsada por default
    // (o caminho primário é criar do zero); expandir CARREGA as conversas (lazy, só no gesto —
    // `modal_toggle_saved_sessions`), então o modal abre instantâneo e nada lê o log sem o clique.
    if !is_edit {
        let mut sec = div().flex().flex_col().gap_2();
        sec = sec.child(
            div()
                .id("m6-resume-toggle")
                .flex()
                .flex_row()
                .items_center()
                .px_3()
                .py_2()
                .rounded_chrome()
                .bg(rgb(th.surface.panel))
                .text_color(rgb(th.text.primary))
                .cursor_pointer()
                .aria_label("continuar uma conversa salva")
                .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                    v.modal_toggle_saved_sessions(cx);
                }))
                .child(text!(if modal.saved_open() {
                    COPY_RESUME_HEADER_OPEN
                } else {
                    COPY_RESUME_HEADER_CLOSED
                })),
        );
        if modal.saved_open() {
            sec = sec.child(
                div()
                    .px_3()
                    .text_color(rgb(th.text.muted))
                    .child(text!(COPY_RESUME_HINT)),
            );
            let rows = modal.saved_rows();
            if modal.saved_loaded() && rows.is_empty() {
                // Estado vazio HONESTO: explica QUANDO conversas aparecem (após o 1º trabalho).
                sec = sec.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(rgb(th.text.muted))
                        .child(text!(COPY_RESUME_EMPTY)),
                );
            } else {
                // Viewport que rola por dentro (mesma doutrina da galeria): a lista nunca estoura
                // o modal; o teto é const NOMEADA → fora da catraca de tokens.
                let mut list = div()
                    .id("m6-resume-list")
                    .aria_label(format!("conversas salvas, {} para continuar", rows.len()))
                    .track_scroll(modal.saved_scroll())
                    .overflow_y_scroll()
                    .max_h(px(RESUME_VIEWPORT_MAX))
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_1();
                for (i, r) in rows.iter().enumerate() {
                    let name_color = if r.resumable {
                        th.text.bright
                    } else {
                        th.text.muted
                    };
                    // Resumível → idade ("ontem"); indisponível → aviso honesto (motor sumiu).
                    let meta = if r.resumable {
                        r.age.clone()
                    } else {
                        COPY_RESUME_UNAVAILABLE.to_string()
                    };
                    let mut row = div()
                        .id(("m6-resume-item", i))
                        .aria_label(format!("continuar a conversa {} · {meta}", r.display_name))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded_content()
                        .bg(rgb(th.surface.raised));
                    // Só linha resumível é clicável — a inerte não promete o que não cumpre.
                    if r.resumable {
                        row = row.cursor_pointer().on_click(cx.listener(
                            move |v, _ev: &ClickEvent, _w, cx| {
                                v.modal_resume_saved_session(i, cx);
                            },
                        ));
                    }
                    row = row
                        .child(
                            // Doutrina B3: o nome CEDE (min_w 0 + clip); a idade é flex_none.
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .font_weight(FontWeight(f32::from(th.typography.weight.semibold)))
                                .text_color(rgb(name_color))
                                .child(text!(r.display_name.clone())),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(rgb(th.text.muted))
                                .child(text!(meta)),
                        );
                    list = list.child(row);
                }
                sec = sec.child(list);
            }
        }
        col = col.child(sec);
    }

    // ── quick-start de motores (M6-E1) ──
    match modal.engines() {
        EngineScan::Scanning => {
            col = col.child(
                div()
                    .text_color(rgb(th.text.muted))
                    .child(text!(COPY_SCANNING)),
            );
        }
        EngineScan::Ready(v) if v.is_empty() => {
            // Estado vazio (critério 4): caminho ÚNICO de instalar — nunca quick-start vazio.
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .rounded_chrome()
                    .bg(rgb(th.surface.panel))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(th.text.primary))
                            .child(text!(COPY_EMPTY_TITLE)),
                    )
                    .child(
                        div()
                            .id("m6-install-reco")
                            .px_4()
                            .py_2()
                            .rounded_content()
                            .bg(rgb(th.accent.action))
                            .text_color(rgb(th.text.on_accent))
                            .cursor_pointer()
                            .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                                v.modal_install_engine(cx);
                            }))
                            .child(text!(COPY_EMPTY_CTA)),
                    ),
            );
        }
        EngineScan::Ready(v) => {
            col = col.child(
                div()
                    .text_color(rgb(th.text.muted))
                    .child(text!(COPY_QUICKSTART)),
            );
            let mut chips = div().flex().flex_row().flex_wrap().items_center().gap_2();
            for (i, e) in v.iter().enumerate() {
                let active = i == modal.selected_engine_idx();
                let mark = if active { "◉" } else { "○" };
                let reco = if e.id == "claude" {
                    " «recomendado»"
                } else {
                    ""
                };
                let mut chip = div()
                    .id(("m6-engine", i))
                    .px_3()
                    .py_2()
                    .rounded_content()
                    .bg(rgb(if active {
                        th.surface.raised_alt
                    } else {
                        th.surface.raised
                    }))
                    .text_color(rgb(th.text.bright))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _ev: &ClickEvent, _w, cx| {
                        view.modal_select_engine(i, cx);
                    }))
                    .child(text!(format!("{mark} {}{reco}", e.label)));
                if active {
                    chip = chip.border_2().border_color(rgb(th.focus.ring));
                }
                chips = chips.child(chip);
            }
            chips = chips.child(
                div()
                    .id("m6-install-other")
                    .px_3()
                    .py_2()
                    .rounded_content()
                    .bg(rgb(th.surface.raised))
                    .text_color(rgb(th.text.primary))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_install_engine(cx);
                    }))
                    .child(text!(COPY_INSTALL_OTHER)),
            );
            col = col.child(chips);
            // F1-1-9 (render no gate de saída F1): motor selecionado em aposentadoria →
            // banner leigo de transição. Informa, NUNCA bloqueia (a criação segue livre).
            if let Some(notice) = modal.eol_notice() {
                col = col.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded_content()
                        .border_1()
                        .border_color(rgb(th.state.warning))
                        .text_color(rgb(th.state.warning))
                        .child(text!(format!("⚠ {notice}"))),
                );
            }
        }
    }

    // ADR 0039: trocar o motor de um Agente vivo o reinicia com o novo CLI. O aviso só aparece
    // quando o motor foi DE FATO trocado (não polui quem só abriu o Editar) — nunca silencioso.
    if is_edit && modal.engine_changed() {
        col = col.child(
            div()
                .text_color(rgb(th.state.warning))
                .child(text!(COPY_EDIT_ENGINE_RESTART_WARN)),
        );
    }

    // ── Nome (foco do teclado; dispara o co-piloto) ──
    let (typed, typed_fg) = if modal.name().is_empty() {
        (COPY_NAME_PLACEHOLDER.to_string(), th.text.muted)
    } else {
        (format!("{}▌", modal.name()), th.text.bright)
    };
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ROW_GAP))
            .child(
                div()
                    .flex_none()
                    .w(px(ROW_LABEL_W))
                    .text_color(rgb(th.text.primary))
                    .child(text!(COPY_NAME_LABEL)),
            )
            .child(
                div()
                    .id("m6-name")
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .px_3()
                    .py_2()
                    .rounded_content()
                    .bg(rgb(th.surface.chrome))
                    .border_1()
                    .border_color(rgb(if modal.focus_field() == FocusField::Name {
                        th.focus.ring
                    } else {
                        th.surface.border
                    }))
                    .text_color(rgb(typed_fg))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_focus(FocusField::Name, cx);
                    }))
                    .child(text!(typed)),
            ),
    );

    // ── linha-fantasma do co-piloto (M6-E2) + por quê ──
    if let Some(s) = modal.suggestion() {
        let mut ghost = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                // bugB3: o ghost é a célula que CEDE (clicável mesmo clipado); o ⓘ por quê?
                // é a ação flex_none — nunca sai da linha.
                div()
                    .id("m6-ghost")
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_color(rgb(th.accent.primary))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_accept_suggestion(cx);
                    }))
                    .child(text!(copy_ghost(&s.label))),
            )
            .child(
                div()
                    .id("m6-why")
                    .flex_none()
                    .text_color(rgb(th.text.muted))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_toggle_why(cx);
                    }))
                    .child(text!(COPY_WHY_PREFIX)),
            );
        if modal.why_open() {
            ghost = ghost.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_color(rgb(th.text.secondary))
                    .child(text!(s.why.clone())),
            );
        }
        col = col.child(ghost);
    }

    // ── cartão de Papel — M6 manda CARTÃO humano (ícone + nome + 1 frase), nunca textarea/
    //    YAML; a edição de instruções é o "Ver e ajustar" da F1-2-3. Fix de tela: 4 estados
    //    explícitos, todos com cara do que são — (a) APLICADO: read-only deliberado (sem cursor
    //    de clique); (b) PREVIEW da sugestão VIVA no campo enquanto digita (clicável: aceita);
    //    (c) co-piloto rodou e não reconheceu → fallback HONESTO (nunca silêncio); (d) vazio:
    //    copy que diz onde digitar (o nome, acima) e como escolher (Trocar).
    let role_card: AnyElement = match (modal.role(), modal.suggestion()) {
        (Some(r), _) => div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded_content()
            .bg(rgb(th.surface.panel))
            .border_1()
            .border_color(rgb(th.surface.border))
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(th.text.primary))
                    .child(text!(format!("✎ {}", r.label))),
            )
            .child(
                div()
                    .text_color(rgb(th.text.secondary))
                    .child(text!(r.blurb.clone())),
            )
            .into_any_element(),
        (None, Some(s)) => div()
            .id("m6-role-preview")
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded_content()
            .bg(rgb(th.surface.panel))
            .border_1()
            .border_color(rgb(th.accent.primary))
            .cursor_pointer()
            .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                v.modal_accept_suggestion(cx);
            }))
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(th.accent.primary))
                    .child(text!(format!("✨ {}", s.label))),
            )
            .child(
                div()
                    .text_color(rgb(th.text.secondary))
                    .child(text!(s.blurb.clone())),
            )
            .child(
                div()
                    .text_color(rgb(th.text.muted))
                    .child(text!(COPY_ROLE_SUGGESTED_HINT)),
            )
            .into_any_element(),
        (None, None) if modal.copilot_no_match() => div()
            .px_3()
            .py_2()
            .rounded_content()
            .bg(rgb(th.surface.panel))
            .text_color(rgb(th.text.secondary))
            .child(text!(COPY_ROLE_NO_MATCH))
            .into_any_element(),
        _ => div()
            .px_3()
            .py_2()
            .rounded_content()
            .bg(rgb(th.surface.panel))
            .text_color(rgb(th.text.muted))
            .child(text!(COPY_ROLE_PLACEHOLDER))
            .into_any_element(),
    };
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(ROW_GAP))
            .child(
                div()
                    .flex_none()
                    .w(px(ROW_LABEL_W))
                    .text_color(rgb(th.text.primary))
                    .child(text!(COPY_ROLE_LABEL)),
            )
            .child(
                // bugB3 (A regressão): a célula do cartão é quem CEDE — min_w(0) mata o
                // min-content de linha-única do texto; overflow_hidden é o cinto.
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(role_card),
            )
            .child(
                // O [Trocar] é INEGOCIÁVEL: sem ele a galeria fica inalcançável.
                div()
                    .id("m6-role-swap")
                    .flex_none()
                    .px_3()
                    .py_2()
                    .rounded_content()
                    .bg(rgb(th.surface.raised))
                    .text_color(rgb(th.text.bright))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        // F1-2-3 p2: abre já com os papéis CUSTOM do log (replay → reaparecem).
                        v.modal_open_gallery_with_customs(cx);
                    }))
                    .child(text!(COPY_ROLE_SWAP)),
            ),
    );

    // ── seletor de papéis aberto (F1-2-3): TODOS os papéis + busca + teclado ──
    // Fix bugB2: o dropdown CONTIDO — largura = útil do modal, busca FIXA no topo e lista numa
    // VIEWPORT de altura calculada (matemática pura `gallery_layout`) que rola por dentro.
    // F1-2-3 p2: com o editor leve aberto ('Duplicar'), ele SUBSTITUI busca+lista no dropdown.
    if let Some(g) = modal.gallery() {
        if let Some(ed) = g.editor() {
            col = col.child(render_template_editor(ed, frame, cx));
        } else {
            let filtered = g.filtered();
            let sel = g.selected();
            let lay = gallery_layout(frame, win, filtered.len());
            let (qtxt, qfg) = if g.query().is_empty() {
                (COPY_GALLERY_SEARCH_PLACEHOLDER.to_string(), th.text.muted)
            } else {
                (format!("{}▌", g.query()), th.text.bright)
            };
            // Viewport da lista: raiz de scroll em BLOCO (lição W6: flex no root de scroll não
            // engata) com altura TRAVADA pela conta; as linhas são filhas DIRETAS — é o que o
            // `scroll_to_item` do teclado indexa (child_bounds do container trackeado).
            let mut list = div()
                .id("m6-gallery-list")
                // a11y: anuncia o tamanho real da lista e, quando a viewport corta, que rola.
                .aria_label(if lay.scrolls {
                    format!("lista de papéis, {} resultados — rola", filtered.len())
                } else {
                    format!("lista de papéis, {} resultados", filtered.len())
                })
                .track_scroll(g.scroll_handle())
                .overflow_y_scroll()
                .h(px(lay.list_viewport_h))
                .w_full();
            if filtered.is_empty() {
                list = list.child(
                    div()
                        .h(px(GALLERY_ROW_H))
                        .px_3()
                        .py_2()
                        .overflow_hidden()
                        .line_height(px(GALLERY_LINE_H))
                        .text_color(rgb(th.text.muted))
                        .child(text!(COPY_GALLERY_EMPTY)),
                );
            }
            for (i, r) in filtered.iter().enumerate() {
                let active = i == sel;
                // Geometria UNIFORME por linha: altura fixa (== GALLERY_ROW_H da conta), borda
                // sempre presente (transparente na inativa) — navegar não reflui o layout, e a
                // matemática do viewport nunca mente. a11y: ElementId ÚNICO por linha (lição:
                // text! em loop colide nó AccessKit) + rótulo anunciável completo. Doutrina B3:
                // a célula de texto CEDE (min_w 0 + clip); o [Duplicar] é flex_none — nunca sai.
                let mut label_line = div().flex().flex_row().items_center().gap_2().child(
                    div()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .line_height(px(GALLERY_LINE_H))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(if active {
                            th.text.bright
                        } else {
                            th.text.primary
                        }))
                        .child(text!(r.suggestion.label.clone())),
                );
                if r.custom {
                    // Badge do papel salvo pelo usuário — F1-2-3 p2 (reapareceu do log).
                    label_line = label_line.child(
                        div()
                            .flex_none()
                            .line_height(px(GALLERY_LINE_H))
                            .text_color(rgb(th.text.muted))
                            .child(text!(format!("· {COPY_TPL_BADGE}"))),
                    );
                }
                let row = div()
                    .id(("m6-role-item", i))
                    .aria_label(if r.custom {
                        format!(
                            "papel {} ({COPY_TPL_BADGE}) — {}",
                            r.suggestion.label, r.suggestion.blurb
                        )
                    } else {
                        format!("papel {} — {}", r.suggestion.label, r.suggestion.blurb)
                    })
                    .h(px(GALLERY_ROW_H))
                    .mb(px(GALLERY_ROW_GAP))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_content()
                    .overflow_hidden()
                    .bg(rgb(if active {
                        th.surface.raised_alt
                    } else {
                        th.surface.panel
                    }))
                    .border_2()
                    .border_color(if active {
                        // foco visível: o MESMO token do design system (≥3:1 nos 2 temas — F1-2-1).
                        rgb(th.focus.ring).into()
                    } else {
                        transparent_black()
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |v, _ev: &ClickEvent, _w, cx| {
                        v.modal_gallery_pick(Some(i), cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .child(label_line)
                            .child(
                                div()
                                    .line_height(px(GALLERY_LINE_H))
                                    .text_color(rgb(th.text.secondary))
                                    .child(text!(r.suggestion.blurb.clone())),
                            ),
                    )
                    .child(
                        // F1-2-3 p2: a porta do "duplicar + modificar" — ação INEGOCIÁVEL da linha.
                        div()
                            .id(("m6-role-dup", i))
                            .flex_none()
                            .px_2()
                            .py_1()
                            .rounded_content()
                            .bg(rgb(th.surface.raised))
                            .line_height(px(GALLERY_LINE_H))
                            .text_color(rgb(th.text.bright))
                            .cursor_pointer()
                            .on_click(cx.listener(move |v, _ev: &ClickEvent, _w, cx| {
                                // o clique no Duplicar NÃO escolhe a linha (papel só por gesto
                                // explícito de escolha — critério 4).
                                cx.stop_propagation();
                                v.modal_gallery_duplicate(i, cx);
                            }))
                            .child(text!(COPY_GALLERY_DUPLICATE)),
                    );
                list = list.child(row);
            }
            col = col.child(
                div()
                    // bounds FINAIS da conta: largura útil do modal × chrome+viewport — pinados.
                    .w(px(lay.w))
                    .h(px(lay.h))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_3()
                    .py_3()
                    .rounded_chrome()
                    .bg(rgb(th.surface.chrome))
                    .border_1()
                    .border_color(rgb(th.surface.raised_alt))
                    .child(
                        div()
                            .h(px(GALLERY_HEADER_H))
                            .flex()
                            .flex_row()
                            .items_center()
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .line_height(px(GALLERY_HEADER_H))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(th.text.primary))
                                    .child(text!(COPY_GALLERY_TITLE)),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .line_height(px(GALLERY_HEADER_H))
                                    .text_color(rgb(th.text.muted))
                                    .child(text!(COPY_GALLERY_HINT)),
                            ),
                    )
                    .child(
                        div()
                            .id("m6-gallery-search")
                            .h(px(GALLERY_SEARCH_H))
                            .px_3()
                            .py_2()
                            .overflow_hidden()
                            .line_height(px(GALLERY_LINE_H))
                            .rounded_content()
                            .bg(rgb(th.surface.panel))
                            .text_color(rgb(qfg))
                            .child(text!(format!("🔎 {qtxt}"))),
                    )
                    .child(list),
            );
        }
    }

    // ── Pasta de trabalho ──
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ROW_GAP))
            .child(
                div()
                    .flex_none()
                    .w(px(ROW_LABEL_W))
                    .text_color(rgb(th.text.primary))
                    .child(text!(COPY_CWD_LABEL)),
            )
            .child(
                // bugB3: caminho de pasta pode ser longo — cede; o [Trocar…] nunca sai.
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_color(rgb(th.text.secondary))
                    .child(text!(modal.cwd_display())),
            )
            .child(
                div()
                    .id("m6-cwd-pick")
                    .flex_none()
                    .px_3()
                    .py_2()
                    .rounded_content()
                    .bg(rgb(th.surface.raised))
                    .text_color(rgb(th.text.bright))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_pick_folder(cx);
                    }))
                    .child(text!(COPY_CWD_PICK)),
            ),
    );

    // ── Consentimento do kit (ADR 0022 §4) — SÓ com pasta própria escolhida; a pasta
    //    gerenciada do Espaço sempre recebe o kit, sem pergunta. Opt-in explícito: a Lina
    //    nunca escreve na pasta do usuário sem este toque; recusado → o card avisa.
    if !is_edit && modal.user_cwd_chosen() {
        let on = modal.kit_consent();
        col = col.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .id("m6-kit-consent")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .text_color(rgb(th.text.primary))
                        .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                            v.modal_toggle_kit_consent(cx);
                        }))
                        .child(text!(if on { "☑" } else { "☐" }))
                        .child(text!(COPY_CONSENT_LABEL)),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(if on { th.text.muted } else { th.state.warning }))
                        .child(text!(if on {
                            COPY_CONSENT_DETAIL
                        } else {
                            COPY_CONSENT_OFF_WARN
                        })),
                ),
        );
    }

    // ── ADR 0037: "reiniciar agora na nova pasta" — SÓ no Editar e SÓ quando a pasta MUDOU.
    //    Default DESLIGADO: trocar a pasta já persiste (vale na próxima abertura); ligar =
    //    re-erguer o Agente já na pasta nova (encerra o que está rodando — opt-in explícito).
    if is_edit && modal.cwd_changed() {
        let on = modal.restart_now();
        col = col.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .id("m6-restart-now")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .text_color(rgb(th.text.primary))
                        .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                            v.modal_toggle_restart_now(cx);
                        }))
                        .child(text!(if on { "☑" } else { "☐" }))
                        .child(text!(COPY_RESTART_LABEL)),
                )
                .child(
                    div()
                        .text_size(px(f32::from(th.typography.size.small)))
                        .text_color(rgb(if on { th.state.warning } else { th.text.muted }))
                        .child(text!(if on {
                            COPY_RESTART_DETAIL_ON
                        } else {
                            COPY_RESTART_DETAIL_OFF
                        })),
                ),
        );
    }

    // ── Autonomia (FIX-3) — escolha CONSCIENTE do nível, criar E editar. 3 opções com 1 linha
    //    leiga cada (responde "por que o agente me pede sim/não?" — dogfooding do fundador).
    //    Geometria uniforme por opção (borda sempre presente, transparente na inativa — idioma
    //    da galeria de papéis: selecionar não reflui o layout).
    {
        let mut options = div().flex().flex_col().gap_1();
        for (i, a) in AUTONOMY_OPTIONS.into_iter().enumerate() {
            let active = a == modal.autonomy();
            let mark = if active { "◉" } else { "○" };
            // F2-2-1: o idioma do card selecionável (borda 2px sempre, anel focus.ring quando ativo,
            // fundo raised_alt↔panel — geometria uniforme, sem reflow) consolidado no Panel.card().
            // Ganha role(RadioButton) — a escolha de autonomia é 1-de-N e antes não anunciava o papel.
            options = options.child(
                Panel::card()
                    .id(("m6-autonomy", i))
                    .selected(active)
                    .pad(Space::Md, Space::Xs)
                    .gap(Space::Zero)
                    .role(Role::RadioButton)
                    .aria(format!(
                        "autonomia {} — {}",
                        autonomy_surface_label(a),
                        autonomy_desc(a)
                    ))
                    .on_click(cx.listener(move |v, _ev: &ClickEvent, _w, cx| {
                        v.modal_set_autonomy(a, cx);
                    }))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(if active {
                                th.text.bright
                            } else {
                                th.text.primary
                            }))
                            .child(text!(format!("{mark} {}", autonomy_surface_label(a)))),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_size(px(12.0))
                            .text_color(rgb(th.text.secondary))
                            .child(text!(autonomy_desc(a))),
                    ),
            );
        }
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(ROW_GAP))
                .child(
                    div()
                        .flex_none()
                        .w(px(ROW_LABEL_W))
                        .text_color(rgb(th.text.primary))
                        .child(text!(COPY_AUTONOMY_LABEL)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(options),
                ),
        );
    }

    // ── Raciocínio (F3-0-6) — controle NOMEADO do nível de esforço por Agente, só ao CRIAR (é o
    //    spawn que carimba `LINA_EFFORT`+`EffortAssigned`; em edição não há caminho de aplicação,
    //    então não pintamos um controle morto). Mesma geometria/idioma da Autonomia (radio 1-de-N).
    if matches!(modal.mode, ModalMode::Create) {
        let mut options = div().flex().flex_col().gap_1();
        for (i, e) in crate::dashboard::EFFORT_OPTIONS.into_iter().enumerate() {
            let active = e == modal.effort();
            let mark = if active { "◉" } else { "○" };
            options = options.child(
                Panel::card()
                    .id(("m6-effort", i))
                    .selected(active)
                    .pad(Space::Md, Space::Xs)
                    .gap(Space::Zero)
                    .role(Role::RadioButton)
                    .aria(format!(
                        "raciocínio {} — {}",
                        crate::dashboard::effort_surface_label(e),
                        crate::dashboard::effort_surface_help(e)
                    ))
                    .on_click(cx.listener(move |v, _ev: &ClickEvent, _w, cx| {
                        v.modal_set_effort(e, cx);
                    }))
                    .child(
                        div()
                            .font_weight(FontWeight(f32::from(th.typography.weight.bold)))
                            .text_color(rgb(if active {
                                th.text.bright
                            } else {
                                th.text.primary
                            }))
                            .child(text!(format!(
                                "{mark} {}",
                                crate::dashboard::effort_surface_label(e)
                            ))),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_size(px(f32::from(th.typography.size.body)))
                            .text_color(rgb(th.text.secondary))
                            .child(text!(crate::dashboard::effort_surface_help(e))),
                    ),
            );
        }
        // O nome do controle (pt-br leigo) vive ABAIXO das opções — explica o conceito e o custo
        // sem precisar da chave técnica (DES-4/inv#6).
        options = options.child(
            div()
                .text_size(px(f32::from(th.typography.size.body)))
                .text_color(rgb(th.text.muted))
                .child(text!(COPY_EFFORT_CAPTION)),
        );
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(ROW_GAP))
                .child(
                    div()
                        .flex_none()
                        .w(px(ROW_LABEL_W))
                        .text_color(rgb(th.text.primary))
                        .child(text!(COPY_EFFORT_LABEL)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(options),
                ),
        );
    }

    // ── ▸ Avançado (divulgação progressiva — o comando NUNCA é controle primário) ──
    col = col.child(
        div()
            .id("m6-advanced")
            .text_color(rgb(th.text.muted))
            .cursor_pointer()
            .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                v.modal_toggle_advanced(cx);
            }))
            .child(text!(if modal.advanced_open() {
                COPY_ADVANCED_OPEN
            } else {
                COPY_ADVANCED
            })),
    );
    if modal.advanced_open() {
        let cmd = modal.effective_command();
        col = col.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_color(rgb(th.text.muted))
                        .child(text!(COPY_CMD_LABEL)),
                )
                .child(
                    div()
                        .id("m6-cmd")
                        // bugB3: comando pode ser uma "palavra" longa — clipa dentro da caixa.
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .px_3()
                        .py_2()
                        .rounded_content()
                        .bg(rgb(th.surface.chrome))
                        .border_1()
                        .border_color(rgb(if modal.focus_field() == FocusField::Command {
                            th.focus.ring
                        } else {
                            th.surface.border
                        }))
                        .font_family(th.typography.family.mono)
                        .text_color(rgb(th.text.primary))
                        .cursor_pointer()
                        .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                            v.modal_focus(FocusField::Command, cx);
                        }))
                        .child(text!(format!("{cmd}▌"))),
                ),
        );
    }

    // ── erro inline (leigo) / descarte armado ──
    if let Some(e) = modal.error() {
        col = col.child(
            div()
                .text_color(rgb(th.state.danger))
                .child(text!(e.to_string())),
        );
    }
    if modal.discard_armed() {
        col = col.child(
            div()
                .text_color(rgb(th.state.warning))
                .child(text!(COPY_DISCARD_ARMED)),
        );
    }
    if is_edit {
        col = col.child(
            div()
                .text_color(rgb(th.text.muted))
                .child(text!(COPY_EDIT_APPLY_NOTE)),
        );
    }

    // ── rodapé: Cancelar · Criar/Salvar ──
    col = col.child(
        div()
            .flex()
            .flex_row()
            .justify_end()
            .gap_2()
            .child(
                // bugB3: o par de botões do rodapé é inegociável (flex_none nos dois → no_shrink).
                // F2-2-1: consolidado no Button (secondary/confirm) — ganha role+aria.
                Button::new("m6-cancel", COPY_CANCEL)
                    .secondary()
                    .no_shrink()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_cancel(cx);
                    })),
            )
            .child(
                Button::new("m6-create", if is_edit { COPY_SAVE } else { COPY_CREATE })
                    .confirm()
                    .no_shrink()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, window, cx| {
                        v.modal_commit(window, cx);
                    })),
            ),
    );

    // backdrop + caixa central, consolidados no Modal do catálogo (F2-2-1). Fix bugB2 preservado:
    // a caixa vem do `modal_frame` (clamp à janela nos 2 eixos), e o corpo é raiz de scroll em
    // BLOCO (o Modal faz `min_h(0)+overflow_y_scroll`). GANHA `role(Dialog)`+aria (o maior buraco
    // de a11y do modal — antes não anunciava como diálogo). A OCLUSÃO de mouse vem LIGADA por
    // default do wrapper (`ui::modal`, fix M2): o clique no véu transparente não vaza mais pro
    // terminal atrás. O véu VISUAL (`dim`) segue OFF — comportamento visual idêntico ao atual.
    Modal::new(
        "m6-modal",
        Frame {
            w: frame.w,
            max_h: frame.max_h,
        },
    )
    .aria(if is_edit {
        "Editar agente"
    } else {
        "Criar agente"
    })
    .body(col)
    .into_any_element()
}

/// Uma caixa de CAMPO do editor leve (idioma do `#m6-name`): clique foca, caret ▌ no focado,
/// borda = focus ring do design system. Doutrina B3: `min_w(0)` + clip — o texto cede.
fn editor_field_box(
    id: &'static str,
    aria: String,
    value: &str,
    focused: bool,
    target: EditorField,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let th = theme::active();
    div()
        .id(id)
        .aria_label(aria)
        .min_w(px(0.0))
        .overflow_hidden()
        .px_3()
        .py_2()
        .rounded_content()
        .bg(rgb(th.surface.panel))
        .border_1()
        .border_color(rgb(if focused {
            th.focus.ring
        } else {
            th.surface.border
        }))
        .line_height(px(GALLERY_LINE_H))
        .text_color(rgb(if value.is_empty() && !focused {
            th.text.muted
        } else {
            th.text.bright
        }))
        .cursor_pointer()
        .on_click(cx.listener(move |v, _ev: &ClickEvent, _w, cx| {
            v.modal_gallery_editor_focus(target, cx);
        }))
        .child(text!(format!(
            "{value}{}",
            if focused { "\u{258c}" } else { "" }
        )))
        .into_any_element()
}

/// **F1-2-3 p2 — o editor LEVE do "duplicar + modificar"** (casca gpui; estado no
/// [`TemplateEditor`], gpui-free). Substitui busca+lista DENTRO do dropdown: nome + "o que ele
/// faz" na frente; instruções completas atrás de "ver detalhes" (progressive disclosure —
/// NUNCA obrigatória). Sem altura pinada: o conteúdo é fixo e baixo; em janela degenerada o
/// scroll-fallback do corpo (`#m6-body`, bugB2) absorve. Doutrina B3 em toda linha texto+ação
/// (título cede ao hint; rodapé flex_none). Nada persiste até [Salvar papel].
fn render_template_editor(
    ed: &TemplateEditor,
    frame: ModalFrame,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let th = theme::active();
    let mut panel = div()
        .w(px(frame.w - 2.0 * MODAL_PAD_X))
        .flex()
        .flex_col()
        .gap_2()
        .px_3()
        .py_3()
        .rounded_chrome()
        .bg(rgb(th.surface.chrome))
        .border_1()
        .border_color(rgb(th.surface.raised_alt))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    // o título cede (origem pode ter nome longo); o hint nunca sai.
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .line_height(px(GALLERY_HEADER_H))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(th.text.primary))
                        .child(text!(format!("{COPY_TPL_TITLE} \u{2014} {}", ed.source))),
                )
                .child(
                    div()
                        .flex_none()
                        .line_height(px(GALLERY_HEADER_H))
                        .text_color(rgb(th.text.muted))
                        .child(text!(COPY_TPL_HINT)),
                ),
        )
        .child(
            div()
                .line_height(px(GALLERY_LINE_H))
                .text_color(rgb(th.text.primary))
                .child(text!(COPY_TPL_NAME_LABEL)),
        )
        .child(editor_field_box(
            "m6-tpl-name",
            format!("nome do papel: {}", ed.name),
            &ed.name,
            ed.focus == EditorField::Name,
            EditorField::Name,
            cx,
        ))
        .child(
            div()
                .line_height(px(GALLERY_LINE_H))
                .text_color(rgb(th.text.primary))
                .child(text!(COPY_TPL_DESC_LABEL)),
        )
        .child(editor_field_box(
            "m6-tpl-desc",
            format!("o que ele faz: {}", ed.description),
            &ed.description,
            ed.focus == EditorField::Description,
            EditorField::Description,
            cx,
        ))
        .child(
            // progressive disclosure: as instruções completas moram ATRÁS deste toggle.
            div()
                .id("m6-tpl-details")
                .line_height(px(GALLERY_LINE_H))
                .text_color(rgb(th.text.muted))
                .cursor_pointer()
                .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                    v.modal_gallery_editor_toggle_details(cx);
                }))
                .child(text!(if ed.details {
                    COPY_TPL_DETAILS_OPEN
                } else {
                    COPY_TPL_DETAILS
                })),
        );
    if ed.details {
        panel = panel
            .child(
                div()
                    .line_height(px(GALLERY_LINE_H))
                    .text_color(rgb(th.text.muted))
                    .child(text!(COPY_TPL_DOCTRINE_LABEL)),
            )
            .child(
                // Caixa das instruções: raiz de scroll em BLOCO (lição W6), altura fixa — o
                // texto longo rola POR DENTRO; clique foca (digitação vai pra cá).
                div()
                    .id("m6-tpl-doctrine")
                    .aria_label(format!("instruções completas: {}", ed.doctrine))
                    .h(px(4.0 * GALLERY_LINE_H + 16.0))
                    .overflow_y_scroll()
                    .min_w(px(0.0))
                    .px_3()
                    .py_2()
                    .rounded_content()
                    .bg(rgb(th.surface.panel))
                    .border_1()
                    .border_color(rgb(if ed.focus == EditorField::Doctrine {
                        th.focus.ring
                    } else {
                        th.surface.border
                    }))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_gallery_editor_focus(EditorField::Doctrine, cx);
                    }))
                    .child(
                        div()
                            .line_height(px(GALLERY_LINE_H))
                            .text_color(rgb(
                                if ed.doctrine.is_empty() && ed.focus != EditorField::Doctrine {
                                    th.text.muted
                                } else {
                                    th.text.bright
                                },
                            ))
                            .child(text!(format!(
                                "{}{}",
                                ed.doctrine,
                                if ed.focus == EditorField::Doctrine {
                                    "\u{258c}"
                                } else {
                                    ""
                                }
                            ))),
                    ),
            );
    }
    if let Some(err) = &ed.error {
        panel = panel.child(
            div()
                .line_height(px(GALLERY_LINE_H))
                .text_color(rgb(th.state.danger))
                .child(text!(err.clone())),
        );
    }
    panel
        .child(
            // rodapé do editor: o par de ações é INEGOCIÁVEL (doutrina B3 — flex_none).
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(
                    div()
                        .id("m6-tpl-back")
                        .flex_none()
                        .px_4()
                        .py_2()
                        .rounded_content()
                        .bg(rgb(th.surface.raised))
                        .line_height(px(GALLERY_LINE_H))
                        .text_color(rgb(th.text.bright))
                        .cursor_pointer()
                        .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                            v.modal_gallery_editor_back(cx);
                        }))
                        .child(text!(COPY_TPL_BACK)),
                )
                .child(
                    div()
                        .id("m6-tpl-save")
                        .flex_none()
                        .px_4()
                        .py_2()
                        .rounded_content()
                        .bg(rgb(th.accent.confirm))
                        .line_height(px(GALLERY_LINE_H))
                        .text_color(rgb(th.text.on_accent))
                        .font_weight(FontWeight::BOLD)
                        .cursor_pointer()
                        .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                            v.modal_gallery_editor_save(cx);
                        }))
                        .child(text!(COPY_TPL_SAVE)),
                ),
        )
        .into_any_element()
}

// ═══════════ F1-2-3 p2 · handlers do editor (UI deste módulo → seam do bridge) ═══════════
// Moram AQUI, e não no main.rs (costura congelada nesta rodada): `impl` de tipo do próprio
// crate vale em qualquer módulo, e os itens privados do root (campos `agent_modal`/`nodes`,
// método `modal_open_gallery`) são visíveis em módulos descendentes. A ESCRITA core←app passa
// pelo seam do bridge (NodeManager::save_role_template / role_templates — dono LLM Engineer):
// porta ÚNICA de escrita preserva o inv#4; o last-wins fica no modelo (testável headless).
impl WorkspaceView {
    /// [Trocar] (F1-2-3 p2): abre a galeria pelo caminho original e injeta os papéis CUSTOM
    /// do log — o critério "reaparece nos próximos modais" nasce aqui.
    fn modal_open_gallery_with_customs(&mut self, cx: &mut Context<Self>) {
        self.modal_open_gallery(cx); // o caminho original (main.rs) segue vivo.
        let rows = self.nodes.role_templates();
        if let Some(m) = self.agent_modal.as_mut() {
            m.gallery_set_customs(rows);
        }
        cx.notify();
    }

    fn modal_gallery_duplicate(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.gallery_duplicate(idx);
            cx.notify();
        }
    }

    fn modal_gallery_editor_focus(&mut self, field: EditorField, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.gallery_editor_focus(field);
            cx.notify();
        }
    }

    fn modal_gallery_editor_toggle_details(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.gallery_editor_toggle_details();
            cx.notify();
        }
    }

    fn modal_gallery_editor_back(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.gallery_editor_cancel();
            cx.notify();
        }
    }

    /// [Salvar papel]: valida (nome vazio → erro leigo, sem evento) → APPEND pelo seam →
    /// refresh dos customs + volta à lista. O editor só fecha DEPOIS do append confirmado;
    /// falha → erro leigo no editor (o detalhe técnico já foi pro stderr no bridge).
    fn modal_gallery_editor_save(&mut self, cx: &mut Context<Self>) {
        let plan = match self
            .agent_modal
            .as_ref()
            .map(AgentModal::gallery_editor_plan)
        {
            Some(Ok(p)) => p,
            Some(Err(msg)) => {
                if let Some(m) = self.agent_modal.as_mut() {
                    m.gallery_editor_set_error(msg);
                }
                cx.notify();
                return;
            }
            None => return,
        };
        match self
            .nodes
            .save_role_template(&plan.name, &plan.description, &plan.doctrine)
        {
            Ok(()) => {
                let rows = self.nodes.role_templates();
                if let Some(m) = self.agent_modal.as_mut() {
                    m.gallery_editor_cancel();
                    m.gallery_set_customs(rows);
                }
            }
            Err(e) => {
                eprintln!("lina-gpui: F1-2-3p2 — salvar papel falhou: {e}");
                if let Some(m) = self.agent_modal.as_mut() {
                    m.gallery_editor_set_error(COPY_TPL_ERR_SAVE.to_string());
                }
            }
        }
        cx.notify();
    }

    /// **F3-5-3 — abre/fecha "Continuar uma conversa"; CARREGA na 1ª abertura (lazy).** A leitura
    /// (projeção do log + perfis do disco) roda só no GESTO de abrir, nunca por frame — o modal abre
    /// instantâneo. Reabrir usa o cache (o modal é efêmero; não vale reler o log a cada toggle).
    fn modal_toggle_saved_sessions(&mut self, cx: &mut Context<Self>) {
        let load_now = self
            .agent_modal
            .as_ref()
            .is_some_and(|m| !m.saved_open() && !m.saved_loaded());
        if let Some(m) = self.agent_modal.as_mut() {
            m.toggle_saved();
        }
        if load_now {
            let rows = self.build_saved_session_rows();
            if let Some(m) = self.agent_modal.as_mut() {
                m.set_saved_sessions(rows);
            }
        }
        cx.notify();
    }

    /// Monta as linhas da seção: a projeção das conversas salvas (bridge) + os CLI Profiles do disco
    /// (MESMA fonte do quick-start), resolvendo o comando de religação de cada uma com o verbo de
    /// resume do motor + o `session_id` exato (`lina_core::resume_session::resume_command`). Sem
    /// motor resumível → linha inerte (`resumable=false`), nunca um comando que abriria do zero.
    fn build_saved_session_rows(&self) -> Vec<SavedSessionRow> {
        let sessions = self.nodes.saved_sessions();
        let profiles = load_profiles(&profiles_dir(self.nodes.lina_home()));
        let now = lina_core::now_ms();
        sessions
            .into_iter()
            .map(|s| {
                let profile = profiles.get(&s.cli);
                let resumable = profile.is_some_and(CliProfile::can_resume);
                let command = if resumable {
                    profile.map_or_else(Vec::new, |p| {
                        lina_core::resume_session::resume_command(p, Some(&s.session_id))
                    })
                } else {
                    Vec::new()
                };
                SavedSessionRow {
                    display_name: s.display_name,
                    age: humanize_age(now, s.persisted_at_ms),
                    command,
                    profile_id: Some(s.cli),
                    role: s.role,
                    cwd: s.cwd,
                    resumable,
                }
            })
            .collect()
    }

    /// **F3-5-3 — o clique numa conversa → retoma pelo funil único (`admit_node`, sem bypass).**
    /// Sucesso fecha o modal (a conversa volta no canvas); falha mostra um aviso leigo inline (o
    /// detalhe técnico já foi pro stderr no bridge). Linha inerte (`!resumable`) é no-op defensivo.
    fn modal_resume_saved_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(row) = self
            .agent_modal
            .as_ref()
            .and_then(|m| m.saved_rows().get(idx).cloned())
        else {
            return;
        };
        if !row.resumable {
            return;
        }
        match self.nodes.resume_session(
            row.command,
            row.profile_id,
            row.display_name,
            row.role,
            row.cwd,
        ) {
            Ok(_) => {
                self.agent_modal = None;
            }
            Err(e) => {
                eprintln!("lina-gpui: F3-5-3 — retomar conversa falhou: {e}");
                if let Some(m) = self.agent_modal.as_mut() {
                    m.set_error(e);
                }
            }
        }
        cx.notify();
    }
}

// ═══════════════════════════ testes (gpui-free) ═══════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::role_suggester::W31Suggester;

    fn modal() -> AgentModal {
        AgentModal::new_create(Arc::new(W31Suggester), vec![])
    }

    fn type_str(m: &mut AgentModal, s: &str) {
        for c in s.chars() {
            m.type_char(&c.to_string());
        }
    }

    fn engines_one() -> Vec<Engine> {
        vec![Engine {
            id: "claude".into(),
            label: engine_label("claude"),
            version: Some("2.1".into()),
            program: "claude".into(),
            args: vec!["--flag".into()],
            profile_id: Some("claude-code".into()),
        }]
    }

    /// F1-1-9 critério 2 (gate de saída F1): selecionar o **Gemini** no quick-start liga o
    /// aviso leigo de aposentadoria (data 18/jun/2026, sucessor Antigravity, sem jargão);
    /// selecionar o Claude o DESLIGA (não-vacuoso: remova `eol_notice` e este falha).
    #[test]
    fn gemini_selection_shows_transition_notice_claude_does_not() {
        let mut m = modal();
        let mut engines = engines_one();
        engines.push(Engine {
            id: "gemini".into(),
            label: engine_label("gemini"),
            version: None,
            program: "gemini".into(),
            args: vec![],
            profile_id: Some("gemini".into()),
        });
        m.set_engines(engines);
        assert!(
            m.eol_notice().is_none(),
            "Claude selecionado (default) → sem aviso"
        );
        m.select_engine(1);
        let notice = m.eol_notice().expect("Gemini selecionado → aviso presente");
        assert!(notice.contains("18/jun/2026"), "data dura do profile");
        assert!(notice.contains("Antigravity"), "aponta o sucessor");
        assert!(
            !notice.to_lowercase().contains("eol"),
            "zero jargão na tela"
        );
        m.select_engine(0);
        assert!(m.eol_notice().is_none(), "voltar ao Claude apaga o aviso");
    }

    /// Rodada 360 (ADR 0022 §4): o consentimento do kit é OPT-IN explícito — nasce
    /// desligado, o toggle liga, e o `CreatePlan` carrega o valor para o funil de admissão.
    #[test]
    fn kit_consent_explicit_optin_flows_into_plan() {
        let mut m = modal();
        type_str(&mut m, "Free Lancer");
        m.set_engines(engines_one());
        assert!(
            !m.user_cwd_chosen(),
            "sem pasta própria, a linha nem aparece"
        );
        m.set_cwd(PathBuf::from("/tmp/projeto-do-usuario"));
        assert!(m.user_cwd_chosen());
        assert!(!m.kit_consent(), "opt-in explícito: nasce DESLIGADO");
        let plan = m.create_plan().expect("plano");
        assert!(
            !plan.kit_consent,
            "sem toque, o plano vai sem consentimento"
        );

        m.toggle_kit_consent();
        assert!(m.kit_consent());
        let plan = m.create_plan().expect("plano consentido");
        assert!(plan.kit_consent, "o toggle do modal chega ao plano");
    }

    /// M6-E2: o co-piloto dispara na PAUSA (~600ms = SUGGEST_DEBOUNCE_TICKS), nunca a cada tecla.
    #[test]
    fn suggestion_fires_after_debounce_pause() {
        let mut m = modal();
        type_str(&mut m, "Revisor");
        assert!(m.suggestion().is_none(), "sem sugestão antes da pausa");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS - 1 {
            m.tick();
        }
        assert!(m.suggestion().is_none(), "ainda dentro da pausa");
        assert!(m.tick(), "o tick da pausa produz mudança visível");
        let s = m.suggestion().expect("linha-fantasma");
        assert_eq!(s.role, "reviewer");
        assert!(!s.why.is_empty(), "transparência: o porquê vem junto");
    }

    /// Aceitar (Tab) preenche o cartão de Papel; a sugestão NUNCA se aplica sozinha (governança).
    #[test]
    fn accept_fills_role_card_never_silently() {
        let mut m = modal();
        type_str(&mut m, "Revisor");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert!(m.role().is_none(), "antes do Tab o papel está vazio");
        assert!(m.accept_suggestion());
        assert_eq!(m.role().expect("cartão").role, "reviewer");
        assert!(m.suggestion().is_none(), "ghost some após aceitar");
        // continuar digitando NÃO muda o papel aplicado sem novo aceite.
        type_str(&mut m, " Senior");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert_eq!(m.role().expect("cartão").role, "reviewer");
    }

    /// Re-sugestão só em mudança SUBSTANCIAL (papel diferente) — não a cada tecla (M6-E2).
    #[test]
    fn resuggests_only_on_substantial_change() {
        let mut m = modal();
        type_str(&mut m, "Revisor");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        m.accept_suggestion();
        // mesmo papel re-derivado → sem ghost novo (sem ruído).
        type_str(&mut m, " de PRs");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert!(m.suggestion().is_none(), "mesmo papel não re-sugere");
        // nome que muda o papel → ghost novo aparece.
        for _ in 0.."Revisor de PRs".len() {
            m.backspace();
        }
        type_str(&mut m, "Designer de Landing");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert_eq!(
            m.suggestion().expect("re-sugere").role,
            "UIUX_DESIGNER",
            "mudança substancial re-sugere"
        );
    }

    /// Critério 5: nome vazio/só-espaços → erro INLINE leigo; NENHUM plano (nenhum evento).
    #[test]
    fn empty_name_is_rejected_inline_before_commit() {
        let mut m = modal();
        m.set_engines(engines_one());
        assert_eq!(m.create_plan().unwrap_err(), COPY_ERR_EMPTY_NAME);
        type_str(&mut m, "   ");
        assert_eq!(m.create_plan().unwrap_err(), COPY_ERR_EMPTY_NAME);
    }

    /// Critérios 3/4: varrendo → espera leiga; zero motores → caminho de instalar (nunca beco);
    /// com motor → plano com comando do CLI Profile (programa + args do TOML).
    #[test]
    fn engine_states_guard_creation() {
        let mut m = modal();
        type_str(&mut m, "Revisor");
        assert_eq!(m.create_plan().unwrap_err(), COPY_ERR_SCANNING);
        m.set_engines(vec![]);
        assert_eq!(m.create_plan().unwrap_err(), COPY_ERR_NO_ENGINE);
        m.set_engines(engines_one());
        let plan = m.create_plan().expect("plano");
        assert_eq!(plan.engine.program, "claude");
        assert_eq!(plan.engine.args, vec!["--flag".to_string()]);
        assert_eq!(plan.engine.profile_id.as_deref(), Some("claude-code"));
    }

    /// Dedup do nome (tabela de erros): sufixo automático "(2)" + aviso leve, sem bloquear.
    #[test]
    fn duplicate_name_gets_suffix_and_note() {
        let mut m = AgentModal::new_create(
            Arc::new(W31Suggester),
            vec!["Revisor".into(), "Revisor (2)".into()],
        );
        m.set_engines(engines_one());
        type_str(&mut m, "Revisor");
        let plan = m.create_plan().expect("plano");
        assert_eq!(plan.name, "Revisor (3)");
        assert!(plan.dup_note.is_some(), "aviso leve presente");
    }

    /// Critério 7 (neutralidade): os profiles são lidos do DISCO em runtime — trocar o TOML muda
    /// o comando do quick-start SEM recompilar (seed na 1ª vez; depois o arquivo manda).
    #[test]
    fn command_follows_profile_toml_on_disk() {
        let dir = std::env::temp_dir().join(format!(
            "lina-m6-profiles-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        // 1ª carga: semeia o claude-code.toml default e o lê do disco.
        let reg = load_profiles(&dir);
        let found = vec![DiscoveredCli {
            id: "claude".into(),
            version: None,
            path: "/usr/local/bin/claude".into(),
        }];
        let engines = engines_from(&found, &reg);
        assert_eq!(engines[0].profile_id.as_deref(), Some("claude-code"));
        assert_eq!(engines[0].program, "claude");
        let default_args = engines[0].args.clone();
        assert!(!default_args.is_empty(), "args do TOML default presentes");

        // Usuário TROCA o TOML no disco (sem recompilar) → o comando muda.
        std::fs::write(
            dir.join("claude-code.toml"),
            r#"
id = "claude-code"
program = "claude"
args = ["--minha-flag-custom"]
delivery = "pty_inject"
prompt_ready_regex = '.'
[end_signal]
kind = "idle"
"#,
        )
        .expect("escrever TOML custom");
        let reg2 = load_profiles(&dir);
        let engines2 = engines_from(&found, &reg2);
        assert_eq!(
            engines2[0].args,
            vec!["--minha-flag-custom".to_string()],
            "trocar o TOML mudou o comando sem recompilar"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F1-1-9 (seeding): a 1ª carga leva os profiles EMBUTIDOS (claude-code + os NOVOS
    /// Antigravity e Gemini) ao dir de runtime do usuário — um dir vazio vira um registry que
    /// conhece os 3 CLIs, sem o usuário copiar TOML algum. (O repo sozinho não bastaria: o
    /// modal só lê o disco; é o seeding que faz a ponte repo → runtime.)
    #[test]
    fn seeding_writes_all_embedded_profiles_on_first_load() {
        let dir = std::env::temp_dir().join(format!(
            "lina-f1-1-9-seed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir); // garante dir limpo

        let reg = load_profiles(&dir);

        // Os profiles embutidos foram semeados e carregados na 1ª vez (codex entra na A2A
        // universal — sem o seeding, a entrega A2A ao Codex cairia no fallback do Claude).
        for id in ["claude-code", "antigravity", "gemini", "codex"] {
            assert!(
                reg.get(id).is_some(),
                "{id} deve ser semeado e carregado na 1ª carga"
            );
        }
        // E os arquivos REALMENTE existem no disco (não só em RAM).
        assert!(
            dir.join("antigravity.toml").exists(),
            "antigravity.toml no disco"
        );
        assert!(dir.join("gemini.toml").exists(), "gemini.toml no disco");
        assert!(dir.join("codex.toml").exists(), "codex.toml no disco");
        // Identidade do Codex: o binário descoberto "codex" casa o profile → o nó ganha o
        // profile_id "codex" (via `engines_from`/`CliProfileSet`) e a entrega resolve o perfil DELE.
        let codex = reg.get("codex").expect("codex carregado");
        assert_eq!(codex.program, "codex");

        // Identidade-chave do sucessor: binário `agy` (não `gemini`) + capability honesta.
        let agy = reg.get("antigravity").expect("antigravity carregado");
        assert_eq!(agy.program, "agy");
        assert!(
            !agy.capabilities.has("hooks"),
            "spike .agents/hooks.json adiado → hooks=false"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Esc com texto ARMA a confirmação de descarte; o 2º Esc fecha; sem texto fecha direto.
    #[test]
    fn escape_arms_discard_confirmation() {
        let mut m = modal();
        assert!(m.escape(), "vazio fecha direto");
        let mut m = modal();
        type_str(&mut m, "Revisor");
        assert!(!m.escape(), "1º Esc arma, não fecha");
        assert!(m.discard_armed());
        assert!(m.escape(), "2º Esc descarta");
        // digitar desarma (o usuário desistiu de descartar).
        let mut m = modal();
        type_str(&mut m, "Rev");
        m.escape();
        m.type_char("i");
        assert!(!m.discard_armed(), "digitar desarma o descarte");
    }

    /// **GUARDIÃO DO SELETOR (F1-2-3 + feedback do fundador: "dropdown, não ciclar")**:
    /// [Trocar] abre a biblioteca com TODOS os papéis visíveis de uma vez (nome leigo + 1 frase,
    /// nenhum jargão); a busca filtra por nome OU descrição; escolher aplica NA HORA, pina (a
    /// re-sugestão não atropela) e fecha o seletor.
    #[test]
    fn gallery_shows_all_roles_at_once_search_and_pick() {
        let mut m = modal();
        m.open_gallery();
        let total = crate::role_suggester::role_registry()
            .expect("registry")
            .role_names()
            .count();
        {
            let g = m.gallery().expect("seletor aberto");
            let all = g.filtered();
            assert_eq!(all.len(), total, "TODOS os papéis da biblioteca de uma vez");
            assert!(
                all.len() >= 10,
                "biblioteca real, não amostra: {}",
                all.len()
            );
            for r in &all {
                assert!(
                    !r.suggestion.label.contains('_'),
                    "rótulo leigo: {}",
                    r.suggestion.label
                );
                assert!(
                    !r.suggestion.blurb.is_empty(),
                    "1 frase por papel: {}",
                    r.suggestion.label
                );
            }
        }
        // busca por TEXTO (nome leigo)…
        m.gallery_type("revis");
        assert_eq!(m.gallery().unwrap().filtered().len(), 1, "busca estreita");
        // …e escolher aplica na hora + pina + fecha.
        assert!(m.gallery_pick(None));
        assert!(!m.gallery_open(), "escolher fecha o seletor");
        assert_eq!(m.role().expect("papel aplicado").role, "reviewer");
        type_str(&mut m, "Designer de Landing");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert_eq!(
            m.role().unwrap().role,
            "reviewer",
            "pinado não troca sozinho"
        );
        // busca por DESCRIÇÃO também acha (ex.: "telas" → Especialista em telas).
        m.open_gallery();
        m.gallery_type("telas");
        assert!(
            m.gallery()
                .unwrap()
                .filtered()
                .iter()
                .any(|r| r.suggestion.role == "FRONTEND"),
            "busca pela descrição/função acha o papel"
        );
    }

    /// Teclado do seletor (a11y do gate): ↑↓ navega com clamp nas bordas, Enter escolhe o
    /// destacado, Esc fecha SEM aplicar; busca sem resultado → Enter não aplica nada.
    #[test]
    fn gallery_keyboard_navigation_and_escape() {
        let mut m = modal();
        m.open_gallery();
        m.gallery_move(-1);
        assert_eq!(m.gallery().unwrap().selected(), 0, "clamp no topo");
        m.gallery_move(1);
        m.gallery_move(1);
        assert_eq!(m.gallery().unwrap().selected(), 2);
        let expected = m.gallery().unwrap().filtered()[2].suggestion.role.clone();
        assert!(m.gallery_pick(None), "Enter escolhe o destacado");
        assert_eq!(m.role().unwrap().role, expected);
        // Esc fecha sem aplicar.
        let mut m2 = modal();
        m2.open_gallery();
        m2.gallery_close();
        assert!(!m2.gallery_open());
        assert!(m2.role().is_none(), "Esc não aplica nada");
        // busca sem resultados → Enter é no-op (não aplica fantasma).
        let mut m3 = modal();
        m3.open_gallery();
        m3.gallery_type("zzz-nada");
        assert!(m3.gallery().unwrap().filtered().is_empty());
        assert!(!m3.gallery_pick(None), "sem resultado, nada a escolher");
        assert!(
            m3.gallery_open(),
            "seletor segue aberto p/ corrigir a busca"
        );
    }

    /// **GUARDIÃO (fix de tela bugB2 — dropdown estourava modal e janela)**: a caixa do modal
    /// SEMPRE cabe na janela — largura de projeto (640) só quando há espaço; janela estreita
    /// (o screenshot do fundador, ~736 lógicos) clampa com folga de respiro nas duas bordas.
    #[test]
    fn modal_frame_clamps_to_window() {
        // janela folgada → largura de projeto + teto de altura com margem dos dois lados.
        let f = modal_frame(WinSize {
            w: 1440.0,
            h: 900.0,
        });
        assert!((f.w - MODAL_W).abs() < 0.01, "largura de projeto: {}", f.w);
        assert!(
            (f.max_h - (900.0 - 2.0 * WINDOW_MARGIN)).abs() < 0.01,
            "teto = janela - 2*margem: {}",
            f.max_h
        );
        // janela estreita → a caixa encolhe (era o estouro horizontal da tela do fundador).
        let f = modal_frame(WinSize { w: 700.0, h: 800.0 });
        assert!(
            f.w <= 700.0 - 2.0 * WINDOW_MARGIN,
            "caixa dentro da janela estreita: {}",
            f.w
        );
        assert!(f.w >= MODAL_W_MIN, "nunca degenera abaixo do mínimo");
    }

    /// O dropdown NUNCA é mais largo que a largura útil do modal (requisito 1 do fundador) —
    /// em qualquer janela, a largura vem do frame, não do conteúdo.
    #[test]
    fn gallery_never_wider_than_modal_usable_width() {
        for win in [
            WinSize {
                w: 1440.0,
                h: 900.0,
            },
            WinSize { w: 700.0, h: 800.0 },
            WinSize { w: 420.0, h: 600.0 },
        ] {
            let frame = modal_frame(win);
            let lay = gallery_layout(frame, win, 14);
            assert!(
                (lay.w - (frame.w - 2.0 * MODAL_PAD_X)).abs() < 0.01,
                "largura útil exata do modal em {win:?}: {} vs frame {}",
                lay.w,
                frame.w
            );
        }
    }

    /// Requisito 2: lista longa (8+ papéis) → viewport LIMITADA com scroll interno; o dropdown
    /// inteiro (busca + viewport) cabe no orçamento vertical do modal dentro da janela.
    #[test]
    fn gallery_long_list_scrolls_inside_window_budget() {
        let win = WinSize {
            w: 1440.0,
            h: 900.0,
        };
        let frame = modal_frame(win);
        let lay = gallery_layout(frame, win, 14);
        let content = 14.0 * GALLERY_ROW_H + 13.0 * GALLERY_ROW_GAP;
        assert!(
            lay.list_viewport_h < content,
            "viewport menor que o conteúdo: {} < {content}",
            lay.list_viewport_h
        );
        assert!(lay.scrolls, "lista longa rola por dentro");
        assert!(
            MODAL_BASE_RESERVED + lay.h <= frame.max_h + 0.01,
            "modal + dropdown cabem no teto: {} + {} > {}",
            MODAL_BASE_RESERVED,
            lay.h,
            frame.max_h
        );
    }

    /// Poucos itens → viewport JUSTA (sem buraco morto) e sem scroll.
    #[test]
    fn gallery_few_items_fit_exactly_without_scroll() {
        let win = WinSize {
            w: 1440.0,
            h: 900.0,
        };
        let lay = gallery_layout(modal_frame(win), win, 3);
        let content = 3.0 * GALLERY_ROW_H + 2.0 * GALLERY_ROW_GAP;
        assert!(
            (lay.list_viewport_h - content).abs() < 0.01,
            "viewport == conteúdo: {} vs {content}",
            lay.list_viewport_h
        );
        assert!(!lay.scrolls);
    }

    /// Busca sem resultado (n=0) → viewport de UMA linha (a mensagem de vazio), sem scroll.
    #[test]
    fn gallery_empty_search_keeps_single_row_viewport() {
        let win = WinSize {
            w: 1440.0,
            h: 900.0,
        };
        let lay = gallery_layout(modal_frame(win), win, 0);
        assert!(
            (lay.list_viewport_h - GALLERY_ROW_H).abs() < 0.01,
            "1 linha p/ a mensagem de vazio: {}",
            lay.list_viewport_h
        );
        assert!(!lay.scrolls);
    }

    /// Janela DEGENERADA (menor que o orçamento base do modal): a viewport mantém o piso de
    /// 1 linha (usabilidade) — o transbordo residual é absorvido pelo scroll do corpo do modal
    /// (fallback declarativo no render), nunca pintado por cima do canvas.
    #[test]
    fn gallery_degenerate_window_keeps_one_row_floor() {
        let win = WinSize { w: 400.0, h: 300.0 };
        let lay = gallery_layout(modal_frame(win), win, 14);
        assert!(
            lay.list_viewport_h >= GALLERY_ROW_H - 0.01,
            "piso de 1 linha: {}",
            lay.list_viewport_h
        );
        assert!(lay.scrolls, "14 itens numa linha só → rola");
    }

    /// **Varredura de invariantes** (janelas × nº de itens): largura sempre a útil do modal;
    /// viewport sempre ≥ piso e ≤ máx de conforto; `scrolls` ⟺ conteúdo > viewport; altura do
    /// dropdown = chrome + viewport; e em janela não-degenerada (altura ≥ 700 = orçamento base
    /// + chrome + piso de 1 linha + margens) o modal INTEIRO respeita o teto — o bug não volta.
    #[test]
    fn gallery_containment_invariants_sweep() {
        for w in [360.0_f32, 640.0, 736.0, 1024.0, 1440.0, 2560.0] {
            for h in [300.0_f32, 600.0, 640.0, 700.0, 768.0, 900.0, 1200.0, 1600.0] {
                for n in [0_usize, 1, 3, 8, 14, 40] {
                    let win = WinSize { w, h };
                    let frame = modal_frame(win);
                    let lay = gallery_layout(frame, win, n);
                    let content = if n == 0 {
                        GALLERY_ROW_H
                    } else {
                        n as f32 * GALLERY_ROW_H + (n as f32 - 1.0) * GALLERY_ROW_GAP
                    };
                    assert!(
                        (lay.w - (frame.w - 2.0 * MODAL_PAD_X)).abs() < 0.01,
                        "largura útil em {w}x{h}/n={n}"
                    );
                    assert!(
                        lay.list_viewport_h >= GALLERY_ROW_H.min(content) - 0.01,
                        "piso da viewport em {w}x{h}/n={n}: {}",
                        lay.list_viewport_h
                    );
                    assert!(
                        lay.list_viewport_h <= GALLERY_VIEWPORT_MAX.max(GALLERY_ROW_H) + 0.01,
                        "máx de conforto em {w}x{h}/n={n}: {}",
                        lay.list_viewport_h
                    );
                    assert_eq!(
                        lay.scrolls,
                        content > lay.list_viewport_h + 0.5,
                        "scrolls ⟺ conteúdo > viewport em {w}x{h}/n={n}"
                    );
                    assert!(
                        (lay.h - (gallery_chrome_h() + lay.list_viewport_h)).abs() < 0.01,
                        "altura = chrome + viewport em {w}x{h}/n={n}"
                    );
                    if h >= 700.0 {
                        assert!(
                            MODAL_BASE_RESERVED + lay.h <= frame.max_h + 0.01,
                            "contenção estrita em {w}x{h}/n={n}: {} + {} > {}",
                            MODAL_BASE_RESERVED,
                            lay.h,
                            frame.max_h
                        );
                    }
                }
            }
        }
    }

    /// As constantes da matemática SÃO as do render (fonte única): linha da lista = 2 linhas de
    /// texto com line_height explícita (lição: a natural da fonte clipa) + py_2 + borda_2 dos
    /// dois lados (geometria uniforme — a linha inativa usa borda transparente).
    #[test]
    fn gallery_row_math_matches_render_constants() {
        assert!(
            (GALLERY_ROW_H - (2.0 * GALLERY_LINE_H + 2.0 * 8.0 + 2.0 * 2.0)).abs() < 0.01,
            "linha = 2*line_height + py_2 + border_2: {GALLERY_ROW_H}"
        );
        assert!(
            (gallery_chrome_h()
                - (2.0 * GALLERY_BORDER
                    + 2.0 * GALLERY_PAD
                    + 2.0 * GALLERY_GAP
                    + GALLERY_HEADER_H
                    + GALLERY_SEARCH_H))
                .abs()
                < 0.01,
            "chrome = bordas + paddings + gaps + cabeçalho + busca"
        );
    }

    /// **F1-2-3 p2 (duplicar+modificar)**: 'Duplicar' num template abre o editor LEVE
    /// pré-preenchido (nome "(cópia)", descrição = a frase do template; instruções vazias num
    /// embutido — o registry W3-1 não carrega doutrina); foco no Nome; detalhes FECHADOS
    /// (progressive disclosure — doutrina nunca obrigatória).
    #[test]
    fn gallery_duplicate_opens_prefilled_editor() {
        let mut m = modal();
        m.open_gallery();
        let (label, blurb) = {
            let it = &m.gallery().expect("aberta").filtered()[0];
            (it.suggestion.label.clone(), it.suggestion.blurb.clone())
        };
        m.gallery_duplicate(0);
        let ed = m.gallery().unwrap().editor().expect("editor aberto");
        assert_eq!(ed.name, copy_tpl_dup_name(&label));
        assert_eq!(ed.description, blurb);
        assert!(ed.doctrine.is_empty(), "embutido não carrega doutrina");
        assert_eq!(ed.focus, EditorField::Name);
        assert!(!ed.details, "detalhes fechados por padrão");
        assert!(m.gallery_open(), "galeria segue aberta por baixo");
    }

    /// O FUNIL de teclado do main.rs é fixo (gallery_type/backspace/move/pick/close) — com o
    /// editor aberto o MODELO reinterpreta: digitar vai pro campo focado, ↑↓ troca o campo,
    /// Enter NÃO escolhe papel nem fecha (salvar é o botão — clique explícito), Esc fecha SÓ o
    /// editor (volta à lista) e um segundo Esc fecha a galeria.
    #[test]
    fn gallery_editor_reinterprets_key_funnel() {
        let mut m = modal();
        m.open_gallery();
        m.gallery_duplicate(0);
        // limpa o nome pré-preenchido e digita um novo.
        while !m.gallery().unwrap().editor().unwrap().name.is_empty() {
            m.gallery_backspace();
        }
        m.gallery_type("Fa");
        m.gallery_type("z-tudo");
        assert_eq!(m.gallery().unwrap().editor().unwrap().name, "Faz-tudo");
        // ↓ foca a Descrição; digitar vai pra ela.
        m.gallery_move(1);
        assert_eq!(
            m.gallery().unwrap().editor().unwrap().focus,
            EditorField::Description
        );
        m.gallery_backspace();
        m.gallery_type("!");
        assert!(m
            .gallery()
            .unwrap()
            .editor()
            .unwrap()
            .description
            .ends_with('!'));
        // Enter no editor: NÃO escolhe papel, NÃO fecha, NÃO aplica nada (governança).
        assert!(!m.gallery_pick(None), "Enter no editor é no-op");
        assert!(
            m.gallery().unwrap().editor().is_some(),
            "editor segue aberto"
        );
        assert!(m.role().is_none(), "nenhum papel aplicado silenciosamente");
        // Esc 1: fecha só o editor (volta à lista). Esc 2: fecha a galeria.
        m.gallery_close();
        assert!(m.gallery_open(), "Esc no editor volta à LISTA");
        assert!(m.gallery().unwrap().editor().is_none());
        m.gallery_close();
        assert!(!m.gallery_open(), "segundo Esc fecha a galeria");
    }

    /// Progressive disclosure: a doutrina só entra no ciclo do ↑↓ com os detalhes ABERTOS;
    /// digitar com a doutrina focada edita a doutrina.
    #[test]
    fn gallery_editor_doctrine_behind_details() {
        let mut m = modal();
        m.open_gallery();
        m.gallery_duplicate(0);
        // Sem detalhes: ↓↓ para no campo Descrição (doutrina inacessível).
        m.gallery_move(1);
        m.gallery_move(1);
        assert_eq!(
            m.gallery().unwrap().editor().unwrap().focus,
            EditorField::Description,
            "sem detalhes a doutrina não entra no ciclo"
        );
        // Com detalhes: ↓ alcança a doutrina e digitar a edita.
        m.gallery_editor_toggle_details();
        m.gallery_move(1);
        assert_eq!(
            m.gallery().unwrap().editor().unwrap().focus,
            EditorField::Doctrine
        );
        m.gallery_type("Você revisa o trabalho dos colegas.");
        assert!(m
            .gallery()
            .unwrap()
            .editor()
            .unwrap()
            .doctrine
            .starts_with("Você revisa"));
    }

    /// O plano de salvar valida o NOME (vazio → erro leigo, nunca evento); válido → o
    /// [`RoleTemplate`] exato (trim aplicado) que o seam do bridge vai persistir.
    #[test]
    fn gallery_editor_plan_requires_name() {
        let mut m = modal();
        m.open_gallery();
        m.gallery_duplicate(0);
        while !m.gallery().unwrap().editor().unwrap().name.is_empty() {
            m.gallery_backspace();
        }
        assert_eq!(
            m.gallery_editor_plan(),
            Err(COPY_TPL_ERR_EMPTY_NAME.to_string()),
            "sem nome não há evento"
        );
        m.gallery_type("  Minha Revisora  ");
        let tpl = m.gallery_editor_plan().expect("plano válido");
        assert_eq!(tpl.name, "Minha Revisora", "trim no nome");
        assert!(!tpl.description.is_empty(), "descrição herdada do template");
    }

    /// Customs REAPARECEM: `gallery_set_customs` aplica last-wins por nome (doutrina do evento
    /// no core), lista os customs JUNTO dos embutidos (com flag), a BUSCA acha por nome E por
    /// descrição custom, e escolher um custom aplica papel = nome (só por gesto explícito).
    #[test]
    fn gallery_customs_last_wins_search_and_pick() {
        let mut m = modal();
        m.open_gallery();
        let builtins = m.gallery().unwrap().filtered().len();
        m.gallery_set_customs(vec![
            RoleTemplate {
                name: "Faz-tudo".into(),
                description: "versão velha".into(),
                doctrine: String::new(),
            },
            RoleTemplate {
                name: "Garimpeira".into(),
                description: "Garimpa fornecedores baratos.".into(),
                doctrine: "instruções da garimpeira".into(),
            },
            RoleTemplate {
                name: "Faz-tudo".into(),
                description: "Resolve qualquer pepino do dia.".into(),
                doctrine: String::new(),
            },
        ]);
        {
            let g = m.gallery().unwrap();
            let all = g.filtered();
            assert_eq!(all.len(), builtins + 2, "last-wins: Faz-tudo 1× só");
            let faz = all
                .iter()
                .find(|it| it.suggestion.label == "Faz-tudo")
                .expect("custom listado");
            assert!(faz.custom, "flag de custom");
            assert_eq!(
                faz.suggestion.blurb, "Resolve qualquer pepino do dia.",
                "o ÚLTIMO salvar vence"
            );
        }
        // busca pela DESCRIÇÃO custom…
        m.gallery_type("pepino");
        assert_eq!(m.gallery().unwrap().filtered().len(), 1);
        // …e escolher aplica papel = nome do custom (gesto explícito, critério 4).
        assert!(m.gallery_pick(None));
        assert_eq!(m.role().expect("papel aplicado").role, "Faz-tudo");
        assert!(!m.gallery_open(), "escolher fecha a galeria");
    }

    /// **Critério (a) — a perna do REPLAY com o core real**: 2 saves do mesmo nome + 1 de outro
    /// no EventStore; REABRIR o store (replay de verdade) e varrer os records como o bridge
    /// fará (kind + payload serde) → um modal NOVO lista o custom com o campo modificado.
    #[test]
    fn custom_roles_survive_replay_into_new_modal() {
        let dir = std::env::temp_dir().join(format!("lina-f123p2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut store = lina_core::EventStore::open(&dir).expect("abrir store");
            for (name, desc) in [
                ("Garimpeira", "rascunho"),
                ("Garimpeira", "Garimpa fornecedores baratos."),
            ] {
                store
                    .append(&lina_core::DomainEvent::RoleTemplateSaved {
                        name: name.into(),
                        description: desc.into(),
                        doctrine: "instruções".into(),
                        ts: 1,
                    })
                    .expect("append");
            }
        } // fecha o store — o replay abaixo parte do DISCO.
        let store = lina_core::EventStore::open(&dir).expect("reabrir (replay)");
        // Varredura ESPELHO do seam `role_templates()`: ordem de log, sem dedup.
        let rows: Vec<RoleTemplate> = store
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == "RoleTemplateSaved")
            .map(|r| RoleTemplate {
                name: r.payload["name"].as_str().unwrap_or_default().to_string(),
                description: r.payload["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                doctrine: r.payload["doctrine"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            })
            .collect();
        assert_eq!(rows.len(), 2, "sem dedup no seam (last-wins é do modal)");
        let mut m = modal(); // modal NOVO — critério: reaparece nos próximos modais.
        m.open_gallery();
        m.gallery_set_customs(rows);
        let g = m.gallery().unwrap();
        let hits: Vec<_> = g
            .filtered()
            .into_iter()
            .filter(|it| it.suggestion.label == "Garimpeira")
            .collect();
        assert_eq!(hits.len(), 1, "1 custom após last-wins");
        assert_eq!(
            hits[0].suggestion.blurb, "Garimpa fornecedores baratos.",
            "o campo MODIFICADO sobreviveu ao replay"
        );
        assert_eq!(hits[0].doctrine, "instruções");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **GUARDIÃO (regressão bugB3 — [Trocar] expulso da linha Papel)**: em TODA largura que o
    /// `modal_frame` produz, TODA linha texto+ação do modal mantém a ação DENTRO da linha — o
    /// texto é quem cede. Mecanismo da regressão: o min-content do texto gpui é a LINHA INTEIRA
    /// quando a medição roda sem largura Definite (e o cache de TextLayout torna isso dependente
    /// da ORDEM das medições do taffy) → célula sem `min_w(0)` não encolhe e empurra o botão p/
    /// fora (o scroll container clipa → botão some). A doutrina do render: célula `min_w(0)` +
    /// `overflow_hidden`, ação `flex_none` — e esta varredura prova a geometria viável.
    #[test]
    fn action_rows_contained_in_every_modal_frame() {
        // (label_w, gap, action_w por cima) das linhas texto+ação REAIS do modal:
        // título+✕ · Papel+[Trocar] · Pasta+[Trocar…] · ghost+ⓘ por quê? · rodapé (2 botões).
        let rows = [
            (0.0_f32, ROW_GAP, 40.0_f32),  // título: ✕
            (ROW_LABEL_W, ROW_GAP, 100.0), // Papel: [Trocar]
            (ROW_LABEL_W, ROW_GAP, 110.0), // Pasta: [Trocar…]
            (0.0, 8.0, 80.0),              // ghost: ⓘ por quê?
            (0.0, 8.0, 250.0),             // rodapé: Cancelar+Criar Agente (par à direita)
            // F1-2-3 p2 — superfícies novas:
            (0.0, 8.0, 110.0), // linha da galeria: [Duplicar]
            (0.0, 8.0, 200.0), // cabeçalho do editor: hint "↑↓ muda o campo…"
            (0.0, 8.0, 250.0), // rodapé do editor: Voltar+Salvar papel
        ];
        for w in [360.0_f32, 640.0, 736.0, 1024.0, 1440.0, 2560.0] {
            for h in [300.0_f32, 700.0, 900.0, 1600.0] {
                let frame = modal_frame(WinSize { w, h });
                let usable = frame.w - 2.0 * MODAL_PAD_X;
                for (label_w, gap, action_w) in rows {
                    // Pré-condição estrutural (garante que o caso degenerado do clamp da ação
                    // nunca é atingido com a geometria real): partes fixas cabem SEMPRE.
                    assert!(
                        label_w + 2.0 * gap + action_w <= usable + 0.01,
                        "partes fixas não cabem em {w}x{h} (usable {usable}): \
                         label {label_w} + gaps + ação {action_w}"
                    );
                    let r = action_row_layout(usable, label_w, gap, action_w);
                    assert!(r.action_x >= -0.01, "ação começa dentro em {w}x{h}");
                    assert!(
                        r.action_x + r.action_w <= usable + 0.01,
                        "ação TERMINA dentro em {w}x{h}: {} + {} > {usable}",
                        r.action_x,
                        r.action_w
                    );
                    assert!(
                        r.action_x >= label_w + gap - 0.01,
                        "ação não sobrepõe a label em {w}x{h}"
                    );
                    assert!(
                        (r.text_w - (usable - label_w - action_w - 2.0 * gap)).abs() < 0.01,
                        "o texto cede exatamente o resto em {w}x{h}: {}",
                        r.text_w
                    );
                    assert!(r.text_w >= 0.0, "célula nunca negativa");
                }
            }
        }
    }

    /// Degenerado do `action_row_layout`: ação mais larga que a própria linha é CLAMPADA para
    /// dentro (nunca sai), e a célula de texto zera — o render real nunca chega aqui (a
    /// pré-condição da varredura garante), mas a função não pode mentir se chegar.
    #[test]
    fn action_row_clamps_oversized_action_inside() {
        let r = action_row_layout(200.0, 120.0, 12.0, 400.0);
        assert!((r.action_w - 200.0).abs() < 0.01, "ação clampada à linha");
        assert!(r.action_x >= -0.01 && r.action_x + r.action_w <= 200.01);
        assert!((r.text_w - 0.0).abs() < 0.01, "texto cede tudo");
    }

    /// **Critério 3 da F1-2-3 (auditoria de deps)**: NENHUM client de LLM/rede no app — a
    /// sugestão é heurística local (W3-1), offline por construção (invariantes #1/#2).
    #[test]
    fn no_llm_or_network_client_deps() {
        let manifest = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
        )
        .expect("Cargo.toml");
        for forbidden in [
            "openai",
            "anthropic",
            "genai",
            "llm",
            "reqwest",
            "hyper",
            "ureq",
            "curl",
        ] {
            assert!(
                !manifest.lines().any(|l| {
                    let l = l.trim();
                    !l.starts_with('#') && l.starts_with(forbidden)
                }),
                "dep proibida no app (LLM/rede): {forbidden}"
            );
        }
    }

    /// M6-E (modo edição): pré-preenchido; Salvar detecta mudança de nome e papel; renomear p/
    /// o PRÓPRIO nome não ganha sufixo de dedup.
    #[test]
    fn edit_mode_save_plan_detects_changes() {
        let node = uuid::Uuid::now_v7();
        let mut m = AgentModal::new_edit(
            Arc::new(W31Suggester),
            node,
            "Ajudante",
            Some("WRITER"),
            Autonomy::Assisted,
            None,
            None,
            vec!["Ajudante".into(), "Outro".into()],
        );
        assert_eq!(m.role().unwrap().role, "WRITER");
        // sem mudanças: plano preserva nome, papel None (não mudou).
        let p = m.save_plan().expect("plano");
        assert_eq!(p.name, "Ajudante");
        assert_eq!(p.role, None);
        // muda papel via SELETOR → plano carrega papel novo.
        m.open_gallery();
        m.gallery_move(1); // um papel diferente do atual
        assert!(m.gallery_pick(None));
        let role_now = m.role().unwrap().role.clone();
        let p = m.save_plan().expect("plano");
        assert_eq!(p.role.as_deref(), Some(role_now.as_str()));
    }

    /// ADR 0037: editar a Pasta de Trabalho. `new_edit` pré-carrega o cwd REAL do nó (o campo
    /// mostra a pasta de verdade, não o default genérico); sem troca, `cwd` não entra no plano;
    /// trocar carrega `cwd` no `SavePlan`; `restart_now` só vale com a pasta trocada; voltar à
    /// pasta original zera a mudança (nunca persiste troca falsa).
    #[test]
    fn edit_mode_save_plan_detects_cwd_change() {
        let node = uuid::Uuid::now_v7();
        let mut m = AgentModal::new_edit(
            Arc::new(W31Suggester),
            node,
            "Ajudante",
            Some("WRITER"),
            Autonomy::Assisted,
            Some(Path::new("/Users/founder/pasta-original")),
            None,
            vec!["Ajudante".into()],
        );
        // O campo mostra a pasta REAL (não o default genérico) e, sem troca, não entra no plano.
        assert_eq!(m.cwd_display(), "/Users/founder/pasta-original");
        assert!(!m.cwd_changed());
        let p = m.save_plan().expect("plano");
        assert_eq!(p.cwd, None, "pasta intocada → não persiste troca");
        assert!(!p.restart_now);

        // "reiniciar agora" SEM trocar a pasta é inerte (não há pasta nova a aplicar).
        m.toggle_restart_now();
        assert!(!m.save_plan().expect("plano").restart_now);

        // trocar a pasta → entra no plano; com o toggle já ligado, `restart_now` passa a valer.
        m.set_cwd(PathBuf::from("/Users/founder/pasta-nova"));
        assert!(m.cwd_changed());
        let p = m.save_plan().expect("plano");
        assert_eq!(
            p.cwd.as_deref(),
            Some(Path::new("/Users/founder/pasta-nova"))
        );
        assert!(
            p.restart_now,
            "pasta trocada + toggle ligado → reinicia agora"
        );

        // voltar para a pasta original zera a mudança (nunca persiste troca falsa).
        m.set_cwd(PathBuf::from("/Users/founder/pasta-original"));
        assert!(!m.cwd_changed());
        assert_eq!(m.save_plan().expect("plano").cwd, None);
    }

    /// ADR 0039: trocar o MOTOR no Editar. `set_engines` pré-seleciona o motor ATUAL (sem troca
    /// falsa quando a lista vem com outro 1º); clicar o mesmo motor não conta; clicar OUTRO
    /// carrega o engine no `SavePlan` (que re-ergue o Agente com o novo CLI no commit).
    #[test]
    fn edit_mode_save_plan_detects_engine_change() {
        let node = uuid::Uuid::now_v7();
        let mut m = AgentModal::new_edit(
            Arc::new(W31Suggester),
            node,
            "Ajudante",
            Some("WRITER"),
            Autonomy::Assisted,
            None,
            Some("codex"), // motor ATUAL do nó = codex (não o 1º da lista)
            vec!["Ajudante".into()],
        );
        // a descoberta chega com claude (1º) + codex — a pré-seleção tem que casar o ATUAL (codex).
        let engines = vec![
            Engine {
                id: "claude".into(),
                label: "Claude Code".into(),
                version: None,
                program: "claude".into(),
                args: vec![],
                profile_id: Some("claude-code".into()),
            },
            Engine {
                id: "codex".into(),
                label: "Codex".into(),
                version: None,
                program: "codex".into(),
                args: vec![],
                profile_id: Some("codex".into()),
            },
        ];
        m.set_engines(engines);
        // sem clique → motor não mudou (a pré-seleção do atual NÃO marca troca falsa).
        assert!(
            !m.engine_changed(),
            "pré-seleção do motor atual não é troca"
        );
        assert_eq!(m.save_plan().expect("plano").engine, None);
        // clicar o MESMO motor (codex, idx 1) → não conta.
        m.select_engine(1);
        assert!(!m.engine_changed(), "clicar o mesmo motor não é troca");
        assert_eq!(m.save_plan().expect("plano").engine, None);
        // clicar OUTRO (claude, idx 0) → troca; o SavePlan carrega o engine claude.
        m.select_engine(0);
        assert!(m.engine_changed());
        let eng = m
            .save_plan()
            .expect("plano")
            .engine
            .expect("engine trocado");
        assert_eq!(eng.profile_id.as_deref(), Some("claude-code"));
        assert_eq!(eng.program, "claude");
    }

    /// FIX-3 (entrega 1, render headless): a seção AUTONOMIA tem as 3 opções na ordem canônica,
    /// cada uma com descrição leiga de UMA linha, distintas entre si — e a do assistido responde
    /// a dúvida do fundador (de onde vêm os pedidos de sim/não).
    #[test]
    fn autonomy_options_have_one_lay_line_each() {
        assert_eq!(
            AUTONOMY_OPTIONS,
            [Autonomy::Manual, Autonomy::Assisted, Autonomy::Autonomous]
        );
        let mut seen = Vec::new();
        for a in AUTONOMY_OPTIONS {
            let d = autonomy_desc(a);
            assert!(!d.trim().is_empty(), "descrição vazia p/ {a:?}");
            assert!(!d.contains('\n'), "descrição de {a:?} não é 1 linha");
            assert!(
                !seen.contains(&d),
                "descrições devem ser distintas ({a:?} repetiu)"
            );
            seen.push(d);
        }
        // O conteúdo que o FIX-3 promete (sem acoplar a frase inteira):
        assert!(autonomy_desc(Autonomy::Manual).contains("passo a passo"));
        assert!(autonomy_desc(Autonomy::Assisted).contains("sim/não"));
        assert!(autonomy_desc(Autonomy::Assisted).contains("confirmação"));
        assert!(autonomy_desc(Autonomy::Autonomous).contains("apagar, publicar, gastar"));
        // Rótulo de superfície é leigo (com acento) — não o vocabulário interno do env.
        assert_eq!(autonomy_surface_label(Autonomy::Autonomous), "autônomo");
    }

    /// FIX-3: a escolha do modal CHEGA ao plano de criação; o DEFAULT do produto (assistido)
    /// fica intocado — quem nunca tocar na seção cria exatamente como antes do fix.
    #[test]
    fn create_plan_carries_selected_autonomy_default_untouched() {
        let mut m = modal();
        type_str(&mut m, "Ajudante");
        m.set_engines(engines_one());
        assert_eq!(m.autonomy(), Autonomy::Assisted, "default do produto");
        let plan = m.create_plan().expect("plano");
        assert_eq!(plan.autonomy, Autonomy::Assisted, "sem toque → default");

        m.set_autonomy(Autonomy::Autonomous);
        let plan = m.create_plan().expect("plano");
        assert_eq!(
            plan.autonomy,
            Autonomy::Autonomous,
            "a escolha viaja TIPADA"
        );
    }

    /// F3-0-3 (ADR 0031): o `CreatePlan` carrega a CASA de `model`/`effort` (contrato aditivo). A
    /// seleção interativa (controles visíveis + badge) é F3-0-6; aqui o default é `None`.
    #[test]
    fn create_plan_carries_model_effort_house() {
        let mut m = modal();
        type_str(&mut m, "Ajudante");
        m.set_engines(engines_one());
        let plan = m.create_plan().expect("plano");
        assert_eq!(plan.model, None, "casa existe; seleção de modelo é F3-0-6");
        assert_eq!(
            plan.effort, None,
            "sem escolha no controle → None = default neutro (o CLI/Medium decide)"
        );
    }

    /// F3-0-6: a seleção no controle NOMEADO de Raciocínio chega ao `CreatePlan` (que o spawn leva
    /// a `LINA_EFFORT`+`EffortAssigned` em F3-0-4). E o controle reflete o default Medium até a
    /// primeira escolha — honesto: é o nível que o Agente nasce tendo.
    #[test]
    fn create_plan_carries_selected_effort() {
        let mut m = modal();
        type_str(&mut m, "Ajudante");
        m.set_engines(engines_one());
        assert_eq!(
            m.effort(),
            Effort::Medium,
            "casa não-tocada exibe o default neutro"
        );
        m.set_effort(Effort::High);
        assert_eq!(m.effort(), Effort::High, "o controle reflete a escolha");
        let plan = m.create_plan().expect("plano");
        assert_eq!(plan.effort, Some(Effort::High), "a escolha vai ao spawn");
    }

    /// FIX-3 (M6-E): autonomia segue a semântica role do Salvar — `None` = não mudou; mudar e
    /// voltar ao original também é `None` (reabrir+Salvar continua no-op observável).
    #[test]
    fn edit_mode_save_plan_detects_autonomy_change() {
        let node = uuid::Uuid::now_v7();
        let mut m = AgentModal::new_edit(
            Arc::new(W31Suggester),
            node,
            "Ajudante",
            Some("WRITER"),
            Autonomy::Assisted,
            None,
            None,
            vec!["Ajudante".into()],
        );
        assert_eq!(m.autonomy(), Autonomy::Assisted, "prefill do nível efetivo");
        let p = m.save_plan().expect("plano");
        assert_eq!(p.autonomy, None, "sem mudança → None");

        m.set_autonomy(Autonomy::Manual);
        let p = m.save_plan().expect("plano");
        assert_eq!(p.autonomy, Some(Autonomy::Manual), "mudança detectada");

        m.set_autonomy(Autonomy::Assisted);
        let p = m.save_plan().expect("plano");
        assert_eq!(p.autonomy, None, "voltar ao original → no-op de novo");
    }

    /// FIX-3 (entregas 2+3, projeção do card): badge DISTINTO por nível; o mini-aviso e a
    /// explicação do sim/não aparecem SÓ no assistido (é quando os pedidos existem); o rótulo
    /// anunciável aponta o ONDE ajustar (Editar).
    #[test]
    fn card_autonomy_badge_projects_per_level() {
        let badges: Vec<String> = AUTONOMY_OPTIONS
            .into_iter()
            .map(card_autonomy_badge)
            .collect();
        assert_eq!(badges.len(), 3);
        for (i, b) in badges.iter().enumerate() {
            assert!(b.starts_with("autonomia: "), "badge diz o que é: {b:?}");
            assert!(
                badges.iter().skip(i + 1).all(|o| o != b),
                "badges devem variar POR autonomia"
            );
        }
        assert!(card_autonomy_badge(Autonomy::Assisted).contains("pede sim/não"));
        assert!(!card_autonomy_badge(Autonomy::Manual).contains("pede sim/não"));
        assert!(!card_autonomy_badge(Autonomy::Autonomous).contains("pede sim/não"));

        assert_eq!(
            card_autonomy_explain(Autonomy::Assisted),
            Some(COPY_CARD_AUTONOMY_EXPLAIN)
        );
        assert_eq!(card_autonomy_explain(Autonomy::Manual), None);
        assert_eq!(card_autonomy_explain(Autonomy::Autonomous), None);
        let aria = card_autonomy_aria(Autonomy::Assisted);
        assert!(aria.contains("pedindo sua permissão"), "explica o sim/não");
        assert!(aria.contains("Editar"), "aponta ONDE ajustar");
    }

    /// FIX-3 (gate WCAG do badge): o par (texto, fundo) da FONTE ÚNICA `autonomy_badge_colors`
    /// atinge AA (≥4.5:1) nos 2 temas × 8 acentos — quem trocar o par re-passa por aqui.
    #[test]
    fn autonomy_badge_meets_wcag_in_all_themes() {
        use crate::a11y::wcag::{contrast_ratio, meets_aa};
        use crate::theme::{Mode, Theme, ACCENTS};
        for mode in [Mode::Dark, Mode::Light] {
            for (accent, _) in ACCENTS {
                let th = Theme::build(mode, accent);
                let (fg, bg) = autonomy_badge_colors(&th);
                assert!(
                    meets_aa(fg, bg),
                    "[modo={mode:?} acento={accent}] badge de autonomia: {:.2}:1 < 4.5:1",
                    contrast_ratio(fg, bg)
                );
            }
        }
    }

    /// **GUARDIÃO (fix de tela)**: nome SEM padrão conhecido ("Teles", o caso do fundador) →
    /// o co-piloto roda e o campo mostra o fallback HONESTO (`copilot_no_match`), nunca
    /// silêncio. Nome conhecido depois → o fallback dá lugar à sugestão viva no campo.
    #[test]
    fn unknown_name_shows_honest_fallback_never_silence() {
        let mut m = modal();
        type_str(&mut m, "Teles");
        assert!(!m.copilot_no_match(), "antes da pausa, sem veredito");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert!(m.suggestion().is_none(), "'Teles' não tem papel conhecido");
        assert!(
            m.copilot_no_match(),
            "rodou e não reconheceu → fallback honesto NO CAMPO (nunca silêncio)"
        );
        // nome que sugere → o fallback sai e a sugestão viva entra no campo.
        for _ in 0.."Teles".len() {
            m.backspace();
        }
        type_str(&mut m, "Revisor");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert!(m.suggestion().is_some(), "sugestão viva no campo");
        assert!(!m.copilot_no_match(), "fallback deu lugar à sugestão");
        // aceitar (clique no preview/Tab) limpa tudo e aplica.
        assert!(m.accept_suggestion());
        assert!(!m.copilot_no_match());
        assert_eq!(m.role().unwrap().role, "reviewer");
        // [Trocar] também limpa o no_match (escolha manual é resposta válida ao fallback).
        let mut m2 = modal();
        type_str(&mut m2, "Teles");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m2.tick();
        }
        assert!(m2.copilot_no_match());
        m2.open_gallery();
        assert!(m2.gallery_pick(None));
        assert!(!m2.copilot_no_match(), "Trocar resolve o fallback");
        assert!(m2.role().is_some());
    }

    /// Critério 2 (auditoria de copy): NENHUM estado do modal mostra YAML/TOML/"profile"/
    /// "terminal" — o leigo nunca vê jargão (anti-padrões 1/2/5 do fluxo (a)).
    #[test]
    fn copy_has_no_jargon() {
        let dynamic = [
            copy_ghost("Revisor"),
            copy_dup_note("Revisor (2)"),
            copy_tpl_dup_name("Revisor"),
            // FIX-3: toda a copy de autonomia (modal + badge/aria do card) passa pelo guardião.
            card_autonomy_badge(Autonomy::Manual),
            card_autonomy_badge(Autonomy::Assisted),
            card_autonomy_badge(Autonomy::Autonomous),
            card_autonomy_aria(Autonomy::Manual),
            card_autonomy_aria(Autonomy::Assisted),
            card_autonomy_aria(Autonomy::Autonomous),
            engine_label("claude"),
            engine_label("agy"),
        ];
        let all: Vec<&str> = [
            COPY_TITLE_CREATE,
            COPY_TITLE_EDIT,
            COPY_QUICKSTART,
            COPY_SCANNING,
            COPY_EMPTY_TITLE,
            COPY_EMPTY_CTA,
            COPY_INSTALL_OTHER,
            COPY_NAME_LABEL,
            COPY_NAME_PLACEHOLDER,
            COPY_ROLE_LABEL,
            COPY_ROLE_PLACEHOLDER,
            COPY_ROLE_NO_MATCH,
            COPY_ROLE_SUGGESTED_HINT,
            COPY_GALLERY_DUPLICATE,
            COPY_TPL_TITLE,
            COPY_TPL_NAME_LABEL,
            COPY_TPL_DESC_LABEL,
            COPY_TPL_DETAILS,
            COPY_TPL_DETAILS_OPEN,
            COPY_TPL_DOCTRINE_LABEL,
            COPY_TPL_SAVE,
            COPY_TPL_BACK,
            COPY_TPL_HINT,
            COPY_TPL_BADGE,
            COPY_TPL_WHY,
            COPY_TPL_ERR_EMPTY_NAME,
            COPY_TPL_ERR_SAVE,
            COPY_ROLE_SWAP,
            COPY_CWD_LABEL,
            COPY_CWD_DEFAULT,
            COPY_CWD_PICK,
            COPY_ADVANCED,
            COPY_ADVANCED_OPEN,
            COPY_CMD_LABEL,
            COPY_CREATE,
            COPY_SAVE,
            COPY_CANCEL,
            COPY_ERR_EMPTY_NAME,
            COPY_ERR_SCANNING,
            COPY_ERR_NO_ENGINE,
            COPY_DISCARD_ARMED,
            COPY_EDIT_ENGINE_RESTART_WARN,
            COPY_EDIT_APPLY_NOTE,
            COPY_WHY_PREFIX,
            // F3-5-3: seção "Continuar uma conversa" — toda string visível passa pelo guardião.
            COPY_RESUME_HEADER_CLOSED,
            COPY_RESUME_HEADER_OPEN,
            COPY_RESUME_HINT,
            COPY_RESUME_EMPTY,
            COPY_RESUME_UNAVAILABLE,
            // FIX-3: seção AUTONOMIA — rótulo, descrições leigas e explicação do card.
            COPY_AUTONOMY_LABEL,
            COPY_CARD_AUTONOMY_EXPLAIN,
            autonomy_surface_label(Autonomy::Manual),
            autonomy_surface_label(Autonomy::Assisted),
            autonomy_surface_label(Autonomy::Autonomous),
            autonomy_desc(Autonomy::Manual),
            autonomy_desc(Autonomy::Assisted),
            autonomy_desc(Autonomy::Autonomous),
        ]
        .into_iter()
        .chain(dynamic.iter().map(String::as_str))
        .collect();
        for s in all {
            let low = s.to_lowercase();
            for jargon in ["yaml", "toml", "profile", "terminal", "frontmatter"] {
                assert!(
                    !low.contains(jargon),
                    "jargão {jargon:?} na copy do modal: {s:?}"
                );
            }
        }
    }

    /// Comando custom no Avançado: o override entra no plano (programa + args re-parseados);
    /// trocar o motor selecionado descarta o override (o comando segue o motor).
    #[test]
    fn advanced_command_override_enters_plan() {
        let mut m = modal();
        m.set_engines(engines_one());
        type_str(&mut m, "Revisor");
        m.toggle_advanced();
        m.set_focus(FocusField::Command);
        // limpa o derivado e digita um comando custom.
        for _ in 0..m.effective_command().len() {
            m.backspace();
        }
        type_str(&mut m, "meu-cli --x");
        let plan = m.create_plan().expect("plano");
        assert_eq!(plan.engine.program, "meu-cli");
        assert_eq!(plan.engine.args, vec!["--x".to_string()]);
        // trocar o motor reseta o override.
        m.select_engine(0);
        assert_eq!(m.effective_command(), "claude --flag");
    }

    // ───────── F3-5-3 · seção "Continuar uma conversa" (estado do modal) ─────────

    fn saved_row(name: &str, resumable: bool) -> SavedSessionRow {
        SavedSessionRow {
            display_name: name.to_string(),
            age: "ontem".to_string(),
            command: if resumable {
                vec!["claude".into(), "--resume".into(), "sess".into()]
            } else {
                Vec::new()
            },
            profile_id: Some("claude-code".into()),
            role: Some("designer".into()),
            cwd: Some("/tmp/proj".into()),
            resumable,
        }
    }

    /// A seção nasce FECHADA e VAZIA (lazy): o caminho primário é criar; carregar é sob o gesto.
    #[test]
    fn saved_section_starts_closed_and_unloaded() {
        let m = modal();
        assert!(!m.saved_open(), "fechada por default");
        assert!(!m.saved_loaded(), "nada carregado até o 1º gesto de abrir");
        assert!(m.saved_rows().is_empty());
    }

    /// `toggle_saved` alterna; `set_saved_sessions` injeta as linhas e marca como carregado
    /// (distingue "ainda não carregou" de "carregou e está vazio" — estados de tela distintos).
    #[test]
    fn toggle_and_set_saved_sessions_drive_the_section() {
        let mut m = modal();
        m.toggle_saved();
        assert!(m.saved_open());
        assert!(
            !m.saved_loaded(),
            "abrir não carrega sozinho — quem carrega é a view"
        );
        m.set_saved_sessions(vec![saved_row("Revisor", true), saved_row("Velho", false)]);
        assert!(m.saved_loaded(), "carregou (mesmo que viesse vazio)");
        assert_eq!(m.saved_rows().len(), 2);
        assert!(m.saved_rows()[0].resumable);
        assert!(!m.saved_rows()[1].resumable);
        m.toggle_saved();
        assert!(!m.saved_open(), "fecha de novo");
        assert!(m.saved_loaded(), "fechar não descarta o cache");
    }

    /// `humanize_age` em buckets honestos (sem precisão falsa); `now < then` cai em "agora há pouco".
    #[test]
    fn humanize_age_buckets() {
        let min = 60_000u64;
        let hour = 60 * min;
        let day = 24 * hour;
        assert_eq!(humanize_age(1_000, 1_000), "agora há pouco");
        assert_eq!(humanize_age(30 * 1_000, 0), "agora há pouco", "<1 min");
        assert_eq!(humanize_age(5 * min, 0), "há 5 min");
        assert_eq!(humanize_age(3 * hour, 0), "há 3h");
        assert_eq!(humanize_age(day, 0), "ontem");
        assert_eq!(humanize_age(3 * day, 0), "há 3 dias");
        assert_eq!(humanize_age(10 * day, 0), "há 1 semana");
        assert_eq!(humanize_age(21 * day, 0), "há 3 semanas");
        assert_eq!(
            humanize_age(0, 999_999),
            "agora há pouco",
            "relógio não-monotônico"
        );
    }
}
