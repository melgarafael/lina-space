#!/usr/bin/env bash
# codesign-dev-setup.sh — cria (1x por máquina) um certificado LOCAL de assinatura de código
# para dar identidade ESTÁVEL ao Lina.app perante o TCC do macOS.
#
# POR QUÊ: o Lina roda SEM assinatura. Cada rebuild gera um binário com hash novo → o macOS
# trata como "app novo desconhecido" → APAGA todas as permissões já concedidas (pastas
# protegidas Downloads/Desktop/Documents, microfone). Com um certificado FIXO, a "Designated
# Requirement" que o TCC guarda passa a ser baseada no CERTIFICADO (estável), não no hash do
# binário → as permissões persistem entre rebuilds.
#
# NÃO é distribuição/notarização (Apple Developer ID). É um cert self-signed SÓ desta máquina,
# num keychain dedicado com senha conhecida (não usa sua senha de login do Mac nem sudo).
# A decisão do fundador de distribuir SEM assinatura (make-app.sh) continua valendo: este cert
# é só local; quem não tiver o cert gera bundle sem assinatura, como antes.
#
# Idempotente: recria do zero a cada execução. Reverter: `security delete-keychain lina-codesign.keychain`.
#
# Depois deste setup, todo build (make-app.sh / dev-watch.sh / bundle-launcher.sh) re-assina
# automaticamente via packaging/codesign-dev.sh.
set -euo pipefail

CERT_CN="Lina Dev (local TCC)"
KC="lina-codesign.keychain"
KC_PASS="lina-dev-local"   # senha do keychain DEDICADO (não é segredo de valor; cert é só p/ TCC local)
P12_PASS="linap12"

echo "==> [1/5] gerando certificado self-signed (codeSigning) com openssl"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cat > "$WORK/openssl.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = v3
prompt = no
[dn]
CN = ${CERT_CN}
[v3]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
EOF
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout "$WORK/key.pem" -out "$WORK/cert.pem" \
  -days 3650 -config "$WORK/openssl.cnf" >/dev/null 2>&1
# OpenSSL 3 default usa cifras que o `security` da Apple não lê (MAC verification failed).
# -legacy + PBE-SHA1-3DES + -macalg sha1 emitem o p12 no formato que o macOS importa.
openssl pkcs12 -export -inkey "$WORK/key.pem" -in "$WORK/cert.pem" \
  -out "$WORK/id.p12" -passout "pass:${P12_PASS}" -name "$CERT_CN" \
  -legacy -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 >/dev/null 2>&1

echo "==> [2/5] (re)criando keychain dedicado: $KC"
security delete-keychain "$KC" 2>/dev/null || true
security create-keychain -p "$KC_PASS" "$KC"
security set-keychain-settings "$KC"   # sem auto-lock por tempo

echo "==> [3/5] adicionando ao search list do usuário (preserva os existentes)"
EXISTING="$(security list-keychains -d user | sed 's/[" ]//g')"
# shellcheck disable=SC2086
security list-keychains -d user -s "$KC" $EXISTING

echo "==> [4/5] importando identidade e liberando o codesign (sem prompt)"
security unlock-keychain -p "$KC_PASS" "$KC"
security import "$WORK/id.p12" -k "$KC" -P "$P12_PASS" -A -T /usr/bin/codesign -T /usr/bin/security >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KC_PASS" "$KC" >/dev/null 2>&1

echo "==> [5/5] verificando"
if security find-certificate -c "$CERT_CN" "$KC" >/dev/null 2>&1; then
  echo "    OK — certificado '$CERT_CN' instalado no keychain '$KC'."
  echo "    Identidades de codesign visíveis:"
  security find-identity -p codesigning "$KC" | sed 's/^/    /'
else
  echo "    ERRO: certificado não encontrado após import." >&2
  exit 1
fi
echo "PRONTO. Agora rode um build (ou reabra a Lina) — os binários serão assinados automaticamente."
