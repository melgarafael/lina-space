# Graph Report - .  (2026-06-29)

## Corpus Check
- 117 files · ~78,840 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 382 nodes · 549 edges · 37 communities (32 shown, 5 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 18 edges (avg confidence: 0.78)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Harness Scripts Core|Harness Scripts Core]]
- [[_COMMUNITY_AgentDB & Memory Bridge|AgentDB & Memory Bridge]]
- [[_COMMUNITY_Similarity Test Harness|Similarity Test Harness]]
- [[_COMMUNITY_Harness Similarity Metric|Harness Similarity Metric]]
- [[_COMMUNITY_Intelligence Pipeline & Knowledge Graph|Intelligence Pipeline & Knowledge Graph]]
- [[_COMMUNITY_Darwin Evolve Engine|Darwin Evolve Engine]]
- [[_COMMUNITY_Metaharness Evolve|Metaharness Evolve]]
- [[_COMMUNITY_RVF Memory & Security Audit|RVF Memory & Security Audit]]
- [[_COMMUNITY_RedBlue Security Bench|Red/Blue Security Bench]]
- [[_COMMUNITY_Similarity Spike|Similarity Spike]]
- [[_COMMUNITY_Cluster 10|Cluster 10]]
- [[_COMMUNITY_Cluster 11|Cluster 11]]
- [[_COMMUNITY_Loop Workers|Loop Workers]]
- [[_COMMUNITY_Swarm & Anti-Drift|Swarm & Anti-Drift]]
- [[_COMMUNITY_Cluster 14|Cluster 14]]
- [[_COMMUNITY_Cluster 15|Cluster 15]]
- [[_COMMUNITY_Cluster 16|Cluster 16]]
- [[_COMMUNITY_Cluster 17|Cluster 17]]
- [[_COMMUNITY_Cluster 18|Cluster 18]]
- [[_COMMUNITY_Cluster 19|Cluster 19]]
- [[_COMMUNITY_Cluster 20|Cluster 20]]
- [[_COMMUNITY_Cluster 21|Cluster 21]]
- [[_COMMUNITY_Cluster 22|Cluster 22]]
- [[_COMMUNITY_Cluster 23|Cluster 23]]
- [[_COMMUNITY_Cluster 24|Cluster 24]]
- [[_COMMUNITY_Cluster 25|Cluster 25]]
- [[_COMMUNITY_Cluster 26|Cluster 26]]
- [[_COMMUNITY_Cluster 27|Cluster 27]]
- [[_COMMUNITY_Cluster 28|Cluster 28]]
- [[_COMMUNITY_Cluster 29|Cluster 29]]
- [[_COMMUNITY_Cluster 30|Cluster 30]]
- [[_COMMUNITY_Cluster 31|Cluster 31]]
- [[_COMMUNITY_Cluster 32|Cluster 32]]
- [[_COMMUNITY_Cluster 33|Cluster 33]]
- [[_COMMUNITY_Cluster 34|Cluster 34]]
- [[_COMMUNITY_Cluster 35|Cluster 35]]
- [[_COMMUNITY_Cluster 36|Cluster 36]]

## God Nodes (most connected - your core abstractions)
1. `similarity()` - 14 edges
2. `emitDegradedJsonAndExit()` - 13 edges
3. `rankSeverity()` - 11 edges
4. `runMetaharness()` - 10 edges
5. `parseMcpScanText()` - 8 edges
6. `emitDarwinDegradedJsonAndExit()` - 7 edges
7. `runHarness()` - 7 edges
8. `main()` - 7 edges
9. `ruflo-agentdb plugin` - 7 edges
10. `claude-memories auto-memory bridge` - 7 edges

## Surprising Connections (you probably didn't know these)
- `HNSW pattern retrieval` --semantically_similar_to--> `Pathfinder traversal algorithm (seed/expand/score/prune/rank)`  [INFERRED] [semantically similar]
  ruflo-intelligence/README.md → ruflo-knowledge-graph/README.md
- `RaBitQ 1-bit quantization (32x memory)` --semantically_similar_to--> `SmartRetrieval 5-phase pipeline (ADR-090)`  [INFERRED] [semantically similar]
  ruflo-agentdb/README.md → ruflo-rag-memory/README.md
- `Causal knowledge graph (agentdb_causal-edge)` --semantically_similar_to--> `ruvector backend (FlashAttention-3, Graph RAG, DiskANN)`  [INFERRED] [semantically similar]
  ruflo-agentdb/README.md → ruflo-rag-memory/agents/memory-specialist.md
- `SONA trajectory / neural pattern learning` --semantically_similar_to--> `Memory consolidation (dedup/prune/re-index)`  [INFERRED] [semantically similar]
  ruflo-agentdb/README.md → ruflo-rag-memory/agents/memory-specialist.md
- `ADR-0001 intelligence surface completeness` --semantically_similar_to--> `ADR-0001 knowledge-graph contract`  [INFERRED] [semantically similar]
  ruflo-intelligence/docs/adrs/0001-intelligence-surface-completeness.md → ruflo-knowledge-graph/docs/adrs/0001-knowledge-graph-contract.md

## Import Cycles
- 1-file cycle: `ruflo-subset/ruflo-metaharness/scripts/redblue.mjs -> ruflo-subset/ruflo-metaharness/scripts/redblue.mjs`
- 1-file cycle: `ruflo-subset/ruflo-metaharness/scripts/similarity.mjs -> ruflo-subset/ruflo-metaharness/scripts/similarity.mjs`

## Hyperedges (group relationships)
- **AgentDB substrate: three MCP tool families wrapped by ruflo-agentdb** — ruflo_agentdb_readme_plugin, ruflo_agentdb_readme_controller_bridge, ruflo_agentdb_readme_embeddings_engine, ruflo_agentdb_readme_ruvllm_router [INFERRED 0.85]
- **SmartRetrieval 5-phase recall pipeline** — ruflo_rag_memory_readme_smartretrieval, ruflo_rag_memory_agents_memory_specialist_rrf_fusion, ruflo_rag_memory_agents_memory_specialist_mmr_diversity, ruflo_rag_memory_agents_memory_specialist_recency_decay, ruflo_agentdb_skills_vector_search_skill_hnsw_index [INFERRED 0.85]
- **claude-memories auto-import bridge flow** — ruflo_agentdb_readme_claude_memories_bridge, ruflo_agentdb_readme_memory_import_claude, ruflo_agentdb_readme_memory_search_unified, ruflo_agentdb_readme_onnx_minilm, ruflo_rag_memory_readme_agentdb_store [INFERRED 0.85]
- **ruflo-intelligence self-learning surface (agent + skills + readme)** — ruflo_intelligence_readme_surface, ruflo_intelligence_specialist_agent, ruflo_intelligence_neural_train_skill, ruflo_intelligence_route_skill, ruflo_intelligence_transfer_skill [INFERRED 0.85]
- **ruflo-knowledge-graph plugin surface** — ruflo_knowledge_graph_readme, ruflo_knowledge_graph_navigator_agent, ruflo_knowledge_graph_kg_command, ruflo_knowledge_graph_extract_skill, ruflo_knowledge_graph_traverse_skill [INFERRED 0.85]
- **pathfinder traversal implementors** — ruflo_knowledge_graph_navigator_agent, ruflo_knowledge_graph_kg_command, ruflo_knowledge_graph_traverse_skill [INFERRED 0.95]
- **MetaHarness read/describe layer (score+genome+mcp-scan+threat-model+oia-audit)** — ruflo_subset_ruflo_metaharness_skills_harness_score_skill, ruflo_subset_ruflo_metaharness_skills_harness_genome_skill, ruflo_subset_ruflo_metaharness_skills_harness_mcp_scan_skill, ruflo_subset_ruflo_metaharness_skills_harness_threat_model_skill, ruflo_subset_ruflo_metaharness_skills_harness_oia_audit_skill [INFERRED 0.85]
- **Darwin evolution write-loop (bench corpus → evolve variants → security-bench grade)** — ruflo_subset_ruflo_metaharness_skills_harness_bench_skill, ruflo_subset_ruflo_metaharness_skills_harness_evolve_skill, ruflo_subset_ruflo_metaharness_skills_harness_security_bench_skill [INFERRED 0.75]
- **Loop-workers scheduling surface (/loop + CronCreate dispatch)** — ruflo_subset_ruflo_loop_workers_skills_loop_worker_skill, ruflo_subset_ruflo_loop_workers_skills_cron_schedule_skill, ruflo_subset_ruflo_loop_workers_agents_loop_worker_coordinator_agent, ruflo_subset_ruflo_loop_workers_commands_ruflo_loop_command, ruflo_subset_ruflo_loop_workers_commands_ruflo_schedule_command [INFERRED 0.85]
- **ruflo plugin contract cadence (v3.6 pin, namespace coordination, smoke-as-contract)** — ruflo_subset_ruflo_rvf_readme, ruflo_subset_ruflo_security_audit_readme, ruflo_subset_ruflo_swarm_readme [INFERRED 0.85]
- **anti-drift coordination stack (coordinator + hierarchical/raft + swarm-init)** — swarm_concept_anti_drift, swarm_coordinator_agent, swarm_skill_swarm_init, swarm_concept_hive_consensus [INFERRED 0.75]
- **layered defense (static scan + runtime 3-gate + shell-injection catalog)** — ruflo_subset_ruflo_security_audit_readme, secaudit_concept_aidefence_3gate, secaudit_concept_shell_injection_catalog [INFERRED 0.85]

## Communities (37 total, 5 thin omitted)

### Community 0 - "Harness Scripts Core"
Cohesion: 0.10
Nodes (37): ARGS, loadRecord(), main(), memRetrieve(), ARGS, bench(), gate, payload (+29 more)

### Community 1 - "AgentDB & Memory Bridge"
Cohesion: 0.08
Nodes (41): agentdb-specialist agent, pattern-store fallback (memory-store-fallback, ADR-093), /agentdb command, /embeddings command, ADR-0001 Optimize ruflo-agentdb, Causal knowledge graph (agentdb_causal-edge), claude-memories auto-memory bridge, AgentDB controller bridge (agentdb_* 15 MCP tools) (+33 more)

### Community 2 - "Similarity Test Harness"
Cohesion: 0.06
Nodes (29): A, ARGS, B, cont, cosAB, empty, failures, jacOnly (+21 more)

### Community 3 - "Harness Similarity Metric"
Cohesion: 0.13
Nodes (22): ARGS, bench(), CHEAP, gate, payload, results, RICH, TYPICAL (+14 more)

### Community 4 - "Intelligence Pipeline & Knowledge Graph"
Cohesion: 0.10
Nodes (24): ADR-0001 intelligence surface completeness, /neural command, neural-train skill, EWC++ consolidation, 4-step pipeline RETRIEVE-JUDGE-DISTILL-CONSOLIDATE, HNSW pattern retrieval, hooks_transfer IPFS pattern transfer, MicroLoRA adapter (+16 more)

### Community 5 - "Darwin Evolve Engine"
Cohesion: 0.18
Nodes (18): ARGS, main(), safetyChecks(), buildArgv(), classifyDegraded(), emitDarwinDegradedJsonAndExit(), maybeParseJson(), runDarwin() (+10 more)

### Community 6 - "Metaharness Evolve"
Cohesion: 0.17
Nodes (16): Darwin Mode human-initiated, not autonomous self-modification (--confirm gate), Seven mutation surfaces (planner/contextBuilder/reviewer/retryPolicy/toolPolicy/memoryPolicy/scorePolicy), Similarity formula: 0.60·cosine + 0.25·categorical + 0.15·jaccard, ruflo-metaharness plugin (MetaHarness integration), ruflo-metaharness command (score/genome/mint/mcp-scan/threat-model), harness-bench skill (darwin bench create/verify corpora), harness-drift-from-history skill (audit-list+oia-audit+audit-trend), harness-evolve skill (darwin evolve, 7 policy surfaces, promote wins) (+8 more)

### Community 7 - "RVF Memory & Security Audit"
Cohesion: 0.15
Nodes (16): /rvf command (memory stats + session list), ruflo-rvf plugin (portable memory + session persistence), ruflo-security-audit plugin (static scanning, policy gates), ADR-0001 ruflo-rvf plugin contract, encryption at rest (AES-256-GCM, RFE1 prefix, ADR-096), RVF cognitive container (vector embeddings + causal graph), session-specialist agent (haiku), rvf-manage skill (RVF file import/export/migrate) (+8 more)

### Community 8 - "Red/Blue Security Bench"
Cohesion: 0.23
Nodes (12): ARGS, buildUpstreamArgs(), CACHE_DIR, classifyDegraded(), emitRedblueDegradedJsonAndExit(), ensureInstalled(), main(), printHelp() (+4 more)

### Community 9 - "Similarity Spike"
Cohesion: 0.19
Nodes (13): ARGS, categoricalAgreement(), cosine(), DEVOPS, jaccard(), LEGAL, legalVsDevops, legalVsSupport (+5 more)

### Community 10 - "Cluster 10"
Cohesion: 0.33
Nodes (9): ARGS, assert(), cheapInferenceCall(), failures, fetchSecretFromGcp(), listOpenRouterModels(), main(), runHarness() (+1 more)

### Community 11 - "Cluster 11"
Cohesion: 0.22
Nodes (6): ARGS, failures, REPO_ROOT, SCRIPTS_DIR, summary, tmp

### Community 12 - "Loop Workers"
Cohesion: 0.29
Nodes (8): 270s cache-aware ScheduleWakeup heartbeat (under 5-min prompt-cache TTL), ruflo-loop-workers plugin (cache-aware /loop + CronCreate, 12 workers), loop-worker-coordinator agent (dispatch/monitor/schedule workers), ruflo-loop command (start cache-aware /loop worker), ruflo-schedule command (persistent worker via CronCreate), ADR-0001 ruflo-loop-workers plugin contract, cron-schedule skill (CronCreate persistent workers), loop-worker skill (native /loop + ScheduleWakeup)

### Community 13 - "Swarm & Anti-Drift"
Cohesion: 0.32
Nodes (8): ruflo-swarm plugin (12-tool MCP, topologies, monitor), ADR-0001 ruflo-swarm plugin contract, /watch command (live NDJSON event stream), anti-drift defaults (hierarchical/specialized/raft, 6-8 agents), hive-mind consensus strategies (Byzantine/Raft/Gossip/CRDT/Quorum), coordinator agent (anti-drift enforcement), monitor-stream skill (Monitor over polling), swarm-init skill (topology + anti-drift config)

### Community 14 - "Cluster 14"
Cohesion: 0.43
Nodes (6): ARGS, emitAndExit(), main(), runScriptJson(), runScriptJsonAsync(), SCRIPTS_DIR

### Community 15 - "Cluster 15"
Cohesion: 0.38
Nodes (6): ARGS, assert(), failures, main(), runWithUnreachableRegistry(), SCRIPTS_DIR

### Community 16 - "Cluster 16"
Cohesion: 0.53
Nodes (5): ARGS, main(), memList(), memRetrieve(), parseDurationMs()

### Community 17 - "Cluster 17"
Cohesion: 0.47
Nodes (4): ARGS, main(), mean(), pctile()

### Community 18 - "Cluster 18"
Cohesion: 0.40
Nodes (5): ARGS, assert(), failures, main(), SCRIPTS_DIR

### Community 19 - "Cluster 19"
Cohesion: 0.70
Nodes (4): bad(), ok(), smoke.sh script, step()

### Community 20 - "Cluster 20"
Cohesion: 0.70
Nodes (4): bad(), ok(), smoke.sh script, step()

### Community 21 - "Cluster 21"
Cohesion: 0.70
Nodes (4): bad(), ok(), smoke.sh script, step()

### Community 22 - "Cluster 22"
Cohesion: 0.70
Nodes (4): bad(), ok(), smoke.sh script, step()

### Community 23 - "Cluster 23"
Cohesion: 0.70
Nodes (4): bad(), ok(), smoke.sh script, step()

### Community 24 - "Cluster 24"
Cohesion: 0.50
Nodes (4): assert(), failures, main(), SCRIPTS_DIR

### Community 25 - "Cluster 25"
Cohesion: 0.70
Nodes (4): bad(), ok(), smoke.sh script, step()

### Community 26 - "Cluster 26"
Cohesion: 0.70
Nodes (4): bad(), ok(), smoke.sh script, step()

### Community 27 - "Cluster 27"
Cohesion: 0.70
Nodes (4): bad(), ok(), smoke.sh script, step()

### Community 28 - "Cluster 28"
Cohesion: 0.70
Nodes (4): bad(), ok(), smoke.sh script, step()

### Community 29 - "Cluster 29"
Cohesion: 0.67
Nodes (3): ARGS, bench(), main()

### Community 30 - "Cluster 30"
Cohesion: 0.67
Nodes (3): ADR-150 constraint: MetaHarness as removable augmentation (4 rules), _harness.mjs shared subprocess helper (degraded JSON, 60s timeout), metaharness-architect agent (ADR-150 enforcement)

### Community 31 - "Cluster 31"
Cohesion: 0.67
Nodes (3): /intelligence dashboard command, intelligence-route skill, 3-tier model routing (codemod/Haiku/Sonnet-Opus)

## Knowledge Gaps
- **119 isolated node(s):** `CACHE_DIR`, `RESOLVED_CLI`, `DEFAULT_WEIGHTS`, `CATEGORICAL_FIELDS`, `ARGS` (+114 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `similarity()` connect `Harness Similarity Metric` to `Harness Scripts Core`, `Similarity Test Harness`?**
  _High betweenness centrality (0.025) - this node is a cross-community bridge._
- **Why does `rankSeverity()` connect `Harness Scripts Core` to `Similarity Test Harness`, `Cluster 14`?**
  _High betweenness centrality (0.010) - this node is a cross-community bridge._
- **Why does `parseMcpScanText()` connect `Harness Scripts Core` to `Similarity Test Harness`?**
  _High betweenness centrality (0.006) - this node is a cross-community bridge._
- **What connects `CACHE_DIR`, `RESOLVED_CLI`, `DEFAULT_WEIGHTS` to the rest of the system?**
  _123 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Harness Scripts Core` be split into smaller, more focused modules?**
  _Cohesion score 0.09565217391304348 - nodes in this community are weakly interconnected._
- **Should `AgentDB & Memory Bridge` be split into smaller, more focused modules?**
  _Cohesion score 0.08048780487804878 - nodes in this community are weakly interconnected._
- **Should `Similarity Test Harness` be split into smaller, more focused modules?**
  _Cohesion score 0.0625 - nodes in this community are weakly interconnected._