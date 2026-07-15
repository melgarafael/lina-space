# Loop de confiabilidade de workspaces

## Missão

Corrigir definitivamente a criação, o carregamento e a navegação de workspaces do Lina.
O ciclo continua até haver prova repetível de que:

1. escolher uma pasta grande não altera o custo de criação nem causa `os error 24`;
2. uma tentativa lógica cria exatamente um workspace completo;
3. uma falha nunca publica um workspace parcial ou uma duplicata;
4. uma falha de leitura nunca apaga da sidebar uma lista que já era válida;
5. alternar, arquivar e reabrir workspaces não exige reiniciar o Lina;
6. descritores, threads, listeners e runtimes atingem um platô mensurável.

Este documento é o controlador do ciclo. O estado entre execuções vive em
`tasks/workspace-reliability/STATE.md`. Uma nova sessão começa lendo os dois arquivos;
ela nunca recomeça a investigação do zero.

## Base do método

O princípio de Peter Steinberger é projetar o sistema que volta a instruir os agentes,
em vez de depender de uma sequência manual de prompts. Este loop materializa o princípio
com estado externo, trabalho isolado, agentes separados para produzir e verificar,
critérios determinísticos e retomada após troca de contexto.

Fontes:

- Peter Steinberger: <https://x.com/steipete/status/2063697162748260627>
- Anatomia operacional citando Steinberger: <https://addyosmani.com/blog/loop-engineering/>

## Evidência inicial observada em 2026-07-10

- O limite macio deste Mac é `256` descritores.
- O registry real contém nove raízes `TomikOS - OpenSource*`, oito delas sufixadas e
  arquivadas após as tentativas descritas pelo usuário.
- Esses stores têm `WorkspaceCreated`, mas não têm `WorkspaceIdAssigned`, roster nem
  `WorkspaceDefaultCwdSet`. São criações interrompidas, não workspaces completos.
- Em vários casos, o SQLite contém o `WorkspaceCreated` original com o preset escolhido,
  enquanto `log.jsonl` não contém essa gravação.
- `EventStore::insert_raw` confirma primeiro no SQLite e abre/escreve `log.jsonl` depois.
  Se a segunda etapa recebe erro 24, a função responde erro apesar de o banco já ter
  confirmado o evento.
- `create_workspace` deixa a pasta e o store intermediários no local final. O retry vê a
  pasta, considera o nome ocupado e cria `(2)`, `(3)` e assim por diante.
- `refresh_sidebar_rows` converte falha de registry em lista vazia; a varredura faz o
  mesmo com falha de `read_dir`; `set_rows` então substitui o último estado bom por zero.
- Cada workspace visitado fica montado em `RuntimeMap`. Não há teto nem remoção completa;
  descarregar encerra PTYs ociosos, mas não todos os stores, pumps e listeners.

Arquivos-âncora:

- `crates/lina-core/src/events.rs`
- `crates/lina-core/src/workspace.rs`
- `app/lina-gpui/src/workspace_boot.rs`
- `app/lina-gpui/src/runtime.rs`
- `app/lina-gpui/src/main.rs`
- `app/lina-gpui/src/persistence_ui.rs`
- `app/lina-gpui/src/sidebar.rs`

## Restrições

- Preservar alterações preexistentes do usuário, especialmente
  `tasks/despachos/achados-dogfooding-sessao.md` e `app/lina-gpui/test.out`.
- Não apagar nem reparar silenciosamente os workspaces reais do usuário. Migração ou
  limpeza do registry real exige plano reversível e confirmação humana.
- Não usar `cargo clean` durante o ciclo.
- Não aumentar `ulimit` como correção. O produto deve funcionar dentro do limite real.
- Não mascarar erro com `.ok()`, `unwrap_or_default()` ou fallback temporário compartilhado.
- Não considerar o tamanho da pasta escolhida uma desculpa: a criação só precisa validar
  a raiz, nunca percorrer o projeto.
- Não fazer push, merge, release ou deploy sem confirmação humana.
- Um agente nunca aprova o próprio trabalho.

## Estrutura mínima da solução

A implementação deve buscar a menor mudança que garanta estes contratos:

1. **Sem falso erro depois de commit.** A API do event store deve distinguir falha antes
   do commit de degradação do espelho depois do commit. SQLite continua sendo a autoridade;
   o espelho precisa ser reconciliável sem induzir retry semântico. Recuperação a partir do
   JSONL exige um certificado durável `complete` com contagem, cabeça e digest exatamente iguais;
   um marcador `pending` precisa chegar ao disco antes do commit SQLite. Sequência contígua,
   sozinha, nunca prova que o espelho contém o último commit.
2. **Criação atômica e retomável.** Preparar em staging no mesmo volume, validar a projeção
   completa, publicar por rename e só então atualizar o catálogo. Falha pré-publicação
   deixa zero workspace visível. Falha pós-publicação retoma o mesmo workspace.
3. **Idempotência por intenção.** A mesma operação de criação usa um `creation_id` durável.
   Repetir Enter, retry ou reiniciar não cria outro nome. Uma nova intenção humana ainda
   pode criar um workspace distinto e receber sufixo.
4. **Catálogo último-bom.** Sidebar, criação, atalhos e troca compartilham um snapshot
   válido. Uma leitura transitória com erro preserva esse snapshot e mostra o problema;
   nunca o substitui por vazio nem por uma lista parcial. Erros determinísticos de identidade
   podem isolar uma entrada; erro de I/O/parse/permissão torna o snapshot inteiro inconclusivo.
5. **Isolamento de falha.** Um store inválido vira uma linha com aviso ou fica em
   quarentena; não derruba os workspaces saudáveis. Um fallback jamais compartilha o
   mesmo store entre raízes diferentes.
6. **Foco depois de prontidão.** O ponteiro global muda somente quando o runtime alvo
   terminou o boot. Falha mantém o workspace atual e desmonta tudo que o boot parcial abriu.
7. **Orçamento de recursos.** Serviços naturalmente globais são compartilhados. Todo
   recurso por-workspace tem shutdown e `join`. Runtime de fundo ocioso pode ser evictado;
   trabalho realmente ativo é preservado. Restore/relight preparam todos os recursos antes de
   persistir um batch único; prompt antigo ainda visível nunca autoriza evicção — é preciso
   observar uma nova ocorrência do prompt depois da última entrada.
8. **Compatibilidade observável.** Stores parciais legados não aparecem como workspaces
   normais. O app identifica, explica e oferece retomada/arquivamento sem perder dados.

Só criar uma abstração nova quando houver pelo menos dois consumidores reais. Hoje há
justificativa para um catálogo único, consumido por sidebar, criação, atalhos e troca. Não
há autorização para reescrever todo o runtime.

## Papéis do ciclo

O controlador usa no máximo três workers por iteração:

- **Investigador:** somente leitura; mede e produz uma hipótese falsificável.
- **Implementador:** escreve uma única fatia coerente e seu teste de regressão.
- **Verificador:** não edita; repete a prova em fixture nova, revisa o diff e emite
  `PASS` ou `FAIL` com comando, exit code e artefato.

Se houver trabalho independente, investigador de persistência e investigador de runtime
podem atuar em paralelo. Dois implementadores nunca editam a mesma fronteira.

## Algoritmo do loop

```text
ENQUANTO todos_os_gates_verdes_por_3_execucoes_consecutivas == falso:
    1. Ler LOOP.md, STATE.md, git status e o último artefato de falha.
    2. Escolher o primeiro invariante vermelho da matriz de aceite.
    3. Investigador:
       a. reproduzir em HOME e workspace isolados;
       b. medir antes;
       c. escrever uma hipótese que possa ser refutada;
       d. criar ou especificar um teste RED não-vácuo.
    4. Implementador:
       a. confirmar que o teste falha pelo motivo esperado;
       b. fazer a menor mudança que resolve a causa;
       c. rodar testes direcionados;
       d. atualizar STATE.md com diff, comandos e outputs.
    5. Verificador independente:
       a. verificar que o controle negativo realmente morde;
       b. revisar a semântica de commit, rollback e shutdown;
       c. repetir em fixture, seed e HOME novos;
       d. rodar o conjunto direcionado e o stress correspondente;
       e. emitir PASS ou FAIL, nunca "parece correto".
    6. Se FAIL:
       a. gravar fingerprint da falha e evidência;
       b. zerar a sequência verde;
       c. devolver o defeito exato ao implementador;
       d. após 3 fingerprints iguais, proibir outro remendo igual e exigir nova hipótese.
    7. Se PASS:
       a. marcar o invariante verde;
       b. escolher o próximo vermelho;
       c. quando todos estiverem verdes, incrementar green_streak e repetir a matriz
          inteira com nova seed.
    8. A cada 12 iterações, iniciar uma sessão fresca e retomar de STATE.md. Isso é
       renovação de contexto, não conclusão nem abandono.
```

O ciclo só pede ajuda humana quando faltar autorização externa, quando a prova depender
de uma ação irreversível ou quando o gate visual final precisar do usuário. Dificuldade,
tempo de compilação ou troca de contexto não são bloqueios.

## Matriz obrigatória de aceite

### A. Semântica do event store

`workspace_reliability_eventstore_never_reports_uncommitted_after_db_commit`

- Injetar falha entre o commit SQLite e o espelho JSONL.
- O chamador precisa saber inequivocamente se deve ou não repetir a operação.
- Reabrir reconcilia DB e espelho sem duplicar eventos.
- Um controle negativo precisa provar que o comportamento antigo falhava.

### B. Criação exatamente uma vez

`workspace_reliability_same_creation_id_is_exactly_once`

- Reenviar cem vezes a mesma intenção, inclusive após restart.
- Resultado: uma raiz, um `workspace_id`, uma sequência canônica de criação, um roster e
  uma entrada no catálogo.
- Uma intenção nova com o mesmo nome continua sendo uma operação distinta.

### C. Matriz de falhas

`workspace_reliability_failure_matrix_never_publishes_partial`

Injetar falha depois de cada fronteira:

- criação do staging;
- abertura do store;
- `WorkspaceCreated`;
- roster/preset;
- `WorkspaceIdAssigned`;
- `WorkspaceDefaultCwdSet`;
- publicação por rename;
- persistência do catálogo;
- boot do runtime;
- restore de cada agente;
- carimbo de foco.

Após cada corte, restart converge para exatamente um dos estados: não criado ou completo.
Nunca há workspace normal sem agentes, ID ou cwd esperado.

### D. Pasta grande com custo constante

`workspace_reliability_large_workdir_does_not_expand_creation_work`

- Usar fixture profunda com muitos arquivos, Unicode, symlink e subárvore sem leitura.
- A criação não chama scan recursivo do Diretório de Trabalho.
- Rodar sob `ulimit -n 128` depois de aquecer o build.
- Comparar com pasta vazia: mesma quantidade de operações sobre a raiz e delta de FDs
  dentro da tolerância definida no teste.
- Repetir no final com a pasta `tomikos` real, sem modificar seu conteúdo.

### E. Sidebar resiliente

`workspace_reliability_transient_read_error_keeps_last_good_sidebar`

- Começar com vários workspaces válidos e sidebar populada.
- Falhar registry e `read_dir` na atualização seguinte.
- As linhas válidas permanecem; o ativo sempre está presente; a UI recebe um aviso.
- Uma entrada inválida no meio do catálogo não esconde as outras.
- Arquivado não reaparece quando o registry estiver temporariamente indisponível.

### F. Troca e recuperação

`workspace_reliability_switch_failure_keeps_current_ready_workspace`

- Ponteiro morto, ID divergente, store corrompido e boot interrompido.
- Nenhum caso cria store novo no path morto ou usa fallback compartilhado.
- O foco só muda após `Ready`.
- Recursos do boot parcial são encerrados e aguardados.

### G. Platô de recursos

`workspace_reliability_runtime_resources_reach_plateau`

- Visitar pelo menos vinte workspaces e alternar mil vezes.
- Medir FDs, threads, listeners e runtimes montados.
- Entre a metade e o fim do soak, os FDs não podem crescer linearmente; o teste fixa um
  teto e reserva de segurança abaixo de `RLIMIT_NOFILE`.
- Depois de evictar/desmontar, os recursos retornam ao baseline dentro da tolerância.
- Workspaces com agente ocupado continuam trabalhando; ociosos podem ser desmontados.

### H. Restart/arquivo/navegação

`workspace_reliability_switch_archive_restart_soak`

- Criar, alternar, arquivar, desfazer e reiniciar em ciclos com HOME isolado.
- Em toda transição, sidebar e catálogo representam o mesmo conjunto.
- Nenhuma aba fica dependente de fechar e abrir o Lina.

## Comandos direcionados

O app GPUI é excluído do workspace Cargo raiz. Portanto os dois conjuntos são obrigatórios:

```sh
cargo test --manifest-path app/lina-gpui/Cargo.toml \
  workspace_reliability_ -- --test-threads=1 --nocapture

cargo test --manifest-path app/lina-gpui/Cargo.toml \
  workspace_boot::tests -- --test-threads=1

cargo test --manifest-path app/lina-gpui/Cargo.toml \
  sidebar::tests -- --test-threads=1

cargo test --manifest-path app/lina-gpui/Cargo.toml \
  runtime::tests -- --test-threads=1

cargo test -p lina-core --test gate_f1_4_1 -- --test-threads=1

cargo test -p lina-webhooks workspace_reliability_ -- --test-threads=1 --nocapture
```

Stress com poucos descritores, somente depois de compilar uma vez:

```sh
cargo test --manifest-path app/lina-gpui/Cargo.toml \
  workspace_reliability_ --no-run

/bin/zsh -c 'ulimit -n 128; cargo test --offline \
  --manifest-path app/lina-gpui/Cargo.toml \
  workspace_reliability_ -- --test-threads=1 --nocapture'
```

## Gates finais

```sh
cargo test --workspace -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings

cargo test --manifest-path app/lina-gpui/Cargo.toml -- --test-threads=1
cargo clippy --manifest-path app/lina-gpui/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path app/lina-gpui/Cargo.toml -- --check
cargo fmt -p lina-core -- --check
```

O smoke usa binário compilado, `HOME` isolado e sem `LINA_DEMO=1`, pois o modo demo pula
parte do comportamento real do registry. O empacotamento final usa
`packaging/make-app.sh --no-dmg`; `packaging/dev-watch.sh` não participa do loop.

## Condição única de parada

O controlador só escreve `COMPLETE` em `STATE.md` quando observar:

- todos os testes A–H verdes em três execuções consecutivas, com fixtures novas;
- zero erro 24;
- zero duplicata para o mesmo `creation_id`;
- zero workspace parcial publicado;
- um workspace ruim sem reduzir o conjunto dos bons;
- mil trocas com platô de recursos;
- suítes completas, clippy e fmt verdes;
- smoke de restart em HOME isolado verde;
- gate visual no Lina real: criar com `tomikos`, trocar repetidamente entre workspaces e
  outras abas, arquivar/desfazer e reabrir sem desaparecimento nem reinício forçado;
- revisão independente sem achado bloqueante.

Ausência de erro, teste histórico e afirmação do implementador não contam como prova.
