#!/usr/bin/env python3
"""lina_watch — observador legível do event log do Lina Space (F1-0-1).

Promoção a ferramenta versionada do observador efêmero ``/tmp/lina_watch.py``
descrito em ``docs/DIRECIONAMENTO-coordenacao-a2a.md`` §4.3. É 100% LEITOR do
``log.jsonl`` (espelho JSONL do EventStore — `.lina/events/log.jsonl`): nenhum
canal paralelo de verdade, nenhuma escrita no workspace (invariante #4 + âncora
Event Store do doc 01 §3).

Uso:
    # acompanhar um workspace vivo (o jeito da observação de 14h):
    tail -f "$WS/.lina/events/log.jsonl" | python3 tools/lina_watch.py

    # idem, sem tail externo:
    python3 tools/lina_watch.py --follow "$WS/.lina/events/log.jsonl"

    # re-ler um log inteiro (replay legível):
    python3 tools/lina_watch.py --replay "$WS/.lina/events/log.jsonl"

    # análise de atropelamento (entregas ao MESMO alvo com <2s de intervalo):
    python3 tools/lina_watch.py --stats --replay "$WS/.lina/events/log.jsonl"

    # testes embutidos (log sintético; exit 0/1):
    python3 tools/lina_watch.py --self-test

Formato de cada linha do log (EventRecord do events.rs):
    {"seq": u64, "ts": u64_ms, "kind": str, "version": u32, "payload": {...}}

O mapeamento NodeId→nome vem de NodeRenamed{node,name} e Handshake{node,name}
(o app emite NodeRenamed ao nomear o nó do canvas; o harness headless da
baseline emite o mesmo evento ao registrar cada terminal).
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from collections import defaultdict

# Kinds ruidosos pulados por padrão (DIRECIONAMENTO §4.3: ~338 TokenUsageReported/min).
DEFAULT_SKIP = {"TokenUsageReported"}

# Limiar de atropelamento (DIRECIONAMENTO §4.5: "retornos a A com < 2s de intervalo").
COLLISION_THRESHOLD_MS = 2000


class NameMap:
    """NodeId → nome legível, alimentado pelos eventos do próprio log."""

    def __init__(self) -> None:
        self._names: dict[str, str] = {}

    def feed(self, kind: str, payload: dict) -> None:
        if kind == "NodeRenamed":
            node, name = payload.get("node"), payload.get("name")
            if node is not None and name:
                self._names[str(node)] = str(name)
        elif kind == "Handshake":
            node, name = payload.get("node"), payload.get("name")
            if node is not None and name:
                self._names.setdefault(str(node), str(name))

    def name(self, node) -> str:
        if node is None:
            return "?"
        key = str(node)
        if key in self._names:
            return self._names[key]
        # NodeId desconhecido: encurta UUID p/ legibilidade sem perder unicidade visual.
        return key[:8] if len(key) > 8 else key


def fmt_ts(ts_ms) -> str:
    try:
        t = time.localtime(int(ts_ms) / 1000.0)
        return time.strftime("%H:%M:%S", t) + f".{int(ts_ms) % 1000:03d}"
    except (ValueError, TypeError, OverflowError):
        return str(ts_ms)


def fmt_event(rec: dict, names: NameMap) -> str:
    """Uma linha legível por evento: [seq] hh:mm:ss kind from→to intent/reason."""
    seq = rec.get("seq", "?")
    ts = fmt_ts(rec.get("ts", 0))
    kind = rec.get("kind", "?")
    p = rec.get("payload") or {}

    if kind == "MessageRouted":
        frm = names.name(p.get("from"))
        to = p.get("to", "?")
        intent = p.get("intent", "?")
        hops = p.get("hops", 0)
        return f"[{seq}] {ts} ROUTED    {frm} → {to} intent={intent} hops={hops} id={p.get('id', '?')}"
    if kind == "MessageDelivered":
        to = names.name(p.get("to"))
        return f"[{seq}] {ts} DELIVERED → {to} id={p.get('id', '?')}"
    if kind == "RouteBlocked":
        frm = p.get("from", "?")  # nomes crus no evento (cobre unknown_sender)
        to = p.get("to", "?")
        reason = p.get("reason", "?")
        return f"[{seq}] {ts} BLOCKED   {frm} → {to} reason={reason} id={p.get('id', '?')}"
    if kind == "BusMessageSent":
        frm = names.name(p.get("from"))
        return f"[{seq}] {ts} BUS       {frm} → {p.get('to', '?')} id={p.get('id', '?')}"
    if kind in ("NodeRenamed", "Handshake"):
        return f"[{seq}] {ts} {kind:<9} {names.name(p.get('node'))}"
    if kind == "TerminalSpawned":
        return f"[{seq}] {ts} SPAWNED   {names.name(p.get('node'))} cli={p.get('cli', '?')}"
    if kind in ("AwaitOpened", "AwaitClosed"):
        return f"[{seq}] {ts} {kind:<9} {json.dumps(p, ensure_ascii=False)}"
    # Demais kinds: linha compacta com o payload (lifecycle/plan/custódia/etc.).
    return f"[{seq}] {ts} {kind:<9} {json.dumps(p, ensure_ascii=False)}"


def parse_line(line: str):
    line = line.strip()
    if not line:
        return None
    try:
        rec = json.loads(line)
    except json.JSONDecodeError:
        return None
    return rec if isinstance(rec, dict) and "kind" in rec else None


class CollisionStats:
    """Atropelamento: entregas consecutivas ao MESMO alvo com Δt < limiar.

    Casa cada MessageDelivered com o MessageRouted do mesmo id (p/ saber o
    remetente) e agrupa por alvo, na ordem do log (seq).
    """

    def __init__(self, threshold_ms: int = COLLISION_THRESHOLD_MS) -> None:
        self.threshold_ms = threshold_ms
        self.routed_from: dict[str, str] = {}  # msg id → nome do remetente
        self.deliveries: dict[str, list] = defaultdict(list)  # alvo → [(seq, ts, id, from)]

    def feed(self, rec: dict, names: NameMap) -> None:
        kind = rec.get("kind")
        p = rec.get("payload") or {}
        if kind == "MessageRouted":
            self.routed_from[p.get("id", "")] = names.name(p.get("from"))
        elif kind == "MessageDelivered":
            to = names.name(p.get("to"))
            mid = p.get("id", "?")
            frm = self.routed_from.get(mid, "?")
            self.deliveries[to].append((rec.get("seq", 0), rec.get("ts", 0), mid, frm))

    def collisions(self) -> dict[str, list]:
        """Por alvo: lista de pares consecutivos (anterior, atual, delta_ms) abaixo do limiar."""
        out: dict[str, list] = {}
        for target, rows in self.deliveries.items():
            rows = sorted(rows)  # por seq
            pairs = []
            for prev, cur in zip(rows, rows[1:]):
                delta = int(cur[1]) - int(prev[1])
                if 0 <= delta < self.threshold_ms:
                    pairs.append((prev, cur, delta))
            if pairs:
                out[target] = pairs
        return out

    def render(self) -> str:
        lines = ["== lina_watch --stats: entregas ao mesmo alvo =="]
        if not self.deliveries:
            lines.append("(nenhum MessageDelivered no log)")
            return "\n".join(lines)
        for target, rows in sorted(self.deliveries.items()):
            rows = sorted(rows)
            lines.append(f"alvo {target}: {len(rows)} entregas")
            for prev, cur in zip(rows, rows[1:]):
                delta = int(cur[1]) - int(prev[1])
                flag = "  ⚠ COLISÃO <2s" if 0 <= delta < self.threshold_ms else ""
                lines.append(
                    f"  seq {prev[0]}→{cur[0]}  Δ {delta} ms  "
                    f"({prev[3]} id={prev[2]} → {cur[3]} id={cur[2]}){flag}"
                )
        col = self.collisions()
        total_pairs = sum(len(v) for v in col.values())
        lines.append(f"TOTAL de pares em colisão (<{self.threshold_ms} ms): {total_pairs}")
        return "\n".join(lines)


def stream(fh, skip: set, stats: CollisionStats | None, out=sys.stdout) -> None:
    names = NameMap()
    for line in fh:
        rec = parse_line(line)
        if rec is None:
            continue
        names.feed(rec["kind"], rec.get("payload") or {})
        if stats is not None:
            stats.feed(rec, names)
        if rec["kind"] in skip:
            continue
        print(fmt_event(rec, names), file=out, flush=True)


def follow(path: str, skip: set, stats: CollisionStats | None) -> None:
    """tail -f embutido (poll simples; suficiente p/ observação)."""
    names = NameMap()
    with open(path, "r", encoding="utf-8") as fh:
        while True:
            line = fh.readline()
            if not line:
                time.sleep(0.2)
                continue
            rec = parse_line(line)
            if rec is None:
                continue
            names.feed(rec["kind"], rec.get("payload") or {})
            if stats is not None:
                stats.feed(rec, names)
            if rec["kind"] in skip:
                continue
            print(fmt_event(rec, names), flush=True)


# ───────────────────────────── self-test (log sintético) ─────────────────────────────

SYNTHETIC = [
    {"seq": 1, "ts": 1000, "kind": "NodeRenamed", "version": 1,
     "payload": {"node": "aaaa-bbbb-cccc", "name": "Maestro"}},
    {"seq": 2, "ts": 1100, "kind": "NodeRenamed", "version": 1,
     "payload": {"node": "dddd-eeee-ffff", "name": "W1"}},
    {"seq": 3, "ts": 1200, "kind": "TokenUsageReported", "version": 1,
     "payload": {"node": "dddd-eeee-ffff", "tokens": 42}},
    {"seq": 4, "ts": 2000, "kind": "MessageRouted", "version": 1,
     "payload": {"id": "msg_1", "from": "dddd-eeee-ffff", "to": "@Maestro",
                 "intent": "ask", "hops": 0}},
    {"seq": 5, "ts": 2400, "kind": "MessageDelivered", "version": 1,
     "payload": {"id": "msg_1", "to": "aaaa-bbbb-cccc"}},
    {"seq": 6, "ts": 2500, "kind": "MessageRouted", "version": 1,
     "payload": {"id": "msg_2", "from": "dddd-eeee-ffff", "to": "@Maestro",
                 "intent": "ask", "hops": 0}},
    {"seq": 7, "ts": 3000, "kind": "MessageDelivered", "version": 1,
     "payload": {"id": "msg_2", "to": "aaaa-bbbb-cccc"}},
    {"seq": 8, "ts": 9999, "kind": "RouteBlocked", "version": 1,
     "payload": {"id": "msg_3", "reason": "no_target", "from": "W1", "to": "@Ghost"}},
]


def self_test() -> int:
    import io

    failures: list[str] = []

    def check(cond: bool, label: str) -> None:
        if not cond:
            failures.append(label)

    # 1) Mapeamento de nomes + formatação legível from/to/intent/reason.
    names = NameMap()
    out = io.StringIO()
    stats = CollisionStats()
    fh = io.StringIO("\n".join(json.dumps(r) for r in SYNTHETIC))
    stream(fh, DEFAULT_SKIP, stats, out=out)
    text = out.getvalue()

    check("Maestro" in text, "nome do alvo mapeado (NodeRenamed)")
    check("W1 → @Maestro" in text, "ROUTED imprime from legível → to")
    check("intent=ask" in text, "ROUTED imprime intent")
    check("reason=no_target" in text, "BLOCKED imprime reason")
    check("DELIVERED → Maestro" in text, "DELIVERED imprime alvo legível")
    check("TokenUsageReported" not in text, "TokenUsageReported é pulado por padrão")

    # 2) Estatística de colisão: as 2 entregas ao Maestro têm Δ=600ms < 2000ms.
    col = stats.collisions()
    check("Maestro" in col, "colisão detectada no alvo Maestro")
    if "Maestro" in col:
        pairs = col["Maestro"]
        check(len(pairs) == 1, "exatamente 1 par em colisão")
        check(pairs[0][2] == 600, f"delta calculado 600ms (obtido {pairs[0][2]})")
        check(pairs[0][0][0] == 5 and pairs[0][1][0] == 7, "par cita os seq corretos (5→7)")

    # 3) Render do stats inclui seq e delta (o formato que a baseline cita).
    rendered = stats.render()
    check("seq 5→7" in rendered, "render cita seq do par")
    check("Δ 600 ms" in rendered, "render cita o delta em ms")
    check("COLISÃO" in rendered, "render marca a colisão")

    # 4) Linha inválida não derruba o parser.
    check(parse_line("não é json") is None, "linha inválida vira None")
    check(parse_line("") is None, "linha vazia vira None")

    if failures:
        print("SELF-TEST: FALHOU")
        for f in failures:
            print(f"  ✗ {f}")
        return 1
    print("SELF-TEST: OK (14 asserções)")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--follow", metavar="LOG", help="acompanha o log.jsonl (tail -f embutido)")
    ap.add_argument("--replay", metavar="LOG", help="lê o log inteiro e sai")
    ap.add_argument("--stats", action="store_true",
                    help="ao final (--replay), imprime análise de atropelamento por alvo")
    ap.add_argument("--all", action="store_true",
                    help="não pula nenhum kind (inclui TokenUsageReported)")
    ap.add_argument("--self-test", action="store_true", help="roda os testes embutidos")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    skip = set() if args.all else set(DEFAULT_SKIP)
    stats = CollisionStats() if args.stats else None

    if args.replay:
        with open(args.replay, "r", encoding="utf-8") as fh:
            stream(fh, skip, stats)
        if stats is not None:
            print()
            print(stats.render())
        return 0

    if args.follow:
        try:
            follow(args.follow, skip, stats)
        except KeyboardInterrupt:
            if stats is not None:
                print()
                print(stats.render())
        return 0

    # default: stdin (composição com `tail -f … |`)
    try:
        stream(sys.stdin, skip, stats)
    except KeyboardInterrupt:
        pass
    if stats is not None:
        print()
        print(stats.render())
    return 0


if __name__ == "__main__":
    sys.exit(main())
