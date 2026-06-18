//! `session` — store de SESSÃO LOCAL por Espaço (story **F2-3-7**).
//!
//! Estado de sessão por-usuário que **NÃO** é fato do Espaço: hoje, a câmera (pan/zoom).
//! Vive FORA do event log (ADR 0029 §3: *"persistir ≠ event-sourcear"*) — perder este
//! arquivo perde no máximo o ENQUADRAMENTO, nunca um fato. Por isso NÃO é o "terceiro
//! store" que o ADR 0022 §6 proíbe: aquele fala dos FATOS do Espaço; isto é cache de
//! preferência descartável, e o `load` degrada honestamente para a câmera HOME na
//! ausência/corrupção. Por isso também NÃO mora na [`crate::Workspace`] (a guardiã dos
//! fatos) — é um seam separado, irmão de `.lina/events`, nunca dentro dele.
//!
//! Persistência: um `camera.json` sob o dir de sessão, escrito ATOMICAMENTE (temp +
//! rename) para nunca deixar um arquivo parcial num crash no meio da escrita.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// O diretório de sessão local de um Espaço: `<root>/.lina/session` — irmão de
/// `.lina/events`, NUNCA dentro dele (espelha [`crate::Workspace::events_dir`], mas
/// fora da Workspace: sessão ≠ fato).
#[must_use]
pub fn session_dir(root: &Path) -> PathBuf {
    root.join(".lina").join("session")
}

/// Erros do store de sessão. Só falhas REAIS de I/O/serialização ao SALVAR — o `load`
/// nunca falha (ausência/corrupção → câmera home, doutrina ADR 0029 §3).
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Snapshot da câmera (pan/zoom) de um Espaço — estado de SESSÃO LOCAL (ADR 0029 §3).
/// `f64` pela precisão de persistência; a UI converte de/para sua `Camera` (f32).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraSnapshot {
    pub pan: (f64, f64),
    pub zoom: f64,
}

impl Default for CameraSnapshot {
    /// HOME: pan `(0, 0)`, zoom `1.0` — o enquadramento de um Espaço sem snapshot.
    fn default() -> Self {
        Self {
            pan: (0.0, 0.0),
            zoom: 1.0,
        }
    }
}

impl CameraSnapshot {
    /// Sanidade de DOMÍNIO (independe dos limites de UI/zoom): valores finitos e zoom
    /// estritamente positivo — um `zoom` 0/negativo/`NaN` quebraria `screen_to_world`
    /// (divisão por zoom) na UI. Um snapshot insano (disco corrompido/editado à mão) é
    /// rejeitado para a câmera home, nunca propagado.
    #[must_use]
    fn is_sane(&self) -> bool {
        self.pan.0.is_finite() && self.pan.1.is_finite() && self.zoom.is_finite() && self.zoom > 0.0
    }
}

/// Store de sessão local de um Espaço (FORA do event log). Hoje guarda só a câmera.
#[derive(Debug, Clone)]
pub struct SessionStore {
    camera_path: PathBuf,
}

impl SessionStore {
    /// Abre o store sob o dir de sessão (use [`session_dir`]), criando-o se necessário.
    /// O `camera.json` fica em `<dir>/camera.json`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, SessionError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            camera_path: dir.join("camera.json"),
        })
    }

    /// Carrega a câmera. Ausente OU corrompida OU insana → HOME (ADR 0029 §3: perder o
    /// snapshot só perde o enquadramento). NUNCA falha — degradação honesta.
    #[must_use]
    pub fn load_camera(&self) -> CameraSnapshot {
        let Ok(raw) = std::fs::read_to_string(&self.camera_path) else {
            return CameraSnapshot::default();
        };
        match serde_json::from_str::<CameraSnapshot>(&raw) {
            Ok(cam) if cam.is_sane() => cam,
            _ => CameraSnapshot::default(),
        }
    }

    /// Salva a câmera ATOMICAMENTE (temp + rename): uma escrita interrompida nunca deixa
    /// um `camera.json` parcial — ou o antigo, ou o novo, jamais lixo. O tmp leva o PID
    /// (espelha `workspace.rs`): dois escritores concorrentes (a UI + o bin `lina`) nunca
    /// colidem no mesmo tmp; o `rename` é a troca atômica final.
    pub fn save_camera(&self, cam: &CameraSnapshot) -> Result<(), SessionError> {
        let json = serde_json::to_string(cam)?;
        let tmp = self
            .camera_path
            .with_extension(format!("json.tmp-{}", std::process::id()));
        std::fs::write(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, &self.camera_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DomainEvent, EventStore, Workspace};

    /// Diretório temporário único; removido no `Drop` (best-effort).
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lina-sess-{tag}-{}", uuid::Uuid::now_v7()));
            std::fs::create_dir_all(&p).expect("criar tempdir");
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn round_trip_salva_e_carrega_igual() {
        let root = TempDir::new("roundtrip");
        let store = SessionStore::open(session_dir(root.path())).expect("open");
        let cam = CameraSnapshot {
            pan: (123.5, -40.25),
            zoom: 1.75,
        };
        store.save_camera(&cam).expect("save");
        assert_eq!(store.load_camera(), cam, "carrega exatamente o que salvou");
    }

    #[test]
    fn ausencia_carrega_home() {
        let root = TempDir::new("ausencia");
        let store = SessionStore::open(session_dir(root.path())).expect("open");
        // Nenhum save → arquivo ausente → câmera home (nunca falha).
        assert_eq!(store.load_camera(), CameraSnapshot::default());
    }

    #[test]
    fn corrompido_carrega_home() {
        let root = TempDir::new("corrompido");
        let dir = session_dir(root.path());
        let store = SessionStore::open(&dir).expect("open");
        std::fs::write(dir.join("camera.json"), b"{ nao eh json valido").expect("escreve lixo");
        assert_eq!(
            store.load_camera(),
            CameraSnapshot::default(),
            "JSON corrompido → home, sem panic"
        );
    }

    #[test]
    fn zoom_insano_carrega_home() {
        let root = TempDir::new("insano");
        let store = SessionStore::open(session_dir(root.path())).expect("open");
        // zoom 0 quebraria screen_to_world (÷0) → rejeitado para home (is_sane).
        store
            .save_camera(&CameraSnapshot {
                pan: (10.0, 20.0),
                zoom: 0.0,
            })
            .expect("save");
        assert_eq!(
            store.load_camera(),
            CameraSnapshot::default(),
            "snapshot insano (zoom 0) → home"
        );
    }

    #[test]
    fn ultimo_save_vence() {
        let root = TempDir::new("ultimo");
        let store = SessionStore::open(session_dir(root.path())).expect("open");
        store
            .save_camera(&CameraSnapshot {
                pan: (1.0, 1.0),
                zoom: 0.5,
            })
            .expect("save 1");
        let ultimo = CameraSnapshot {
            pan: (-9.0, 9.0),
            zoom: 2.0,
        };
        store.save_camera(&ultimo).expect("save 2");
        assert_eq!(store.load_camera(), ultimo, "último save sobrescreve");
    }

    /// Guardião do ADR 0029 §3: salvar a câmera NÃO adiciona nenhum evento ao log do
    /// Espaço (câmera é sessão local, fora da stream) — e ainda assim persiste.
    #[test]
    fn guardiao_camera_nao_entra_no_event_log() {
        let root = TempDir::new("guardiao");
        let mut store = EventStore::open(Workspace::events_dir(root.path())).expect("open events");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "X".into(),
                focus_preset: String::new(),
            })
            .expect("append");
        let antes = store.event_count().expect("count antes");

        let sess = SessionStore::open(session_dir(root.path())).expect("open sess");
        let cam = CameraSnapshot {
            pan: (120.0, -40.0),
            zoom: 1.5,
        };
        sess.save_camera(&cam).expect("save camera");

        let depois = store.event_count().expect("count depois");
        assert_eq!(
            antes, depois,
            "salvar câmera NÃO adiciona evento ao log (ADR 0029 §3)"
        );
        assert_eq!(
            sess.load_camera(),
            cam,
            "mas a câmera persiste fora da stream"
        );
    }

    #[test]
    fn is_sane_rejeita_valores_degenerados() {
        // Cada condição de `is_sane` exercitada diretamente (não-vacuosidade completa):
        // remover qualquer cláusula faz um destes casos passar como são → o teste falha.
        let home = CameraSnapshot::default();
        assert!(home.is_sane(), "home é são");
        for ruim in [
            CameraSnapshot {
                pan: (f64::NAN, 0.0),
                zoom: 1.0,
            },
            CameraSnapshot {
                pan: (0.0, f64::INFINITY),
                zoom: 1.0,
            },
            CameraSnapshot {
                pan: (0.0, 0.0),
                zoom: f64::NAN,
            },
            CameraSnapshot {
                pan: (0.0, 0.0),
                zoom: f64::INFINITY,
            },
            CameraSnapshot {
                pan: (0.0, 0.0),
                zoom: 0.0,
            },
            CameraSnapshot {
                pan: (0.0, 0.0),
                zoom: -1.5,
            },
        ] {
            assert!(!ruim.is_sane(), "insano rejeitado: {ruim:?}");
        }
    }

    #[test]
    fn load_zoom_negativo_carrega_home() {
        // JSON VÁLIDO mas insano (zoom negativo, editado à mão) → o load aplica is_sane → home.
        let root = TempDir::new("zoomneg");
        let dir = session_dir(root.path());
        let store = SessionStore::open(&dir).expect("open");
        std::fs::write(dir.join("camera.json"), br#"{"pan":[5.0,5.0],"zoom":-2.0}"#)
            .expect("write");
        assert_eq!(store.load_camera(), CameraSnapshot::default());
    }

    #[test]
    fn missing_field_json_carrega_home() {
        // Campo `zoom` faltando (arquivo truncado/editado) → parse falha → home, sem panic.
        let root = TempDir::new("missing");
        let dir = session_dir(root.path());
        let store = SessionStore::open(&dir).expect("open");
        std::fs::write(dir.join("camera.json"), br#"{"pan":[5.0,5.0]}"#).expect("write");
        assert_eq!(store.load_camera(), CameraSnapshot::default());
    }

    #[test]
    fn save_nao_deixa_tmp_residual() {
        // Após o save, o tmp foi renomeado — nenhum `camera.json.tmp*` sobra no dir.
        let root = TempDir::new("notmp");
        let dir = session_dir(root.path());
        let store = SessionStore::open(&dir).expect("open");
        store
            .save_camera(&CameraSnapshot {
                pan: (3.0, 4.0),
                zoom: 1.25,
            })
            .expect("save");
        let residual: Vec<_> = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(residual.is_empty(), "tmp residual: {residual:?}");
    }

    #[test]
    fn round_trip_preserva_precisao_f64() {
        // serde_json round-trips f64 EXATAMENTE (ryu): nada de perda de casas na persistência.
        let root = TempDir::new("precisao");
        let store = SessionStore::open(session_dir(root.path())).expect("open");
        let cam = CameraSnapshot {
            pan: (0.1 + 0.2, 1.0 / 3.0),
            zoom: 1.234_567_890_123_456_7,
        };
        store.save_camera(&cam).expect("save");
        assert_eq!(store.load_camera(), cam, "f64 persiste sem perder precisão");
    }

    #[test]
    fn conversao_f32_para_f64_e_volta_e_identidade() {
        // A costura converte Camera(f32) ↔ CameraSnapshot(f64). f32 ⊂ f64 (IEEE 754):
        // f32→f64→f32 é a IDENTIDADE — nenhum zoom/pan muta no trajeto (refuta a hipótese
        // de arredondamento; o "1.9999999→2.0" só ocorreria com um f64 que NÃO veio de f32).
        for v in [0.4_f32, 1.0, 2.0, 0.000_1, 1.999_999_9, f32::MIN_POSITIVE] {
            assert_eq!(v as f64 as f32, v, "f32→f64→f32 identidade p/ {v}");
        }
    }
}
