//! **F2-2-1 · Catálogo de componentes núcleo** — a story-base da Onda F2-2 (épico 38 §F2-2). Os
//! 4 componentes do núcleo (botão · painel/card · input · modal) materializam a fusão **T1+T3**
//! ("Instrumento de Estúdio com a temperatura do Ateliê", §VIII.1): cor = significado acoplado ao
//! acionável (OP-1), superfícies quentes, flat honesto, tipografia Plex/Fraunces dos tokens.
//!
//! **Padrão:** builder + `RenderOnce`, **view-agnóstico** — o `on_click` guarda o
//! `Fn(&ClickEvent, &mut Window, &mut App)` que o call-site produz com `cx.listener(...)`, então o
//! mesmo componente serve `WorkspaceView`/`OnboardingView`/`PersistenceView` sem genéricos. Toda
//! cor/medida vem de [`crate::theme::active`] (zero literal) — a catraca F2-1-5 nasce verde aqui e
//! CAI nos call-sites consolidados. Lógica de cor/medida por variante em funções PURAS (testáveis;
//! gpui não roda headless — o render é casca fina sobre elas).
//!
//! `allow(dead_code)`: catálogo-biblioteca consumido INCREMENTALMENTE — a r4 consolida os call-sites
//! diretos; F2-2-2..6 (toast/badge sobre o Element de live-region, paleta, toolbar) consomem o resto.
//! Mesmo padrão de [`crate::gallery`]/[`crate::canvas`]. Remover ao aterrissar o último consumidor.
#![allow(dead_code)]

use gpui::{App, ClickEvent, Window};

/// Handler de clique **view-agnóstico** que os componentes guardam. O call-site o produz com
/// `cx.listener(|view, ev, window, cx| …)`, que adapta um `Fn(&mut V, &E, &mut Window, &mut
/// Context<V>)` para este tipo — por isso o mesmo componente serve qualquer view sem genéricos.
pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

mod badge;
mod button;
mod input;
mod modal;
mod node_toolbar;
mod panel;
mod toast;

// Re-export glob: a API do catálogo é consumida INCREMENTALMENTE (r4 + F2-2-2..6); o glob expõe a
// superfície pública sem disparar `unused_imports` por item ainda-não-consumido (idioma de módulo-
// biblioteca). O `#![allow(dead_code)]` acima cobre os itens definidos ainda sem consumidor.
// F2-2-2: `badge`/`toast` ainda SEM consumidor (a migração por call-site — attention_ui/cards — é
// fronteira do C nesta rodada). O glob de um módulo 100% não-consumido dispara `unused_imports`
// (diferente de button/panel, já consumidos); o allow cai quando C aterrissar o 1º call-site.
#[allow(unused_imports)]
pub use badge::*;
pub use button::*;
pub use input::*;
pub use modal::*;
// F2-2-5: a toolbar é montada na costura `main.rs` (dono: C) — sem consumidor neste módulo ainda.
#[allow(unused_imports)]
pub use node_toolbar::*;
pub use panel::*;
#[allow(unused_imports)]
pub use toast::*;
