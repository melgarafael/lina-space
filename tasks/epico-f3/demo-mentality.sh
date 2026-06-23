#!/bin/bash
# Demo do painel "Como o [papel] pensa" (F3-3 gate h visual).
# Abre um Lina de TESTE isolado em /tmp/lina-mentality-demo — NÃO toca a sua equipe de produção.
set -e
cd "/Users/rafaelmelgaco/einstein workspace/lina-space"

echo "1/2 · Populando o Lina de teste com 2 lições de exemplo (papel: Desenvolvedor)..."
rm -rf /tmp/lina-mentality-demo
cargo run -q -p lina-core --bin mentality_demo_seed -- /tmp/lina-mentality-demo/.lina

echo
echo "2/2 · Abrindo o Lina de TESTE (janela nova, isolada — feche quando terminar)."
echo "      Procure o terminal \"Desenvolvedor\" e o painel \"Como o Desenvolvedor pensa\"."
echo
# LINA_DEMO_ROLE=DEVELOPER: o app sobe UM terminal vivo de papel Desenvolvedor (em vez do par A/B),
# para o painel achar um papel VIVO que case as crenças semeadas acima. As crenças e o papel do
# terminal vivo são o MESMO ("DEVELOPER") — sem isso o painel some (gate h nunca rodava).
LINA_WS_ROOT=/tmp/lina-mentality-demo LINA_DEMO=1 LINA_DEMO_ROLE=DEVELOPER exec app/lina-gpui/target/debug/lina-gpui
