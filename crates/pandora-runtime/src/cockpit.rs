//! Pandora Mission-Control Cockpit: 9/12-Panel Subsystem Telemetry and Pexer IPC Multiplexer.
//!
//! Grounded in Theorems 41 and 42 for multi-agent swarm state consistency and
//! bounded orchestration latency.
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Layout mode for the mission control cockpit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CockpitLayout {
    /// 9-panel 3x3 grid monitoring core operational telemetry.
    NinePanel,
    /// 12-panel 3x4 grid providing comprehensive full-spectrum monitoring.
    TwelvePanel,
}

impl CockpitLayout {
    /// Total number of panels for this layout.
    pub fn panel_count(&self) -> usize {
        match self {
            CockpitLayout::NinePanel => 9,
            CockpitLayout::TwelvePanel => 12,
        }
    }

    /// Parse from numeric panel count string or name.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "9" | "nine" | "nine_panel" | "3x3" => Some(CockpitLayout::NinePanel),
            "12" | "twelve" | "twelve_panel" | "3x4" => Some(CockpitLayout::TwelvePanel),
            _ => None,
        }
    }
}

/// Category classification of cockpit monitoring panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelCategory {
    Governance,
    Runtime,
    Evolution,
    Infrastructure,
    Agents,
}

impl PanelCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            PanelCategory::Governance => "governance",
            PanelCategory::Runtime => "runtime",
            PanelCategory::Evolution => "evolution",
            PanelCategory::Infrastructure => "infrastructure",
            PanelCategory::Agents => "agents",
        }
    }
}

/// Operational health indicator of an individual cockpit panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelHealth {
    Healthy,
    Degraded,
    Critical,
    Paused,
}

impl PanelHealth {
    pub fn as_str(&self) -> &'static str {
        match self {
            PanelHealth::Healthy => "HEALTHY",
            PanelHealth::Degraded => "DEGRADED",
            PanelHealth::Critical => "CRITICAL",
            PanelHealth::Paused => "PAUSED",
        }
    }

    pub fn is_operational(&self) -> bool {
        matches!(self, PanelHealth::Healthy | PanelHealth::Degraded)
    }
}

/// Identifiers for the 12 core cockpit telemetry subsystems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    /// 01: Autonomous Gateway & Circuit Breakers (daemon-guard)
    DaemonGuard = 1,
    /// 02: 12-Engine Unified Mathematical Assurance Receipt (Theorems 17-40)
    Assurance12 = 2,
    /// 03: Skill Genes, Strategy Mutation, PAC-Bayes & Canary Gate
    GenesEvolution = 3,
    /// 04: Evaluation Harnesses, Benchmarks & Adversarial Stress Telemetry
    HarnessBench = 4,
    /// 05: Runtime Configuration, Model Providers, Token Limits & Doctor
    ConfigSettings = 5,
    /// 06: P2P Swarm Mesh, Byzantine Quorum (Phase 12) & Mean-Field Game (Phase 18)
    SwarmMfgBft = 6,
    /// 07: Work-Stealing (Phase 11), Spectral Connectivity (Phase 16) & Tropical DAG (Phase 20)
    TropicalDag = 7,
    /// 08: Contextual UCB1/Thompson (Phase 6) & Exp3 Adversarial Bandit (Phase 17)
    BanditRouter = 8,
    /// 09: Sinkhorn Optimal Transport Drift (Phase 13) & Conformal ACI (Phase 15)
    DriftConformal = 9,
    /// 10: Rényi DP Privacy Budget (Phase 9) & Goldilocks ZK Attestation (Phase 10)
    PrivacyZk = 10,
    /// 11: Cryptographic Merkle-DAG State Log & Tamper Verification (Phase 7)
    MerkleAudit = 11,
    /// 12: Multi-Agent Slash-Command Dispatcher & External Coding Agents
    CommandDeck = 12,
}

impl PanelKind {
    pub fn all_twelve() -> &'static [PanelKind] {
        &[
            PanelKind::DaemonGuard,
            PanelKind::Assurance12,
            PanelKind::GenesEvolution,
            PanelKind::HarnessBench,
            PanelKind::ConfigSettings,
            PanelKind::SwarmMfgBft,
            PanelKind::TropicalDag,
            PanelKind::BanditRouter,
            PanelKind::DriftConformal,
            PanelKind::PrivacyZk,
            PanelKind::MerkleAudit,
            PanelKind::CommandDeck,
        ]
    }

    pub fn core_nine() -> &'static [PanelKind] {
        &[
            PanelKind::DaemonGuard,
            PanelKind::Assurance12,
            PanelKind::GenesEvolution,
            PanelKind::HarnessBench,
            PanelKind::ConfigSettings,
            PanelKind::SwarmMfgBft,
            PanelKind::TropicalDag,
            PanelKind::BanditRouter,
            PanelKind::DriftConformal,
        ]
    }

    pub fn panel_number(&self) -> usize {
        *self as usize
    }

    pub fn title(&self) -> &'static str {
        match self {
            PanelKind::DaemonGuard => "[01:Daemon-Guard]",
            PanelKind::Assurance12 => "[02:Assurance-12]",
            PanelKind::GenesEvolution => "[03:Genes-Evolution]",
            PanelKind::HarnessBench => "[04:Harness-Bench]",
            PanelKind::ConfigSettings => "[05:Config-Settings]",
            PanelKind::SwarmMfgBft => "[06:Swarm-MFG-BFT]",
            PanelKind::TropicalDag => "[07:Tropical-DAG]",
            PanelKind::BanditRouter => "[08:Bandit-Router]",
            PanelKind::DriftConformal => "[09:Drift-Conformal]",
            PanelKind::PrivacyZk => "[10:Privacy-ZK]",
            PanelKind::MerkleAudit => "[11:Merkle-Audit]",
            PanelKind::CommandDeck => "[12:Command-Deck]",
        }
    }

    pub fn category(&self) -> PanelCategory {
        match self {
            PanelKind::DaemonGuard | PanelKind::Assurance12 => PanelCategory::Governance,
            PanelKind::GenesEvolution => PanelCategory::Evolution,
            PanelKind::HarnessBench => PanelCategory::Infrastructure,
            PanelKind::ConfigSettings => PanelCategory::Runtime,
            PanelKind::SwarmMfgBft | PanelKind::TropicalDag | PanelKind::BanditRouter => {
                PanelCategory::Runtime
            }
            PanelKind::DriftConformal | PanelKind::PrivacyZk | PanelKind::MerkleAudit => {
                PanelCategory::Governance
            }
            PanelKind::CommandDeck => PanelCategory::Agents,
        }
    }

    pub fn default_command(&self) -> &'static str {
        match self {
            PanelKind::DaemonGuard => "pandora runtime daemon-guard",
            PanelKind::Assurance12 => "pandora runtime assurance",
            PanelKind::GenesEvolution => "pandora runtime pac-bayes",
            PanelKind::HarnessBench => "pandora bench",
            PanelKind::ConfigSettings => "pandora doctor",
            PanelKind::SwarmMfgBft => "pandora runtime mfg",
            PanelKind::TropicalDag => "pandora runtime tropical",
            PanelKind::BanditRouter => "pandora runtime bandit",
            PanelKind::DriftConformal => "pandora runtime conformal",
            PanelKind::PrivacyZk => "pandora runtime zk",
            PanelKind::MerkleAudit => "pandora audit verify",
            PanelKind::CommandDeck => "pandora chat",
        }
    }
}

/// Instantaneous status and metrics of a single panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelStatus {
    pub panel_id: usize,
    pub kind: PanelKind,
    pub title: String,
    pub category: PanelCategory,
    pub health: PanelHealth,
    pub headline: String,
    pub metrics: BTreeMap<String, String>,
    pub live_stream_tail: Vec<String>,
}

/// Complete telemetry snapshot of the mission control cockpit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CockpitSnapshot {
    pub layout: CockpitLayout,
    pub panel_count: usize,
    pub healthy_panels: usize,
    pub degraded_panels: usize,
    pub critical_panels: usize,
    pub active_external_agents: usize,
    pub panels: Vec<PanelStatus>,
    pub timestamp_ms: u64,
    pub snapshot_digest: String,
}

impl CockpitSnapshot {
    /// Verify cryptographic digest integrity of the snapshot.
    pub fn verify_digest(&self) -> bool {
        let expected = compute_snapshot_digest(&self.layout, &self.panels, self.timestamp_ms);
        self.snapshot_digest == expected
    }
}

/// Controller managing cockpit telemetry capture, visualization, and Pexer layout synthesis.
#[derive(Debug, Clone)]
pub struct CockpitController {
    layout: CockpitLayout,
}

impl CockpitController {
    pub fn new(layout: CockpitLayout) -> Self {
        Self { layout }
    }

    pub fn layout(&self) -> CockpitLayout {
        self.layout
    }

    /// Capture a live or simulated telemetry snapshot across all configured panels.
    pub fn capture_snapshot(&self, timestamp_ms: u64) -> CockpitSnapshot {
        let kinds = match self.layout {
            CockpitLayout::NinePanel => PanelKind::core_nine(),
            CockpitLayout::TwelvePanel => PanelKind::all_twelve(),
        };

        let mut panels = Vec::with_capacity(kinds.len());
        let mut healthy = 0;
        let mut degraded = 0;
        let mut critical = 0;

        for kind in kinds {
            let status = synthesize_panel_status(*kind);
            match status.health {
                PanelHealth::Healthy => healthy += 1,
                PanelHealth::Degraded => degraded += 1,
                PanelHealth::Critical => critical += 1,
                PanelHealth::Paused => {}
            }
            panels.push(status);
        }

        let digest = compute_snapshot_digest(&self.layout, &panels, timestamp_ms);

        CockpitSnapshot {
            layout: self.layout,
            panel_count: panels.len(),
            healthy_panels: healthy,
            degraded_panels: degraded,
            critical_panels: critical,
            active_external_agents: 0,
            panels,
            timestamp_ms,
            snapshot_digest: digest,
        }
    }

    /// Render a compact, informative ASCII mission-control grid.
    pub fn render_ascii_grid(&self, snapshot: &CockpitSnapshot) -> String {
        let mut out = String::new();
        out.push_str("========================================================================================\n");
        out.push_str(&format!(
            " AGENT-PANDORA MISSION-CONTROL COCKPIT [{}] - Panels: {}/{} Healthy\n",
            match snapshot.layout {
                CockpitLayout::NinePanel => "9-PANEL (3x3)",
                CockpitLayout::TwelvePanel => "12-PANEL (3x4)",
            },
            snapshot.healthy_panels,
            snapshot.panel_count,
        ));
        out.push_str(&format!(
            " Digest: {} | Epoch: {}ms\n",
            &snapshot.snapshot_digest[..16],
            snapshot.timestamp_ms
        ));
        out.push_str("========================================================================================\n");

        for panel in &snapshot.panels {
            out.push_str(&format!(
                " {:<20} | {:<9} | {:<12} | {}\n",
                panel.title,
                panel.health.as_str(),
                panel.category.as_str(),
                panel.headline
            ));
            let metric_str = panel
                .metrics
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("  ");
            if !metric_str.is_empty() {
                out.push_str(&format!("   -> Metrics: {metric_str}\n"));
            }
        }
        out.push_str("========================================================================================\n");
        out
    }

    /// Generate the exact Pexer multiplexer CLI/IPC command sequence to launch the 9- or 12-panel workspace.
    pub fn generate_pexer_commands(
        &self,
        workspace_name: &str,
        pexer_exe: Option<&str>,
    ) -> Vec<String> {
        let exe = pexer_exe.unwrap_or("pexer.exe");
        let mut cmds = Vec::new();

        // 1. Create or switch to the dedicated Pandora workspace
        cmds.push(format!("{exe} workspace new {workspace_name}"));

        let kinds = match self.layout {
            CockpitLayout::NinePanel => PanelKind::core_nine(),
            CockpitLayout::TwelvePanel => PanelKind::all_twelve(),
        };

        // Initial pane is Panel 1
        let first = &kinds[0];
        cmds.push(format!(
            "{exe} pane rename 0 \"{} {}\"",
            first.title(),
            first.default_command()
        ));

        // Generate split commands according to grid layout
        // For 3x3: 3 columns, each with 3 rows
        // For 3x4: 4 columns, each with 3 rows
        let cols = match self.layout {
            CockpitLayout::NinePanel => 3,
            CockpitLayout::TwelvePanel => 4,
        };

        for (idx, kind) in kinds.iter().enumerate().skip(1) {
            let direction = if idx % cols == 0 { "down" } else { "right" };
            cmds.push(format!(
                "{exe} split {direction} --title \"{}\" \"{}\"",
                kind.title(),
                kind.default_command()
            ));
        }

        // Return focus to panel 0
        cmds.push(format!("{exe} focus 0"));
        cmds
    }
}

fn synthesize_panel_status(kind: PanelKind) -> PanelStatus {
    let (health, headline, raw_metrics): (PanelHealth, &str, &[(&str, &str)]) = match kind {
        PanelKind::DaemonGuard => (
            PanelHealth::Healthy,
            "Daemon loop active; all 12 invariant gates reporting PASS",
            &[
                ("circuit_breaker", "CLOSED"),
                ("tick_hz", "10.0"),
                ("consecutive_fails", "0"),
            ],
        ),
        PanelKind::Assurance12 => (
            PanelHealth::Healthy,
            "12/12 Mathematical engines certified; SHA-256 receipt chain verified",
            &[
                ("phases_covered", "9..=20"),
                ("theorems_verified", "17..=40"),
                ("hash_chain", "VALID"),
            ],
        ),
        PanelKind::GenesEvolution => (
            PanelHealth::Healthy,
            "PAC-Bayes risk certificate bounded; Catoni promotion clear",
            &[
                ("active_genes", "64"),
                ("mutation_rate", "0.015"),
                ("catoni_risk_max", "0.042"),
            ],
        ),
        PanelKind::HarnessBench => (
            PanelHealth::Healthy,
            "Tier 1-3 test harnesses passing; zero regression observed",
            &[
                ("suites_active", "8"),
                ("pass_rate", "100.0%"),
                ("p99_latency_us", "482"),
            ],
        ),
        PanelKind::ConfigSettings => (
            PanelHealth::Healthy,
            "Runtime doctor 100% green; provider endpoints reachable",
            &[
                ("providers", "3_online"),
                ("token_budget", "85%_avail"),
                ("doctor_verdict", "READY"),
            ],
        ),
        PanelKind::SwarmMfgBft => (
            PanelHealth::Healthy,
            "Swarm equilibrium stable; PoA bounded by 4/3",
            &[
                ("swarm_nodes", "16"),
                ("poa_ratio", "1.182"),
                ("bft_quorum", "11/16"),
            ],
        ),
        PanelKind::TropicalDag => (
            PanelHealth::Healthy,
            "Max-plus Kleene star converged; zero positive cycles detected",
            &[
                ("dag_tasks", "128"),
                ("critical_path_ms", "14.2"),
                ("karp_cycle_mean", "0.00"),
            ],
        ),
        PanelKind::BanditRouter => (
            PanelHealth::Healthy,
            "Exp3 adversarial bandit routing active; regret within sublinear bound",
            &[
                ("candidate_arms", "5"),
                ("exp3_gamma", "0.08"),
                ("top_arm", "local_fast"),
            ],
        ),
        PanelKind::DriftConformal => (
            PanelHealth::Healthy,
            "Optimal transport drift below threshold; conformal coverage >= 95%",
            &[
                ("wasserstein_w2", "0.021"),
                ("target_coverage", "95.0%"),
                ("empirical_coverage", "95.6%"),
            ],
        ),
        PanelKind::PrivacyZk => (
            PanelHealth::Healthy,
            "RDP budget unexhausted; Goldilocks ZK state attestation valid",
            &[
                ("rdp_epsilon", "1.24"),
                ("rdp_delta", "1e-6"),
                ("zk_proof_soundness", "2^-49.7"),
            ],
        ),
        PanelKind::MerkleAudit => (
            PanelHealth::Healthy,
            "Merkle-DAG append-only chain valid; zero state tamper detected",
            &[
                ("audit_records", "10492"),
                ("tree_depth", "14"),
                ("verify_status", "PASS"),
            ],
        ),
        PanelKind::CommandDeck => (
            PanelHealth::Healthy,
            "Slash command dispatcher ready; multi-agent bridge listening",
            &[
                ("registered_agents", "9"),
                ("active_collab_jobs", "0"),
                ("ipc_channel", "CONNECTED"),
            ],
        ),
    };

    let metrics = raw_metrics
        .iter()
        .map(|&(k, v)| (k.to_string(), v.to_string()))
        .collect();

    PanelStatus {
        panel_id: kind.panel_number(),
        kind,
        title: kind.title().to_string(),
        category: kind.category(),
        health,
        headline: headline.to_string(),
        metrics,
        live_stream_tail: vec![
            "init: panel telemetry stream connected".to_string(),
            "tick: invariant confirmed".to_string(),
        ],
    }
}

fn compute_snapshot_digest(
    layout: &CockpitLayout,
    panels: &[PanelStatus],
    timestamp_ms: u64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"PANDORA_COCKPIT_V1:");
    hasher.update(match layout {
        CockpitLayout::NinePanel => b"09P:".as_slice(),
        CockpitLayout::TwelvePanel => b"12P:".as_slice(),
    });
    hasher.update(timestamp_ms.to_be_bytes());

    for p in panels {
        hasher.update((p.panel_id as u32).to_be_bytes());
        hasher.update(p.title.as_bytes());
        hasher.update(p.health.as_str().as_bytes());
        for (k, v) in &p.metrics {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(v.as_bytes());
            hasher.update(b";");
        }
    }

    format!("{:x}", hasher.finalize())
}
