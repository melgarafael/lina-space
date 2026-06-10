# GABARITO — fixture de slop plantado

> **Não entregar este arquivo ao revisor.** É a chave de correção do harness de validação.
> O cold-review recebe SÓ `Hero.tsx`, `hero.css` e `CRITERIOS.md` — nunca este gabarito.

## Veredito esperado: **FAIL**

Há **7 violações ALTA** (qualquer uma já força FAIL pela regra da rubrica §1) e score muito abaixo de 80.

## Violações plantadas (N = 14)

| # | ID rubrica | Sev. | Arquivo | Onde | Evidência |
|---|---|---|---|---|---|
| 1 | DES-1 | ALTA | hero.css | `.hero` | `font-family: "Inter"` como escolha estética (fonte default banida). |
| 2 | DES-2 | ALTA | hero.css | `.hero` | `linear-gradient(... #7c3aed ... #a855f7 ... #ffffff)` — gradiente "AI purple". |
| 3 | DES-3 | MÉDIA | hero.css | `.hero` | `backdrop-filter: blur(12px)` + `rgba(255,255,255,0.1)` — glassmorphism decorativo. |
| 4 | DES-4 | ALTA | (artefato) | — | Nenhuma direção estética declarada (não há DIRECAO-ESTETICA; CRITERIOS não declara). |
| 5 | COD-1 | MÉDIA | Hero.tsx | `handleData`, `handleClick` | Nomes genéricos que não dizem o quê. |
| 6 | COD-2 | MÉDIA | Hero.tsx | comentários `// Hero component`, `// handle the data`, `// handle the click` | Comentários óbvios que repetem o código. |
| 7 | COD-3 | ALTA | Hero.tsx | `props: any`, `(data: any)` ×2 | Cast/escape de tipo `any`. |
| 8 | COD-4 | ALTA | Hero.tsx | `catch (e) {}` ×2 | Erro engolido (catch vazio). |
| 9 | COD-5 | MÉDIA | Hero.tsx | `handleData` vs `handleClick` | Duplicação — corpo idêntico copiado. |
| 10 | COP-1 | ALTA | Hero.tsx | `"Welcome to the Future"` | Filler genérico. |
| 11 | COP-2 | MÉDIA | Hero.tsx | `"Unlock your potential with our cutting-edge, innovative solution."` | Genericidade de template. |
| 12 | COP-3 | MÉDIA | Hero.tsx | `"Click here"` | CTA sem voz. |
| 13 | ARQ-1 | ALTA | Hero.tsx | `class WidgetFactory<T>` | Abstração especulativa (`<T>` genérico, um único uso). |
| 14 | ARQ-2 | MÉDIA | Hero.tsx | `defaultConfig`/`config` | Sistema de config para uma hero estática — complexidade não pedida. |

ALTA (7): #1, #2, #4, #7, #8, #10, #13.

## Critério de aprovação do teste (story F1-3-2 crit. 1)

- **Detecção:** o revisor deve reportar **≥ 80% de 14 = ≥ 12** violações plantadas (casadas por ID + local).
  - Clusters contam justo: reportar "COD-2: comentários óbvios" cobre o marcador COD-2 (1 acerto), mesmo
    citando 1 das 3 ocorrências. O que importa é o marcador certo no local certo.
- **Veredito:** deve ser **FAIL**.
- Achados EXTRA além dos 14 (ex.: DES-5 border-radius/shadow genéricos como BAIXA) **não penalizam** — só
  confirmam o rigor. O que falha o teste é deixar passar marcador duro ou dar PASS.
