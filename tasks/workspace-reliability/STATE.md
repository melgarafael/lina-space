# Estado do loop de confiabilidade de workspaces

```yaml
status: complete
iteration: 13
green_streak: 3
same_failure_count: 0
current_invariant: A+B+C+D+E+F+G+H
current_hypothesis: >-
  As invariantes A-H permaneceram estáveis em três fixtures, no pacote reiniciado e no fluxo
  nativo observado; os dois defeitos finais de UI encontrados no dogfood têm regressão automatizada.
last_failure_fingerprint: hook-drain-observer-yield-scheduler-flake
last_red_command: /bin/zsh /private/tmp/lina-workspace-final-gates.sh
last_red_artifact: matrix 2/3 app ulimit=128; hook_drain_drop_transfers_observation failed after 8 yields
last_checker_verdict: PASS 100; nenhuma violação ou bloqueador demonstrável
```

## Evidência inicial observada

- `ulimit -n` retornou `256`.
- O processo vivo antigo `lina-gpui-bin` (PID 22793) ocupava exatamente 256 descritores,
  do FD 0 ao FD 255: 72 PTYs, 23 listeners TCP, 26 handles de event DB e 11 handles de
  scrollback DB.
- O registry real tinha 31 entradas e nove raízes `TomikOS - OpenSource*`/`TomikOS - OPen`.
- Os stores TomikOS parciais continham `WorkspaceCreated` no SQLite, mas não
  `WorkspaceIdAssigned`, roster nem cwd; vários espelhos JSONL omitiam o primeiro evento.
- A pasta real indicada pelo relato foi localizada, sem modificá-la, em
  `/Users/rafaelmelgaco/tomikos`; ela será usada apenas no smoke final com HOME isolado.

## Provas observadas nesta iteração

- Sequência verde pós-correção concluída em três fixtures novas. Em cada matriz:
  `lina-core 34/34`, `lina-webhooks 2/2` e `lina-gpui 124/124` com um gate manual reservado,
  sempre sob `ulimit -n 128`; os três soaks de 1.000 trocas atingiram platô.

- A bateria pós-fix passou a suíte integral do app: 801/801 testes executados, um gate manual
  ignorado por contrato, acessibilidade 14/14, overflow 2/2 e token ratchet 2/2.
- Após o último ajuste da sidebar, a suíte integral foi observada novamente: 802 passaram,
  zero falhou e um gate manual foi ignorado por contrato. Clippy estrito, fmt e `git diff --check`
  saíram com código zero.
- A matriz 1/3 passou `core 34/34`, `webhooks 2/2` e `app 124/124` sob `ulimit -n 128`.
- A matriz 2/3 revelou um checker não determinístico no dreno de hooks: o `Drop` da future
  cancelada ocorria antes de o observador do JoinHandle necessariamente concluir, e oito
  `yield_now` não garantiam o agendamento. Controle negativo independente reproduziu 437/500.
- O encerramento agora decide a transferência antes do `abort` e o teste aguarda diretamente o
  único JoinHandle com timeout anti-hang. Quatro cenários direcionados passaram; stress local
  pós-fix passou 100/100, além de 1.000/1.000 isolados e 400/400 agrupados pelo verificador.

- RED original do event store: falha real entre commit SQLite e abertura do JSONL retornava
  `Err` apesar de `event_count == 1`, induzindo retry semântico.
- Criação atômica: `workspace_boot::tests` = 19/19; inclui seis cortes pré-publicação,
  restart, falha do registry, adoção pós-crash e 100 retries com o mesmo `creation_id`.
- Matriz app antes da revisão: `workspace_reliability_` = 25/25.
- Matriz core após endurecimento do espelho: `workspace_reliability_` = 6/6.
- Gate core de workspaces após tornar `Workspace::create` atômico:
  `cargo test -p lina-core --test gate_f1_4_1` = 19/19.
- Stress real com `/bin/zsh -lc 'ulimit -n 128; ...runtime_resources...'`:
  20 workspaces + 1.000 trocas; `fds 64→64`, `threads 34→34`, `listeners 8→8`,
  `runtimes 4→4`; PTY Busy preservado e prompt ocioso desmontado.
- Teste de falha de restore: 0/1 agentes restaurados aborta a troca, mantém foco/runtime atual,
  não publica o alvo e deixa zero PTY parcial.
- Teste de archive/restart: 20 ciclos de foco, Desfazer sem evento, arquivamento persistido e
  restart mantendo registry e scan convergentes.

## Achados que zeraram a sequência verde

### Segunda revisão fria (iteração 4)

- Um `lina.db` existente mas sem a tabela `events` era inicializado como vazio e podia
  reescrever um JSONL íntegro para zero eventos.
- JSONL com sequências contíguas não prova que contém o último commit SQLite; um prefixo válido
  foi aceito como recuperação completa e perdeu `WorkspaceIdAssigned`.
- O gate de recovery validava o envelope, mas não desserializava `DomainEvent` nem conferia a
  coerência entre `kind` e a tag do payload.
- O refresh aplicava as linhas válidas antes de tratar avisos transitórios, substituindo o
  último catálogo bom por um catálogo menor.
- Registry corrompido era contornado por scan apenas em memória e continuava corrompido para o
  switch; o modal bloqueava criação apenas quando o texto do erro parecia EMFILE.
- O fallback inicial podia montar o mesmo store arquivado/incompleto que acabara de rejeitar.
- Restore/relight persistiam eventos antes de todos os recursos estarem prontos e não conseguiam
  compensar o log; relight também podia duplicar o card preservado.
- A prontidão aceitava um prompt velho ainda visível durante comando silencioso.
- O teste de 1.000 switches girava somente sobre os quatro runtimes já montados, não sobre os
  vinte workspaces.
- A tentativa escalada da matriz integral acumulou longa espera de autorização e foi encerrada
  logo após iniciar; não há evidência suficiente para chamar isso de hang do produto. A próxima
  execução usará timeout explícito e separará tempo de aprovação do tempo do teste.

Remediação atual: watermark durável pending/complete e batch atômico no event store; catálogo com
erros estruturados, reconstrução persistida e bloqueio integral; restore/relight em duas fases,
prontidão por época de saída e soak realmente rotativo.

### Terceira revisão fria e stress real (iteração 5)

- Event store passou 53/53 testes funcionais, mas a reabertura saudável ainda fazia
  `integrity_check`/scan do SQLite e hash integral do JSONL; o custo crescia com o histórico.
- Catálogo com 30 stores × 700 eventos levou ~936 ms antes do mini-status e reabria cada store
  no scan, na reconciliação e novamente na sidebar.
- Ponteiro focado quebrado + default ausente encerrava o app mesmo havendo outro workspace saudável.
- ID divergente no registry era posto em quarentena, mas a flag de conflito impedia a própria
  reconciliação com a identidade autoritativa do store.
- `present_recovery` confundia ausência de artefato `.corrupt-*` com banco íntegro.
- O construtor core podia publicar por rename e depois retornar erro ao reabrir o store.
- O soak verdadeiro de 20 workspaces × 1.000 ativações atingiu apenas 124 focos e bateu timeout
  de 900 s. Perfil observado: verify/catalog 94–184 ms; boot 650–919 ms; focus/catalog
  210–573 ms; evict 553 ms (park 49 + joins 264 + Drop 236).

Remediação em curso: abertura saudável O(1), catálogo de uma passagem, fallback pelo próximo
`last_focus` íntegro, conflito reparável regravado, estado de recovery verificado, resultado
pós-publicação tipado, pumps acordáveis/parada em duas fases e novo profile 20/100 antes do soak 1.000.

### Event store

- Fast path antigo comparava apenas contagens e não detectava truncamento/corrupção física.
- Reconcile antigo podia colar evento depois de cauda parcial e preservava duplicatas.
- DB ausente com JSONL válido não reconstruía; DB corrompido sem JSONL fingia recuperação vazia.
- O carregamento real usava `EventStore::open`, não o caminho resiliente.

Remediação já aplicada, aguardando reauditoria: metadados físicos confirmados, `sync_data`,
rewrite canônico por tmp+rename+fsync do diretório, recuperação de DB ausente, recusa de JSONL
ambíguo/incompleto, ligação `open_store_resilient` no boot e teste pré-commit real no funil de PTY.

### Catálogo/criação

- Ponteiros stale podiam renderizar parcial/arquivado ou bloquear fallback válido.
- Registry corrompido na primeira carga encerrava antes do scan de recuperação.
- Erro de registry no modal virava no-op só no stderr.
- Uma entrada ruim de `read_dir`/metadata ainda podia abortar toda a enumeração.
- O construtor público `Workspace::create` era um segundo caminho não-atômico e
  `registry_entry` ainda usava path como identidade de parcial.

Remediação do core já aplicada; remediação do app em execução por worker independente.

### Runtime

- Admissão em curso podia terminar depois do primeiro recheck da evicção e deixar PTY órfão.
- Dois segundos sem input não provavam prompt ocioso; comando silencioso podia ser morto.
- Tarefas de canal/kill permaneciam destacadas sem quiescência.
- Relight e fatos pós-spawn ainda permitiam falso Ready parcial.

Remediação em execução por worker independente.

### Auditoria dos gates e reauditoria v6 (iteração 7)

- O event store foi reaudidato após validação de sequência zero, versão, `kind` e payload:
  PASS 100; 62 testes de eventos e 765 testes do core foram observados pelo revisor.
- O catálogo v6 encontrou e corrigiu três caminhos de desaparecimento ainda reais: Settings
  calculava a base como `<root>/.lina`, não usava o registry unificado e stores conhecidos sem
  fatos obrigatórios/base temporariamente ausente ainda podiam regravar o last-good menor.
- A prova de escrita do Diretório de Trabalho usava nome previsível por PID, podia truncar uma
  colisão e engolia falha de remoção. Agora usa UUID, `create_new`, `sync_all` e cleanup obrigatório;
  o gate manual do TomikOS foi compilado, mas ainda não executado.
- A revisão do runtime encontrou três cortes reais: dreno de hooks abortado sem join, entrega de
  webhook aceita durante quiescência e foco ordenado por relógio de parede. A remediação específica
  passou nove testes, check e clippy; a reauditoria independente continua ativa.
- A auditoria adversarial A-H emitiu FAIL 34: B não reiniciava processo nem provava roster; C só
  cobria seis cortes pré-rename; D não comparava baseline vazio/operações; E/F eram provas locais;
  G permitia reduzir o soak e pular métricas; H simulava restart/Undo apenas com JSON/Vec.
- A mesma auditoria revelou defeito de produto: `CreateSpaceModal.creation_id` vive apenas em RAM.
  Um crash pré-publicação deixa `.lina-create-*` que um modal novo não sabe retomar; um staging
  completo também podia ser enumerado pelo catálogo. A criação agora persiste uma intenção
  versionada antes do staging, serializa criadores entre processos e recupera nove cortes abruptos
  até `RegistrySaved`; staging sem journal fica oculto e é reportado, nunca adotado por heurística.
- Gate E integrado passou usando catálogo, cache e `SidebarState` reais: registry inválido e falha
  de `read_dir` preservam linhas, foco, atalhos, arquivados e os bytes do last-good.
- Gate H deixou de simular reinício com reload de JSON: passou com vinte trocas pelo runtime real,
  cada uma em processo novo sobre o mesmo HOME isolado, além de Desfazer, commit de arquivamento,
  recusa do arquivado e convergência entre catálogo e linhas da sidebar em cada transição.
- Primeira bateria ampla após essas mudanças: 98 passaram, 4 falharam e 1 foi ignorado. Os REDs
  remanescentes foram isolados: arquivo de lock contado no Gate B, quiescência impedindo evicção
  no soak G, expectativa textual de JSONL ambíguo e bind loopback proibido pelo sandbox.
- Filas humanas, reinjeção, entregas externas e trabalho interno agora fecham ingresso antes do
  descarregamento e acordam/aguardam os pumps por condição real; os testes específicos de perda,
  handle de hook e quiescência passaram. O park transacional ainda está em implementação.
- O scrollback deixou de criar um `FlushGuard` por runtime: todos os stores registram-se num único
  coordenador/thread do processo, e um sinal drena todos antes de ser reemitido. Provas observadas:
  gate novo 1/1, `gate_f1_5_6_9` 15/15 e clippy estrito do core verde.
- A matriz reliability do core foi repetida depois do foco interprocesso: 29/29; a matriz de
  webhooks passou 1/1. Essas execuções ainda não contam para a sequência verde enquanto app/F/G
  e as revisões independentes estiverem pendentes.

## Matriz

| Invariante | Estado | Prova atual | Checker |
|---|---|---|---|
| A · semântica DB/JSONL | verde 3/3 | commit/recovery/certificado dentro dos 34 testes core | PASS |
| B · criação exatamente uma vez | verde 3/3 | 100 retries, restart e lock entre processos | PASS |
| C · matriz de falhas | verde 3/3 | cortes de criação, boot, restore e foco | PASS |
| D · pasta grande/custo constante | verde 3/3 | vazio/grande sob ulimit; TomikOS real sem resíduo e FDs 4→4 | PASS |
| E · sidebar último-bom | verde 3/3 | registry/read_dir/entrada ruim preservam snapshot | PASS |
| F · troca/fallback/Ready | verde 3/3 | falhas mantêm atual e desmontam boot parcial | PASS |
| G · platô de recursos | verde 3/3 | 20 workspaces, 1.000 trocas, teto real | PASS |
| H · restart/arquivo/navegação | verde 3/3 | processos novos, archive/undo/sidebar convergentes | PASS |

## Restrições preservadas

- Não modificar nem remover `tasks/despachos/achados-dogfooding-sessao.md`.
- Não modificar nem remover `app/lina-gpui/test.out`.
- Não limpar nem migrar silenciosamente os workspaces TomikOS reais.
- Não aumentar `ulimit` como correção.

## Fechamento observado — iteração 13

- `packaging/make-app.sh --no-dmg` reconstruiu o bundle final; binários Mach-O arm64, UUIDs,
  recursos, perfis e `Info.plist` foram comparados com os targets de release: `PACKAGE_AUDIT=PASS`.
- O binário empacotado abriu duas vezes sobre o mesmo HOME isolado: registry continuou com uma
  entrada, um `WorkspaceCreated` e um `WorkspaceIdAssigned`: `RESTART_SMOKE=PASS`.
- O probe efêmero no Diretório de Trabalho real `/Users/rafaelmelgaco/tomikos` terminou sem
  resíduo e sem aumento de descritores (`4→4`); nenhum conteúdo do projeto foi percorrido.
- O dogfood nativo criou um workspace apontando para TomikOS, alternou 25 vezes, abriu outras
  superfícies e não observou erro 24, duplicata, parcial, desaparecimento ou reinício forçado.
  Ele encontrou duas lacunas finais: busca da sidebar permanecia dona do teclado depois da troca,
  e Desfazer não tinha papel/rótulo acessível. Ambas foram corrigidas e cobertas por regressões;
  a suíte final de 802 testes e o gate de acessibilidade passaram depois das mudanças.
- O bundle final voltou a iniciar e renderizar integralmente em HOME isolado em
  `/private/tmp/lina-native-final.gFuvcb`. Nesta sessão remota, o macOS não publicou a janela para
  `System Events`, tanto no launch direto quanto via `.app`; por isso a automação visual final não
  conseguiu repetir os cliques. Os logs provam boot/render saudável e a limitação ficou registrada,
  sem ser mascarada como falha do produto nem provocar outra bateria redundante.
- Gate H integrado observou processos novos, troca, arquivar/desfazer, restart e convergência da
  sidebar; o teste específico da busca pós-troca observou que as duas linhas voltam e o teclado é
  liberado. A combinação cobre os gestos que o host não permitiu repetir por automação visual.
- Revisão fria independente final: `PASS`, score 100, nenhuma violação e nenhum bloqueador
  demonstrável contra A-H.
- O registry e os stores reais do usuário não foram limpos, migrados ou modificados.

## Resultado

Loop encerrado. Todas as invariantes A-H estão verdes 3/3; o pacote final, o restart isolado,
o TomikOS real, a regressão da sidebar e a revisão independente têm evidência observada.
