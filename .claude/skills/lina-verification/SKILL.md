---
name: lina-verification
description: >-
  Nunca declarar trabalho "pronto" sem evidência OBSERVADA. Use SEMPRE que estiver prestes a
  afirmar que algo está concluído, funcionando ou corrigido, e ANTES de entregar/commitar/marcar
  item como feito — gatilhos: "está pronto?", "isso funciona?", "acho que resolvi", "deve
  funcionar", "terminei", "pode marcar como concluído?". Exige a prova antes da afirmação: rodou
  de fato, leu o output, viu o comportamento — e a régua "um staff engineer aprovaria?". Espelha
  a calibração de enforcement da rubrica anti-slop (§6.2: existência de mecanismo ≠ propriedade
  vale; PASS é afirmação com evidência, não ausência de objeção). NÃO é para revisar entrega de
  colega (lina-cold-review) — é para você não se auto-enganar. Agnóstica de CLI.
---

# Lina Verification — evidência antes da afirmação

O modo de falha mais caro de um agente é dizer "funciona" sem ter visto funcionar. "Deve funcionar"
não é evidência — é suposição com fantasia de fato. Esta doutrina é o freio.

## 1. A regra dura
**Nunca afirme "funciona", "passou", "pronto" ou "corrigido" sem evidência OBSERVADA:**
- **Rodou** o comando/teste/app de verdade (não imaginou que rodaria).
- **Leu** o output real (não assumiu o que ele diria — stdout glitchado? redirecione p/ arquivo e leia).
- **Viu** o comportamento (especialmente em UI: teste verde headless ≠ o usuário enxerga a feature).

## 2. A régua
Antes de mostrar, pergunte: **"um staff engineer aprovaria isto?"** Se a resposta é não — ou "não sei" —
**itere antes de mostrar**. Honestidade acima de conveniência: se o teste falhou, diga com o output;
se um passo foi pulado, diga; só afirme "feito e verificado" quando viu o comportamento.

## 3. Existência ≠ enforcement (espelha rubrica §6.2)
"O mecanismo existe" não é "a propriedade vale". Para cada critério, pergunte: *está ENFORCED no que
eu entreguei, ou delegado a convenção/runtime/SO não-verificado?* Código correto que não **garante** o
critério ainda falha. **PASS/pronto é uma afirmação com evidência**, não a ausência de objeção numa
leitura rasa.

## 4. O que conta como evidência (checklist)
- [ ] Comando/teste rodado e **exit code** lido direto (pipe pode mascarar; cache de lint engana).
- [ ] Output real lido, não suposto. · [ ] Comportamento visto (UI: na tela, não só no log).
- [ ] O critério de aceite específico foi exercido — não um proxy. · [ ] "Staff engineer aprovaria?" = sim.
- [ ] Ao reportar: distingo o que VI do que ASSUMO; ressalvo o que não consegui verificar.

## Notas por CLI
Corpo agnóstico (inv#3): "rodar e ler o output" usa a ferramenta de shell/execução do seu CLI; o
princípio é idêntico em todos. Em Espaço Lina, o veredito de prontidão vira evento (inv#4). Se o CLI
não ativar por `description`, o bootstrap turno-0 carrega explícito.
