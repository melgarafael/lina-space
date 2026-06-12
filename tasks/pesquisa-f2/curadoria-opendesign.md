# Curadoria OpenDesign × territórios do Lina — garimpo dos 152 design systems

> **Data:** 2026-06-12 · **Garimpeiro:** Maestro (Terminal B) · **Fonte:** clone local `einstein workspace/open-design/design-systems/` (152 DESIGN.md, headers lidos 1 a 1)
> **Regra herdada da pesquisa (D1 §III):** design systems de MARCA alheia (Apple/Stripe/Linear/…) NÃO entram na curadoria de onboarding (mimetismo + risco de trademark que o leigo não entende) — servem só como **estudo de mecanismo**. A curadoria embarcável usa os SEM marca (estilos/genéricos/originais).
> **Uso:** (a) referência imediata para os protótipos F2-0-D e o design system do Lina; (b) shortlist da curadoria local vendorizada do onboarding (F2-4-5).

## I. Afinidade por território

### T1 — "Instrumento de Estúdio" (cor semântica · viewer vivo · flat honesto · mono-com-calor)
| Sistema | Marca? | Por quê tem afinidade |
|---|---|---|
| **mission-control** ⭐ | não | "Dark command center, amber telemetry, monospace precision. Functional clarity above all else" — é o mecanismo T1 quase literal (âmbar=trabalhando!) |
| **hud** ⭐ | não | "Zero ambiguity at speed" — cor fosforescente como código semântico; legibilidade de relance é o job do canvas |
| **voltagent** | semi (projeto OSS) | "Void-black, emerald accent, terminal-native" — terminal-nativo com 1 acento significante |
| **ollama** | semi | "Terminal-first, monochrome simplicity" — a ilha-terminal honesta |
| **huggingface** | sim (estudo) | "Monospace identity, cheerful and dense" — prova que mono+calor coexistem (o tempero anti-frieza do T1) |
| **warp** | sim (estudo) | Block-based command UI — referência direta de card-de-terminal |
| **github** | sim (estudo) | "Functional density" — densidade governável sem cosmética |
| **agentic** | não | Fluxos de delegação a agentes com "minimal controls, clear outcomes" — o conteúdo do nosso canvas |

### T2 — "Oficina de Precisão" (monocromo · 1 acento · chrome que recua)
| Sistema | Marca? | Por quê |
|---|---|---|
| **vercel** | sim (estudo) | "Black and white precision" — o cânone do território |
| **linear-app** | sim (estudo) | "Ultra-minimal, precise" — o método (nunca o look 2021) |
| **refined** ⭐ | não | "Elegant serif + understated palettes" — T2 embarcável sem marca |
| **clean** / **sleek** / **minimal** | não | Cluster genérico do território (escolher 1 para a curadoria — `sleek` é o mais completo) |
| **openai** / **x-ai** / **tesla** / **bugatti** / **hashicorp** | sim (estudo) | Variações de subtração radical/monocromo austero |
| **resend** / **cal** | sim (estudo) | Minimal-dark com acentos mono; developer-clean |
| **shadcn** | semi | ⚠️ útil como mecânica de componentes, mas é O default-de-IA que a doutrina manda evitar como estética |

### T3 — "Ateliê Caloroso" (papel quente · editorial · ilhas dark · celebração contida)
| Sistema | Marca? | Por quê |
|---|---|---|
| **atelier-zero** ⭐ | não | "Warm paper canvas… tiny editorial annotations" — o T3 de altíssimo craft; melhor peça do repo |
| **kami** ⭐ | não | "Warm parchment, ink-blue accent, serif-led" — print-quality, multilíngue por design |
| **warm-editorial** ⭐ | não | "Terracotta on warm off-white paper" — starter oficial do clima T3 |
| **paper** | não | Textura de papel + serif/sans limpa — base neutra do território |
| **editorial** / **publication** / **modern** | não | Cluster editorial genérico (grids de revista, serif refinada) |
| **notion** | sim (estudo) | "Warm minimalism, serif headings" — prova consumer do warm-light em ferramenta de trabalho |
| **claude** | sim (estudo) | "Warm terracotta, clean editorial" — calor + IA sem frieza |
| **wired** | sim (estudo) | "Paper-white broadsheet + mono kickers" — a PONTE T3↔T1 (editorial com espinha técnica) |
| **cafe** / **friendly** | não | Calor acolhedor; risco de virar "fofo demais" — só como tempero |
| **mastercard** / **starbucks** | sim (estudo) | Cream-canvas comercial (estudo de warm em escala) |

### T4 — "Sala de Controle" (dark profundo · denso · telemetria)
| Sistema | Marca? | Por quê |
|---|---|---|
| **trading-terminal** ⭐ | não | "Bloomberg-style… readable at a glance from two meters away" — o T4 canônico, e a frase-critério é ótima |
| **mission-control** | não | Dual T1/T4 — a versão governada dele é T1; a densa é T4 |
| **hud** | não | Idem dual |
| **sentry** / **kraken** / **dashboard** / **perplexity** | sim/semi (estudo) | Dark data-dense com hierarquia de informação real |
| **cisco** / **nvidia** | sim (estudo) | Dark técnico enterprise |
| ⚠️ **cosmic** / **neon** | não | Sci-fi/neon — é o slop que o T4 vira quando mal feito; usar como ANTI-referência |

## II. Shortlist da curadoria embarcável (F2-4-5 — sem marca, vendorizável, Apache-2.0)

**Núcleo (8):** `mission-control` · `hud` · `trading-terminal` · `atelier-zero` · `kami` · `warm-editorial` · `refined` · `sleek`
**Complemento opcional (4):** `paper` · `editorial` · `bento` (estrutura modular) · `spacious` (respiro) — e `default` (Neutral Modern) como fallback honesto do picker.
**Formato:** cada um já traz `DESIGN.md + design-tokens.json + components.html + manifest` — vendorizar a pasta inteira dos escolhidos; os territórios do Lina (T1-T4) entram como DESIGN.md PRÓPRIOS no mesmo formato, no topo do picker.

## III. Excluídos com motivo (não re-garimpar)
- **Marcas (≈70 sistemas):** airbnb, apple, stripe, figma, spotify, nike, bmw*, ferrari, lamborghini, discord, slack, etc. — mimetismo/trademark (decisão D1 §III). Ficam no clone como estudo.
- **Vocabulário banido pela doutrina:** `glassmorphism`, `gradient`, `neon`, `claymorphism`, `neumorphism` (morphism-cosmético = default de IA), `cursor`/`lovable` (gradient-dev genérico).
- **Fora do job do Lina:** e-commerce/retail, automotive, fintech-trading (exceto trading-terminal como T4), social/messaging, jogos/retro (pacman, tetris, dithered, vintage, 8-bit) — divertidos, errados para estação de trabalho leiga.
- **`mono`:** "hacker-chic matrix" — sedutor e ERRADO para o nosso leigo (é a estética que intimida; a D1-A3 pós-refutação manda medir, não assumir).

## IV. Achado de bônus
O formato do repo confirma a recomendação da pesquisa com folga: `design-tokens.json` por sistema é exatamente o "valores como dados" da arquitetura D2 — os DESIGN.md dos territórios do Lina podem nascer JÁ com seu design-tokens.json irmão, que o futuro `lina-theme` consome.
