//! Provas headless dos critérios de aceite de F1-4-5.
//! Cada teste cita o critério que cobre; remover o mecanismo quebra o teste.

use ed25519_dalek::{Signer, SigningKey};
use lina_license::{
    activate, encode_token, ActivationError, BlockReason, LicenseClaims, LicenseState,
    LicenseStatus, Verifier, WorkspaceGate, FREE_WORKSPACE_LIMIT,
};
use rand_core::OsRng;
use std::path::PathBuf;

const NOW: u64 = 1_750_000_000;

struct Fixture {
    key: SigningKey,
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            key: SigningKey::generate(&mut OsRng),
            dir: tempfile::tempdir().expect("tempdir"),
        }
    }
    fn verifier(&self) -> Verifier {
        Verifier::with_key(self.key.verifying_key())
    }
    fn license_path(&self) -> PathBuf {
        self.dir.path().join("license.json")
    }
    fn token(&self, claims: &LicenseClaims) -> String {
        let sig = self.key.sign(&claims.canonical_bytes());
        encode_token(claims, &sig.to_bytes()).expect("token")
    }
    /// Ativa uma licença e devolve o estado carregado do disco.
    fn install_and_load(&self, claims: &LicenseClaims) -> LicenseState {
        activate(
            &self.token(claims),
            &self.license_path(),
            &self.verifier(),
            NOW,
        )
        .expect("ativação válida");
        LicenseState::load(&self.license_path(), &self.verifier())
    }
}

fn pro_claims(limit: Option<u32>, expiry: Option<u64>) -> LicenseClaims {
    LicenseClaims {
        license_id: "lic-teste-1".into(),
        tier: "pro".into(),
        workspace_limit: limit,
        entitlements: vec![],
        expiry,
        machine_id: None,
    }
}

fn assert_blocked(gate: WorkspaceGate, expected_reason: BlockReason) {
    match gate {
        WorkspaceGate::Blocked { reason, .. } => assert_eq!(reason, expected_reason),
        WorkspaceGate::Allowed => panic!("esperava bloqueio {expected_reason:?}, veio Allowed"),
    }
}

// Critério 1: PRO válida permite 2º+ Espaço; sem licença bloqueia gracioso.
#[test]
fn pro_valida_permite_segundo_espaco_e_free_bloqueia() {
    let f = Fixture::new();
    let state = f.install_and_load(&pro_claims(None, None));
    assert_eq!(state.can_create_workspace(1, NOW), WorkspaceGate::Allowed);
    assert_eq!(state.can_create_workspace(50, NOW), WorkspaceGate::Allowed);

    let free = LicenseState::load(&f.dir.path().join("inexistente.json"), &f.verifier());
    assert_eq!(*free.status(), LicenseStatus::NoLicense);
    assert_eq!(free.can_create_workspace(0, NOW), WorkspaceGate::Allowed);
    assert_blocked(
        free.can_create_workspace(FREE_WORKSPACE_LIMIT, NOW),
        BlockReason::FreeTier,
    );
}

// Critério 2: gating data-driven — workspace_limit=3 permite 3, bloqueia o 4º.
#[test]
fn limite_intermediario_tres_permite_tres_e_bloqueia_o_quarto() {
    let f = Fixture::new();
    let state = f.install_and_load(&pro_claims(Some(3), None));
    for existing in 0..3 {
        assert_eq!(
            state.can_create_workspace(existing, NOW),
            WorkspaceGate::Allowed,
            "com {existing} existentes deve permitir"
        );
    }
    assert_blocked(
        state.can_create_workspace(3, NOW),
        BlockReason::LimitReached,
    );
}

// Critério 3 (adversarial): editar tier/limit no JSON sem re-assinar ⇒ free.
#[test]
fn editar_license_json_na_mao_degrada_para_free() {
    let f = Fixture::new();
    f.install_and_load(&pro_claims(Some(1), None));

    let raw = std::fs::read_to_string(f.license_path()).expect("ler license.json");
    let adulterado = raw.replace("\"workspace_limit\": 1", "\"workspace_limit\": 999");
    assert_ne!(raw, adulterado, "o replace precisa ter efeito");
    std::fs::write(f.license_path(), adulterado).expect("gravar adulterado");

    let state = LicenseState::load(&f.license_path(), &f.verifier());
    assert_eq!(*state.status(), LicenseStatus::DegradedInvalidSignature);
    assert_blocked(state.can_create_workspace(1, NOW), BlockReason::FreeTier);
}

// Critério 4: chave assinada por keypair de terceiro não valida.
#[test]
fn chave_de_keypair_de_terceiro_nao_ativa() {
    let f = Fixture::new();
    let atacante = Fixture::new();
    let token_falso = atacante.token(&pro_claims(None, None));
    let err =
        activate(&token_falso, &f.license_path(), &f.verifier(), NOW).expect_err("não pode ativar");
    assert!(matches!(err, ActivationError::InvalidSignature));
    assert!(!f.license_path().exists(), "nada pode ter sido persistido");
}

// Critério 6: perpétua imune a relógio retrocedido/avançado.
#[test]
fn perpetua_e_imune_a_qualquer_relogio() {
    let f = Fixture::new();
    let state = f.install_and_load(&pro_claims(None, None));
    for clock in [0u64, NOW, u64::MAX] {
        assert_eq!(
            state.can_create_workspace(10, clock),
            WorkspaceGate::Allowed
        );
        assert!(!state.effective(clock).expired);
    }
}

// Critérios 6 (parte expiry) e 7: vencida degrada gracioso SÓ no ponto de
// gating, com sinal `expired` para o aviso honesto não-bloqueante.
#[test]
fn expiry_degrada_gracioso_apenas_no_ponto_de_gating() {
    let f = Fixture::new();
    let state = f.install_and_load(&pro_claims(None, Some(NOW + 100)));

    // Antes de vencer: pro pleno.
    assert_eq!(state.can_create_workspace(5, NOW), WorkspaceGate::Allowed);
    // Mesmo estado, relógio depois do expiry: o MESMO objeto degrada só
    // quando consultado com `now` posterior — não há autoridade paralela.
    let depois = NOW + 200;
    let eff = state.effective(depois);
    assert!(eff.expired, "sinal para o aviso honesto da F1-4-6");
    assert_eq!(eff.workspace_limit, Some(FREE_WORKSPACE_LIMIT));
    assert_blocked(state.can_create_workspace(1, depois), BlockReason::Expired);
    // Relógio retrocedido re-habilita (chave de aluno dentro da validade).
    assert_eq!(state.can_create_workspace(5, NOW), WorkspaceGate::Allowed);
}

// Critério 8: workspaces pré-existentes além do limite continuam ABRINDO —
// o gate só governa CRIAÇÃO; abrir nunca consulta o gate. Provamos que o
// estado degradado continua respondendo (app vivo) e só bloqueia criar.
#[test]
fn downgrade_nunca_sequestra_workspaces_existentes() {
    let f = Fixture::new();
    let state = f.install_and_load(&pro_claims(Some(2), Some(NOW + 100)));
    let depois_do_expiry = NOW + 200;
    // 5 workspaces existentes (criados na era PRO): o estado responde normal
    // (nenhum panic/travamento) e a ÚNICA consequência é bloquear criação.
    assert_blocked(
        state.can_create_workspace(5, depois_do_expiry),
        BlockReason::Expired,
    );
    let eff = state.effective(depois_do_expiry);
    assert_eq!(eff.status, LicenseStatus::Licensed, "licença segue legível");
}

// Ativação de chave vencida é recusada com erro honesto (não instala).
#[test]
fn ativar_chave_vencida_recusa_sem_instalar() {
    let f = Fixture::new();
    let token = f.token(&pro_claims(None, Some(NOW - 1)));
    let err =
        activate(&token, &f.license_path(), &f.verifier(), NOW).expect_err("vencida não ativa");
    assert!(matches!(err, ActivationError::Expired { .. }));
    assert!(!f.license_path().exists());
}

// machine_id presente é transportado mas NÃO validado (porta node-locking).
#[test]
fn machine_id_e_transportado_mas_nao_validado() {
    let f = Fixture::new();
    let mut claims = pro_claims(None, None);
    claims.machine_id = Some("maquina-de-outra-pessoa".into());
    let state = f.install_and_load(&claims);
    assert_eq!(*state.status(), LicenseStatus::Licensed);
    assert_eq!(state.can_create_workspace(3, NOW), WorkspaceGate::Allowed);
}

// JSON corrompido degrada para free legível (nunca trava o app).
#[test]
fn json_corrompido_degrada_para_free() {
    let f = Fixture::new();
    std::fs::write(f.license_path(), b"{isto nao e json").expect("gravar lixo");
    let state = LicenseState::load(&f.license_path(), &f.verifier());
    assert_eq!(*state.status(), LicenseStatus::DegradedUnreadable);
    assert_blocked(state.can_create_workspace(1, NOW), BlockReason::FreeTier);
}

// `last_validated` é metadado não-assinado: editá-lo NÃO invalida a licença
// (não é autoridade), mas os campos assinados continuam protegidos.
#[test]
fn last_validated_nao_e_autoridade() {
    let f = Fixture::new();
    f.install_and_load(&pro_claims(Some(2), None));
    let raw = std::fs::read_to_string(f.license_path()).expect("ler");
    let editado = raw.replace(&format!("{NOW}"), "12345");
    std::fs::write(f.license_path(), editado).expect("gravar");
    let state = LicenseState::load(&f.license_path(), &f.verifier());
    assert_eq!(*state.status(), LicenseStatus::Licensed);
}
