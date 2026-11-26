#![forbid(unsafe_code)]

pub mod a2a_engine;
pub mod adaptive_engine;
pub mod adversarial_bandit;
pub mod agent_loop;
pub mod anyonic_braiding;
pub mod anyonic_interferometry;
pub mod approvals;
pub mod arch_simd;
pub mod artifact_catalog;
pub mod assurance_receipt;
pub mod autopilot;
pub mod bandit_router;
pub mod blaze_adapter;
pub mod blind_mpc;
pub mod byzantine_aggregator;
pub mod canary_worktree;
pub mod causal_engine;
pub mod ckks_fhe;
pub mod cockpit;
pub mod cockpit_web;
pub mod coding_feedback;
pub mod colab_bridge;
pub mod composition_ledger;
pub mod config;
pub mod conformal_calibrator;
pub mod connector_engine;
pub mod containment;
pub mod content_guard;
pub mod context_engine;
pub mod context_recovery;
pub mod context_vfs;
pub mod cron_engine;
pub mod curator;
pub mod decision_engine;
pub mod device_trust;
pub mod distillation;
pub mod distributed_swarm;
pub mod efficiency_engine;
pub mod efficiency_store;
pub mod evaluation_engine;
pub mod evaluation_schedule;
pub mod evaluation_suite;
pub mod evolution;
pub mod execution_controller;
mod execution_profile;
pub mod executors;
pub mod federated_homomorphic;
pub mod federated_policy;
pub mod fleet;
pub mod formal_proof;
pub mod functional_encryption;
pub mod game_mechanism;
pub mod gateway;
pub mod gepa;
pub mod github_client;
pub mod grammar_mask;
pub mod graph_intelligence;
pub mod hardware_autopilot;
pub mod hardware_batch_dispatch;
pub mod harness_evolution;
pub mod harness_registry;
pub mod higher_order_topology;
pub mod hnsw;
pub mod hooks;
pub mod human_review;
pub mod hyper_vfs;
pub mod identity;
pub mod inference_infra;
pub mod job_store;
pub mod jump_diffusion;
pub mod kanban_engine;
pub mod koopman_operator;
pub mod kzg_commitment;
pub mod lats_engine;
pub mod lattice_zk_snark;
pub mod lsp_engine;
pub mod magic_state_distillation;
pub mod majorana_braiding;
pub mod martingale_gate;
pub mod mckean_vlasov_jump;
pub mod mcp;
pub mod mcp_catalog;
pub mod mean_field_control;
pub mod mean_field_game;
pub mod mean_field_master;
pub mod memory_engine;
pub mod memory_schedule;
pub mod merge_resolver;
pub mod merkle_dag;
pub mod merkle_rollup;
pub mod mesh_sync;
pub mod metacognition;
pub mod mock_cdp;
pub mod multi_agent_collab;
pub mod multi_agent_consensus;
pub mod multi_input_functional_encryption;
pub mod multi_tenant_mesh;
pub mod mutation;
pub mod neuromorphic_vfs;
pub mod non_hermitian_topology;
pub mod observability;
pub mod operations;
pub mod orchestration_engine;
pub mod orchestration_store;
pub mod pac_bayes_monitor;
pub mod package_admission;
pub mod package_distribution;
pub mod package_store;
pub mod parliament;
pub mod path_integral_thermodynamics;
pub mod permit_store;
pub mod pexer_ipc;
pub mod port_hamiltonian;
pub mod pq_attestation;
pub mod privacy_budget;
pub mod privacy_engine;
pub mod proactive_secret_redistribution;
pub mod prompt_engine;
pub mod prp_engine;
pub mod quantum_annealer;
pub mod quantum_geometric_tensor;
pub mod quantum_mip;
mod receipt_id;
pub mod recovery_archive;
pub mod recursive_zk_rollup;
pub mod reference_monitor;
pub mod reflexion;
pub mod registry_client;
pub mod replacement;
pub mod research_artifact;
pub mod rollout_reducer;
pub mod run_loop;
pub mod secret_vault;
pub mod self_healing;
pub mod self_healing_kernel;
pub mod semantic_indexer;
pub mod semantic_memory;
pub mod service;
pub mod service_token;
pub mod session_journal;
pub mod sessions;
pub mod shadow_council;
pub mod skill_engine;
pub mod society_engine;
pub mod spectral_topology;
pub mod speculative_decoding;
pub mod speculative_drafting;
pub mod speculative_tree;
pub mod storage_lifecycle;
pub mod strategies;
pub mod subagent;
pub mod subagent_store;
pub mod subsystems;
pub mod swarm_bft;
pub mod swarm_crdt;
pub mod swarm_mesh;
pub mod swarm_mpc;
pub mod swarm_pipeline;
pub mod symplectic_neural_ode;
pub mod synthetic_canvas;
pub mod thermodynamic_uncertainty;
pub mod threshold_fhe;
pub mod tool_engine;
pub mod topological_qec;
pub mod topological_quantum_memory;
pub mod tqft_invariants;
pub mod tripartite_governance;
pub mod tropical_dag;
pub mod tropical_nn;
pub mod verifiable_delay;
pub mod visual_graph;
pub mod voyager_curriculum;
pub mod wasm;
pub mod wasserstein_drift;
pub mod work_stealing;
pub mod worktree_isolation;
pub mod zk_attestation;
pub mod zk_execution_attestation;

#[cfg(test)]
mod test_support;

pub use a2a_engine::A2aEngine;
pub use adaptive_engine::{AdaptationResult, AdaptiveEngine, AdaptiveError};
pub use adversarial_bandit::{
    AdversarialBanditError, Exp3AdversarialBandit, Exp3ArmState, Exp3BanditStatus,
};
pub use agent_loop::{
    AgentApprovalContext, AgentCheckpoint, AgentCheckpointKind, AgentControlStop, AgentLoop,
    AgentLoopError, AgentRunControl, AgentRunRequest, AgentRunSummary, LatsLoopConfig,
    LatsTurnTelemetry, MAX_AGENT_TOOL_CALLS, MAX_AGENT_TURNS, SubagentRunControl,
};
pub use approvals::{
    ApprovalError, ApprovalRequest, ApprovalStatus, ApprovalStore, PendingApproval,
};
pub use arch_simd::{
    CpuFeatures, DimensionBenchmarkStats, SimdBenchmarkResult, benchmark_simd_vs_scalar,
    compute_sample_stats, run_statistical_microbenchmarks, scalar_dot_product_128,
    scalar_dot_product_slice, simd_dot_product_128, simd_dot_product_f32_i8, simd_dot_product_i8,
    simd_dot_product_slice, simd_hash64_seed, simd_norm_sq_128, simd_tropical_max_plus_128,
    student_t_critical_value_95,
};
pub use artifact_catalog::{ArtifactActivation, ArtifactCatalog, ArtifactCatalogError};
pub use assurance_receipt::{PhaseCertificate, UnifiedAssuranceReceipt};
pub use autopilot::{
    AutopilotCoordinator, AutopilotError, AutopilotExecutionResult, AutopilotGoal,
    AutopilotSessionConfig, AutopilotSessionOutcome, CertifiedPatchRecord,
    MultiAgentAutopilotOrchestrator, certify_patch, role_for_agent,
};
pub use bandit_router::{
    BanditArm, BanditError, BanditModelRouter, BanditRoleArm, BanditStrategy,
    ContextualThompsonSamplingRouter, DraftOutcome, LinTsArmState, RoleCapability, RoutingContext,
    RoutingReward, SafePrng, SpeculativeRoutingPlan, TASK_COMPLEXITY_DIM, TaskComplexityFeatures,
    TropicalComplexityPrior,
};
pub use blaze_adapter::{
    BlazeBuildResult, BlazeCompilerAdapter, BlazeConfig, BlazeDiagnostics, DiagnosticLevel,
};
pub use byzantine_aggregator::{
    AgentVectorUpdate, AggregationRule, ByzantineAggregator, ByzantineConsensusResult,
    ByzantineError,
};
pub use canary_worktree::{
    CanaryPredicate, CanaryStagePipeline, CanaryStagingReceipt, CanaryVerificationResult,
    CosignCanaryAttestation, GitWorktreeManager, IsolatedWorktree, IsolatedWorktreeGuard,
    WorktreeError as CanaryWorktreeError, WorktreeStatus as CanaryWorktreeStatus,
};
pub use cockpit_web::{
    ApproveActionRequest, CockpitWebConfig, CockpitWebServer, ConsensusActionRequest,
    DASHBOARD_HTML, DEFAULT_SSE_TICK_MS, LayoutActionRequest, MAX_HTTP_BODY_BYTES,
    format_sse_event,
};
pub use coding_feedback::{
    CodingFeedbackError, CodingFeedbackInput, CodingFeedbackLoop, CodingFeedbackResult,
};
pub use composition_ledger::{
    COMPOSITION_LEDGER_VERSION, CompositionBinding, CompositionLedger, CompositionLedgerError,
    CompositionSource, MAX_COMPOSITION_BINDINGS,
};
pub use config::{
    ConfigOverrides, McpAuthType, McpHttpConfig, McpOAuthConfig, McpServerTarget, RuntimeConfig,
};
pub use conformal_calibrator::{
    AdaptiveConformalStatus, AdaptiveConformalTracker, CandidateAction,
    ConformalCalibrationSummary, ConformalError, ConformalGovernanceVerdict,
    ConformalPredictionReport, SplitConformalCalibrator,
};
pub use connector_engine::{ConnectorEngine, ConnectorError};
pub use containment::shipped_executor_containment;
pub use content_guard::{
    CONTENT_GUARD_POLICY_VERSION, CONTEXT_QUARANTINE_REASON, ContentGuardError,
    TOOL_QUARANTINE_REASON, UntrustedContentAssessment, UntrustedContentDisposition,
    assess_untrusted_content, guard_context_fragments, normalize_untrusted_tool_payload,
    render_untrusted_tool_payload, tool_output_origin_kind,
};
pub use context_engine::{ContextCacheStats, ContextEngine, ContextError};
pub use context_recovery::{ContextRecovery, RecoveryDecision, RecoveryInput, RecoveryStep};
pub use context_vfs::{
    CachedBufferEntry, ContextSearchOptions, ContextVfs, DeserializedVfsIndex, HybridSearchResult,
    SearchResultItem, VFS_INDEX_VERSION, VFS_INDEX_VERSION_V1, VFS_INDEX_VERSION_V2,
    VFS_INDEX_VERSION_V3, VFS_QUANTIZED_VECTOR_SIZE, VfsDoc, VfsError, VfsIndexStats,
    compute_corpus_hash, compute_doc_fingerprint, deserialize_vfs_index, read_index_stats,
    serialize_vfs_index, serialize_vfs_index_v3,
};
pub use cron_engine::{
    CronEngine, CronError, CronExecutionResult, CronJob, CronJobStatus, CronSchedule,
    handle_cronjob_manage_action,
};
pub use curator::{CuratorEngine, CuratorError};
pub use decision_engine::{
    CalibrationAttestationReceipt, CustomApiDecisionDriver, DecisionDriver, DecisionEngine,
    DecisionError, DecisionFeatures, DpoPair, FineTuningSample, GoldenEvaluationCase,
    GoldenSuiteDefinition, GoldenTargetRef, HeuristicDecisionDriver, MockDecisionDriver,
    MoiraDecisionDriver, OllamaDecisionDriver, SinglePassLogitEngine, TropicalDecisionDriver,
    compute_calibration_commitment, compute_dpo_dataset_digest, normalized_entropy,
    should_escalate as decision_should_escalate,
};
pub use device_trust::{
    DeviceKeyError, DeviceKeyStore, DeviceProofRequest, device_proof_message, verify_device_proof,
};
pub use distillation::{
    AggregatedDpoDataset, AttestationVerificationReport, AttestationVerifyConfig,
    CrossSessionDistillConfig, CrossSessionDistillResult, DistillationError, aggregate_dpo_pairs,
    attestation_receipt_path, default_capsule_path, default_telemetry_path, discover_dpo_sources,
    distill_cross_session_checkpoint, load_telemetry_snapshot, save_telemetry_snapshot,
    verify_pnn_and_capsule_attestation,
};
pub use distributed_swarm::{
    DistributedConsensusDecision, DistributedConsensusVote, DistributedFederatedEngine,
    DistributedGradientPacket, DistributedPatchProposal, DistributedSwarmConsensusEngine,
    DistributedSwarmError, DistributedSwarmMessage, DistributedSwarmNode, SwarmPayload, SwarmPeer,
};
pub use efficiency_engine::{DEFAULT_MAX_SAMPLES_PER_TARGET, EfficiencyEngine, EfficiencyError};
pub use efficiency_store::{EfficiencyStore, EfficiencyStoreError};
pub use evaluation_engine::{
    ArtifactScorecard, EvaluationAdapterError, EvaluationEngine, EvaluationError, EvaluationTarget,
    EvaluationTargetKind, EvaluationTaskAdapter, EvaluationTaskResult, GoldenCase,
    GoldenCaseResult, GoldenSetReport, HoldoutCase, HoldoutCaseResult, HoldoutSetReport,
    MAX_EVALUATION_TARGET_ID_BYTES, MAX_EVALUATION_TASK_BYTES, MAX_GOLDEN_CASE_ID_BYTES,
    MAX_GOLDEN_CASES, MAX_GOLDEN_EXPECTED_OUTPUT_BYTES, MAX_HOLDOUT_CASE_ID_BYTES,
    MAX_HOLDOUT_CASES, MAX_HOLDOUT_OUTPUT_BYTES, TaskBackedCase,
};
pub use evaluation_schedule::{
    EvaluationSchedule, EvaluationScheduleError, EvaluationScheduleRun,
    EvaluationScheduleRunEvidence, EvaluationScheduleRunStatus, EvaluationScheduleStore,
    MAX_CLAIM_BATCH, MAX_EVALUATION_SUITE_BYTES, MAX_SCHEDULE_RUN_HISTORY, MAX_SCHEDULES,
    SCHEDULE_LEASE_SECONDS,
};
pub use evaluation_suite::{
    EvaluationSuite, EvaluationSuiteError, EvaluationSuiteStore, MAX_EVALUATION_DEFINITION_BYTES,
    MAX_EVALUATION_SUITE_ID_BYTES, MAX_EVALUATION_SUITES, MAX_REGRESSION_CANDIDATE_ID_BYTES,
    MAX_REGRESSION_CANDIDATES, RegressionCandidate, RegressionCandidateStatus,
};
pub use evolution::{
    BayesianCanaryGate, BayesianCanaryVerdict, EvolutionEngine, EvolutionError, EvolutionRecord,
    EvolutionStepReport, Island, IslandCandidate, IslandModelEvolver, IslandNiche, IslandRunReport,
    IslandSummary, MigrationExchange, MigrationReport, MutationKind, ParetoCandidate,
    ParetoFrontier, default_evolution_sop,
};
pub use execution_controller::{
    ExecutionController, RunStatus, RunSummary, RuntimeError, WorktreeExecutionContext,
};
pub use execution_profile::ExecutionProfileAssemblyError;
pub use executors::{
    GitWorktreeExecutor, ProviderCallMetrics, WorktreeChange, WorktreeCommand, WorktreeError,
    WorktreeResult,
};
pub use federated_policy::{
    AgentReputationRecord, BYZANTINE_BETA_PENALTY, CRITICAL_REPUTATION_THRESHOLD,
    DEFAULT_LEARNING_RATE, DEFAULT_MAX_GRADIENT_NORM, DEFAULT_MOMENTUM, FederatedPolicyEngine,
    FederatedPolicyError, FederatedUpdateReceipt, MIN_EVALS_FOR_QUARANTINE,
    POLICY_DIMENSION_LABELS, POLICY_DIMENSIONS, PolicyGradientVector, QUARANTINE_WEIGHT_PENALTY,
};
pub use fleet::{
    FLEET_SCHEMA_VERSION, FleetBudget, FleetEngine, FleetError, FleetFenceState, FleetLease,
    FleetLeaseFence, FleetLeaseState, FleetNode, FleetNodeState, FleetQuiescenceGuard,
    FleetSupervisor, FleetSupervisorState, MAX_FLEET_CAPABILITIES, MAX_FLEET_FENCES,
    MAX_FLEET_LEASES, MAX_FLEET_NODES,
};
pub use gateway::{
    AuditDagEntry, AuditLeafEntry, AuditStoreFile, AuditTelemetryEvent, AutonomousAction,
    CircuitBreaker, CircuitBreakerState, DagTopologyTelemetry, ExecutionTelemetry, GatewayConfig,
    GatewayController, GatewayError, GatewayTickReport, SelfHealingAction,
};
pub use gepa::{
    GepaCandidate, GepaError, GepaFeedbackTrace, GepaObjective, GepaOptimizer, GepaParetoFrontier,
    GepaReflectiveMutator,
};
pub use github_client::{GitHubPackageClient, GitHubPackageDownload, GitHubPackageError};
pub use grammar_mask::{
    GrammarKind, GrammarMaskEngine, GrammarMaskResult, GrammarSynthesisReport, PushdownFrame,
};
pub use graph_intelligence::{
    GraphEdge, GraphError, GraphInput, GraphIntelligenceEngine, GraphKind, GraphNode,
    GraphNodeKind, GraphScope, GraphSnapshot, GraphStore, GraphStoreError, MAX_GRAPH_EDGES,
    MAX_GRAPH_INPUT_BYTES, MAX_GRAPH_INPUTS, MAX_GRAPH_NODES, MAX_GRAPH_SNAPSHOT_BYTES,
};
pub use hardware_autopilot::HardwareAutopilot;
pub use hardware_batch_dispatch::{BatchVectorDispatcher, Precision, RooflineModel};
pub use harness_evolution::{
    EvolutionTestCase, GenomeEvaluationReceipt, HarnessEvolutionEngine, TestCaseResult,
};
pub use harness_registry::{
    HarnessRegistry, HarnessRegistryError, PackageRecord, PackageState, PublisherTrustRoot,
    PublisherTrustRoots,
};
pub use hnsw::{
    DEFAULT_HNSW_EF_CONSTRUCTION, DEFAULT_HNSW_EF_SEARCH, DEFAULT_HNSW_M, DEFAULT_HNSW_M_MAX_0,
    HnswConfig, HnswError, HnswIndex, HnswNode, HnswSearchResult, HnswVector, HnswVectorType,
    PVHN_HEADER_SIZE, PVHN_MAGIC, PVHN_TRAILER_SIZE, PVHN_VERSION, cosine_distance,
    tropical_distance,
};
pub use hooks::{HookPoint, HookSelector, HooksLoadError, LifecycleHook, LifecycleHooks};
pub use human_review::{
    HumanMode, HumanReviewEngine, ReviewDecision, ReviewError, ReviewReceipt, ReviewRecord,
    ReviewState, ReviewSubject,
};
pub use identity::{
    AccessRole, IdentityEnrollment, IdentityEnrollmentRequest, IdentityStore, IdentityStoreError,
    ServiceIdentity,
};
pub use job_store::{JobRecord, JobStore, JobStoreError, MAX_JOB_RESULT_BYTES};
pub use kanban_engine::{KanbanEngine, KanbanError};
pub use lats_engine::{
    LatsEngine, LatsError, LatsMctsEngine, LatsMemoEntry, LatsMemoKey, LatsNode,
    LatsPrefixMemoCache, LatsSearchMetrics, LatsSearchResult, convergence_choice,
    cosine_similarity, should_escalate, should_escalate_with_entropy,
};
pub use lsp_engine::{LspClientConfig, LspEngine, LspError, LspSession};
pub use martingale_gate::{
    CanaryGateVerdict, MartingaleCanaryGate, MartingaleError, MartingaleGateSnapshot,
};
pub use mcp::{
    MCP_LEGACY_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION, McpError, McpFailure, McpInvocation,
    McpProtocolMode, McpServer, McpStart, McpStdioConfig, McpToolResult, McpWireEra,
};
pub use mcp_catalog::{McpCatalogRevision, McpCatalogTool};
pub use mean_field_game::{
    MeanFieldCongestionStatus, MeanFieldGame, MfgAgentCostState, MfgCongestionModel, MfgEngine,
    MfgEquilibriumResult, MfgError, MfgFixedPointResult, MfgResourceConfig, PoABoundResult,
    PriceOfAnarchyReport, THEORETICAL_POA_UPPER_BOUND, project_onto_simplex,
};
pub use memory_engine::{
    MAX_SYNTHESIS_SOURCE_RECORDS, MemoryEngine, MemoryError, MemorySynthesisProposal,
    MemorySynthesisSnapshot,
};
pub use memory_schedule::{
    MAX_MEMORY_SYNTHESIS_CLAIM_BATCH, MAX_MEMORY_SYNTHESIS_RUNS,
    MAX_MEMORY_SYNTHESIS_SCHEDULE_NAME_BYTES, MAX_MEMORY_SYNTHESIS_SCHEDULES,
    MAX_MEMORY_SYNTHESIS_SUMMARY_BYTES, MEMORY_SYNTHESIS_SCHEDULE_LEASE_SECONDS,
    MemorySynthesisSchedule, MemorySynthesisScheduleError, MemorySynthesisScheduleRun,
    MemorySynthesisScheduleRunStatus, MemorySynthesisScheduleStore,
};
pub use merkle_dag::{
    DagError, DagNode, InclusionProof, MerkleDagAuditLog, MerkleProofStep, NodeDirection, TAG_DAG,
    TAG_LEAF, TAG_NODE,
};
pub use merkle_rollup::{
    EpochRollupHeader, RollupDecisionEntry, RollupEpoch, RollupError, TAG_EPOCH_HEADER,
    TAG_ROLLUP_LEAF, decode_hex_32, encode_hex,
};
pub use mesh_sync::{
    CapsuleValidationVerdict, MeshError, MeshHandshake, MeshPeerEntry, MeshSyncEngine,
    MeshSyncResult, PeerIdentity, SignedCapsulePackage,
};
pub use metacognition::{
    MetacognitiveAuditReport, MetacognitiveFrame, MetacognitiveSupervisor, MetacognitiveVerdict,
};
pub use mock_cdp::{CdpError, CdpResult, ElementRect, MockCdpServer, VirtualDocument, VirtualNode};
pub use mutation::{MutationEngine, MutationError};
pub use observability::{
    DEFAULT_MAX_OBSERVABILITY_SAMPLES, ObservabilityEngine, ObservabilityError,
};
pub use operations::{OperationalEvent, OperationalRecorder, OperationalStatus};
pub use orchestration_engine::{
    DiamondReceipt, DomainProfileRun, EdgeVerificationError, EdgeVerifierPolicy,
    OrchestrationEngine, OrchestrationError, OrchestrationPlanJson, OrchestrationRun,
    OrchestrationRunSnapshot,
};
pub use orchestration_store::{
    OrchestrationBudgetReservationRecord, OrchestrationBudgetReservationState,
    OrchestrationBudgetSnapshot, OrchestrationRunRecord, OrchestrationRunStatus,
    OrchestrationStore, OrchestrationStoreError,
};
pub use pac_bayes_monitor::{
    GibbsPosteriorResult, PacBayesCertificate, PacBayesConfig, PacBayesEngine, PacBayesError,
    PacBayesMonitor, PolicyPromotionGate, PolicyPromotionVerdict, PromotionGateVerdict,
    RiskCertificate, binary_kl,
};
pub use package_distribution::{
    DistributionBinding, DistributionEvent, DistributionEventKind, DistributionMutation,
    DistributionRecord, DistributionSource, DistributionSourceKind, DistributionState,
    MAX_DISTRIBUTION_EVENTS, MAX_DISTRIBUTION_LIST, MAX_DISTRIBUTION_RECORDS,
    PackageDistributionError, PackageDistributionStore, SkillDistributionBundle,
    SkillDistributionFile, materialize_skill_bundle,
};
pub use package_store::{
    MAX_PACKAGE_TRANSPARENCY_EVENTS, MAX_PACKAGE_TRANSPARENCY_LIST, MAX_PUBLISHER_TRUST_ROOTS,
    MAX_STORED_ARTIFACT_BYTES, PackageBinding, PackageStore, PackageStoreError,
    PackageTransparencyEvent, PackageTransparencyEventKind, PackageTransparencyOutcome,
    PublisherTrustRootRecord,
};
pub use parliament::{Parliament, ToolSafetyGateVerdict, ToolSafetyReflexVerdict};
pub use permit_store::{ConsumedPermit, PermitError, PermitStore};
pub use pq_attestation::{
    HybridAttestationReceipt, HybridAttestationSigner, HybridVerifyingKey, LatticeParams,
    PoseidonSpongeEngine,
};
pub use privacy_budget::{
    DistributedPrivacyTracker, GaussianMechanism, LaplaceMechanism, PrivacyAccountantError,
    PrivacyError, PrivacyExpenditure, QueryReservation, RDP_ORDER_KEY_SCALE, RdpBudget, RdpOrder,
    STANDARD_RDP_ORDERS, key_to_order, order_to_key,
};
pub use privacy_engine::{PrivacyBudget, PrivacySupervisor, ScrubbingReceipt};
pub use prompt_engine::{
    PresetKind, PromptEngine, PromptError, PromptPreset, StorageTier, validate_preset_name,
};
pub use prp_engine::PrpEngine;
pub use recovery_archive::{RecoveryArchive, RecoveryArchiveError, RecoveryBundle, RecoveryEntry};
pub use reference_monitor::{AuthorizationError, ReferenceMonitor};
pub use reflexion::{
    ParameterizedLiteral, ReflexionCrystallizeResult, ReflexionCrystallizer, ReflexionDiagnostic,
    ReflexionDiagnosticEngine, ReflexionError, ReflexionFailureKind,
};
pub use registry_client::{PackageRegistryClient, PackageRegistryError, RegistryPackageDownload};
pub use replacement::{CANARY_POLICY_VERSION, CanaryPolicy, ReplacementEngine, ReplacementError};
pub use research_artifact::{
    MAX_RESEARCH_ARTIFACT_BYTES, ResearchArtifactError, ResearchArtifactRecord,
    ResearchArtifactStore, ResearchCandidateRecord,
};
pub use rollout_reducer::{RolloutReducer, RolloutReducerError};
pub use run_loop::{RunLoop, RunLoopError};
pub use secret_vault::{
    SecretVault, SecretVaultEntry, SecretVaultError, StoredMcpAuth, VaultSecret,
};
pub use self_healing::{SelfHealingEngine, SelfHealingError};
pub use semantic_indexer::{
    DEFAULT_HYBRID_ALPHA, DEFAULT_TROPICAL_GAMMA, SemanticIndexer, TROPICAL_DIMENSION,
    TropicalHybridRanker, TropicalProjector, TropicalSemanticProjector, TropicalVector,
};
pub use semantic_memory::{
    BM25_B, BM25_K1, DENSE_DIMENSION, DenseFeatureHasher, HybridRankItem, HybridRrfRanker,
    LessonRecord, QuantizedVector, RRF_K, SparseBm25Index, extract_term_frequencies, tokenize,
};
pub use service::{RuntimeService, RuntimeServiceError, RuntimeServiceScope};
pub use service_token::{ServiceToken, ServiceTokenError, ServiceTokenStore};
pub use session_journal::{
    ForkResult, JournalStep, RewindResult, SessionJournalEngine, SessionStepDiff,
    compute_step_digest,
};
pub use sessions::{RolloutSummary, SessionEventPage, SessionSnapshot, SessionStore};
pub use skill_engine::{
    RemovalReceipt, SkillEngine, SkillError, SkillInspection, SkillProvenance, SkillRecord,
    SkillState,
};
pub use society_engine::SocietyEngine;
pub use spectral_topology::{
    SpectralTopologyAnalyzer, SpectralTopologyError, SpectralTopologyReport, TopologyVerdict,
    WeightedEdge,
};
pub use speculative_decoding::{
    DraftVerificationResult, RadixKvCache, RadixKvNode, SpeculativeEngine, TokenProbability,
};
pub use speculative_drafting::{
    AstNodeKind, CertifiedDraft, DiffHunk, DiffLine, FrontierCriticEvaluation,
    ResidualDiffComposer, SourceSpan, SpeculativeAstNode, SpeculativeDiffDraft,
    SpeculativeDraftingEngine, SpeculativeError, UnifiedPatch, create_unified_diff,
    extract_ast_nodes, mask_comments_and_strings,
};
pub use speculative_tree::{SpeculativeAuditReport, SpeculativeTreeEngine, SpeculativeTreeNode};
pub use storage_lifecycle::{
    MAX_STORAGE_LIFECYCLE_LIST, MAX_STORAGE_LIFECYCLE_RECEIPTS, StorageLifecycleReceipt,
    StorageLifecycleRecordResult, StorageLifecycleStore, StorageLifecycleStoreError,
};
pub use strategies::population::{
    PopulationParentPlan, PopulationPlan, PopulationStrategy, PopulationStrategyError,
};
pub use strategies::{StrategyBudget, StrategyError, StrategyProfile};
pub use subagent::{
    SubagentCleanupContext, SubagentCoordinator, SubagentCoordinatorError, SubagentSpawnContext,
};
pub use subagent_store::{
    ClaimedSubagent, SubagentPreparation, SubagentRecord, SubagentScope, SubagentStore,
    SubagentStoreError,
};
pub use swarm_crdt::{
    CausalOrdering, EpidemicGossipCluster, MerkleDagCrdt, MerkleDagNode, OrSet, PnCounter,
    VectorClock as CrdtVectorClock,
};
pub use swarm_mesh::{
    BftConsensus, BloomFilter, ConsensusDecision, GossipMessage, GossipNode, Proposal, SwarmError,
    TaskDag, TaskNode, VectorClock, Vote,
};
pub use swarm_mpc::{BeaverTriple, MpcAuditReport, SecretShare, ShamirMpcEngine, mod_inv, mod_pow};
pub use synthetic_canvas::{PNG_SIGNATURE, SyntheticCanvas, adler32, crc32};
pub use tool_engine::{
    ToolContext, ToolDefinition, ToolEngine, ToolError, ToolInvocation, ToolPlan, ToolProfile,
};
pub use tripartite_governance::{
    TripartiteApproval, TripartiteError, TripartiteEvaluation, TripartiteGovernance,
    TripartiteProposal, TripartiteRole, branch_name_for_proposal,
};
pub use tropical_dag::{
    KarpCycleMeanResult, KleeneStarResult, MaxPlusMatrix, NEG_INFINITY, TropicalDagEngine,
    TropicalDagScheduler, TropicalError, TropicalMatrix,
};
pub use tropical_nn::{
    TropicalError as TropicalNnError, TropicalLayer, TropicalNetwork, extract_ast_features,
};
pub use visual_graph::{
    AsciiGridRenderer, ForceDirectedLayout, Point2D, ViewportCamera, VisualNode,
};
pub use voyager_curriculum::{CurriculumDifficulty, CurriculumSummary, VoyagerCurriculum};
pub use wasm::{
    DEFAULT_WASM_FUEL, MAX_WASM_INPUT_BYTES, MAX_WASM_MEMORY_BYTES, MAX_WASM_OUTPUT_BYTES,
    WasmError, WasmExecutor, WasmGene, WasmGeneRequest, WasmResult,
};
pub use wasserstein_drift::{
    EmpiricalDistribution, SinkhornDriftDetector, SinkhornDriftReport, WassersteinError,
};
pub use work_stealing::{
    AdaptiveBackoff, BASE_STEAL_BACKOFF_US, BL_CRITICAL_PATH_COEFFICIENT, BoundedTaskDeque,
    DEFAULT_DEQUE_CAPACITY, MAX_STEAL_BACKOFF_US, SchedulerError, SchedulerTelemetry,
    StealableTask, StealableTaskDag, TaskStatus, WorkStealingScheduler,
};
pub use worktree_isolation::{WorktreeIsolationManager, strip_unc};
pub use zk_attestation::{
    FiatShamirTranscript, FieldElement, GOLDILOCKS_GENERATOR, GOLDILOCKS_PRIME,
    GOLDILOCKS_TWO_ADICITY, Polynomial, StateAttestationProof, StateTransitionAir,
    TRANSCRIPT_DOMAIN_LABEL, ZkAttestationError, ZkError,
};
pub use zk_execution_attestation::{
    DEFAULT_EXECUTION_DOMAIN_SIZE, EXECUTION_TRANSCRIPT_DOMAIN, PrivateExecutionWitness,
    PublicExecutionInputs, ZkExecutionAir, ZkExecutionProof,
};
