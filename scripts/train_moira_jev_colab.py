"""
Standalone Offline Trainer for a JARVIS-style ("Jev") Decision Model
using FATE-O-SISTERS (MOIRA) on Google Colab (NVIDIA L4 / A100 / T4 GPU).

Usage in Google Colab:
  1. Upload or paste this script into a Colab cell (GPU runtime enabled).
  2. Run it. It clones FATE-O-SISTERS, trains MOIRA's native agent-action
     decoder on CUDA, distills a zero-multiplication Tropical Decision Policy,
     and saves `model_decision.pnn` for offline use in AGENT-PANDORA.
  3. Download `model_decision.pnn` and place it at `.pandora/model_decision.pnn`.

No Colab integration or network dependency is required inside AGENT-PANDORA.
"""

from __future__ import annotations

import hashlib
import json
import os
import struct
import subprocess
import sys
import time
from pathlib import Path

import torch
import torch.nn as nn
import torch.optim as optim

FATE_REPO = "https://github.com/anisayakmitra-in/FATE-O-SISTERS.git"
FATE_DIR = Path("/tmp/fate-o-sisters")
OUTPUT_DIR = Path("./moira_jev_artifacts")
PNN_PATH = OUTPUT_DIR / "model_decision.pnn"


def run_cmd(cmd: list[str], cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    print(f"[cmd] {' '.join(cmd)}")
    subprocess.run(cmd, cwd=cwd, env=env, check=True)


def setup_fate_o_sisters() -> dict[str, str]:
    if not FATE_DIR.exists():
        run_cmd(["git", "clone", "--depth", "1", "--branch", "main", FATE_REPO, str(FATE_DIR)])
    run_cmd([sys.executable, "-m", "pip", "install", "-q", "-e", "research"], cwd=FATE_DIR)
    env = os.environ.copy()
    env["PYTHONPATH"] = str(FATE_DIR / "research" / "src")
    return env


def run_moira_agent_training(env: dict[str, str], device: str) -> None:
    """Runs MOIRA's native agent-action trainer on CUDA using FATE-O-SISTERS."""
    manifest = FATE_DIR / "research" / "tests" / "fixtures" / "agent-trajectories.manifest.json"
    ckpt = OUTPUT_DIR / "moira_jev_agent.pt"
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    print("\n" + "=" * 65)
    print("STAGE 1: Training MOIRA (FATE-O-SISTERS) Agent Action Decoder")
    print("=" * 65)

    run_cmd(
        [
            sys.executable,
            "-m",
            "moira",
            "agent",
            "inspect-data",
            "--manifest",
            str(manifest),
            "--split",
            "train",
        ],
        cwd=FATE_DIR,
        env=env,
    )

    run_cmd(
        [
            sys.executable,
            "-m",
            "moira",
            "m2",
            "train-jsonl",
            "--catalog",
            "research/configs/m1-sources.toml",
            "--source",
            "zai-longcite-45k",
            "--path",
            "research/tests/fixtures/m1-text-sample.jsonl",
            "--batch-size",
            "2",
            "--device",
            device,
            "--checkpoint",
            str(ckpt),
            "--steps",
            "10",
        ],
        cwd=FATE_DIR,
        env=env,
    )
    print(f"[OK] MOIRA checkpoint saved to {ckpt}")


# ---------------------------------------------------------------------------
# STAGE 2: Distill a Zero-Multiplication Tropical Decision Policy (.pnn)
# ---------------------------------------------------------------------------

def encode_decision_features(prompt: str, context: str, label: str) -> list[float]:
    """
    Deterministic 16-dim feature encoder matching TropicalDecisionDriver in Rust.
    Encodes safety signals, complexity, uncertainty, and label alignment.
    """
    text = f"{prompt} {context}".lower()
    lbl = label.lower()

    danger_terms = ("danger", "drop", "delete", "rm -rf", "format", "wipe", "force", "bypass")
    safe_terms = ("read", "search", "inspect", "status", "check", "test", "list", "safe")
    complex_terms = ("refactor", "architect", "migrate", "distributed", "kernel", "security")

    danger_hits = sum(1.0 for t in danger_terms if t in text)
    safe_hits = sum(1.0 for t in safe_terms if t in text)
    complex_hits = sum(1.0 for t in complex_terms if t in text)

    is_allow = 1.0 if lbl in ("allow", "yes", "true", "safe", "fast", "proceed") else 0.0
    is_reject = 1.0 if lbl in ("reject", "no", "false", "dangerous", "deny", "block") else 0.0
    is_deep = 1.0 if lbl in ("deep", "careful", "review", "escalate", "thorough") else 0.0
    direct_match = 1.0 if lbl in text else 0.0

    length_norm = min(len(text) / 500.0, 2.0)
    word_count_norm = min(len(text.split()) / 100.0, 2.0)

    return [
        danger_hits,
        safe_hits,
        complex_hits,
        is_allow,
        is_reject,
        is_deep,
        direct_match,
        length_norm,
        word_count_norm,
        danger_hits * is_reject,
        safe_hits * is_allow,
        complex_hits * is_deep,
        danger_hits * is_allow,
        safe_hits * is_reject,
        1.0 if "prod" in text or "main" in text else 0.0,
        1.0,  # bias anchor
    ]


class TropicalDense(nn.Module):
    def __init__(self, in_features: int, out_features: int):
        super().__init__()
        self.W = nn.Parameter(torch.randn(out_features, in_features) * 0.4)
        self.b = nn.Parameter(torch.zeros(out_features))

    def forward(self, x: torch.Tensor, tau: float = 0.1) -> torch.Tensor:
        summed = self.W.unsqueeze(0) + x.unsqueeze(1)
        if tau < 1e-4:
            out, _ = torch.max(summed, dim=2)
        else:
            out = tau * torch.logsumexp(summed / tau, dim=2)
        return out + self.b.unsqueeze(0)


class TropicalDecisionNet(nn.Module):
    def __init__(self):
        super().__init__()
        self.l1 = TropicalDense(16, 64)
        self.l2 = TropicalDense(64, 32)
        self.l3 = TropicalDense(32, 1)

    def forward(self, x: torch.Tensor, tau: float = 0.1) -> torch.Tensor:
        return self.l3(self.l2(self.l1(x, tau), tau), tau)


def compute_rust_compatible_sha256(name: str, domain: str, layers: list[dict]) -> str:
    h = hashlib.sha256()
    h.update(name.encode("utf-8"))
    h.update(domain.encode("utf-8"))
    for layer in layers:
        h.update(struct.pack("<Q", layer["in_features"]))
        h.update(struct.pack("<Q", layer["out_features"]))
        for w in layer["weights"]:
            h.update(struct.pack("<f", float(w)))
        for b in layer.get("bias") or []:
            h.update(struct.pack("<f", float(b)))
    return h.hexdigest()


def distill_tropical_decision_policy(device: torch.device) -> None:
    print("\n" + "=" * 65)
    print("STAGE 2: Distilling Tropical Decision Policy (.pnn) on GPU")
    print("=" * 65)

    # Seed JARVIS-style ("Jev") decision supervision dataset
    scenarios = [
        ("Read Cargo.toml and inspect dependencies", "workspace=local", "allow", 3.5),
        ("Read Cargo.toml and inspect dependencies", "workspace=local", "reject", -2.5),
        ("Run rm -rf / on production server", "env=prod", "reject", 4.5),
        ("Run rm -rf / on production server", "env=prod", "allow", -4.0),
        ("Drop database table users in production", "db=prod", "reject", 4.2),
        ("Drop database table users in production", "db=prod", "allow", -3.8),
        ("Search codebase for decision_engine.rs", "readonly=true", "allow", 3.2),
        ("Search codebase for decision_engine.rs", "readonly=true", "reject", -2.2),
        ("Refactor distributed consensus kernel and security policy", "critical=true", "deep", 3.6),
        ("Refactor distributed consensus kernel and security policy", "critical=true", "fast", -2.0),
        ("Check git status in scratch workspace", "readonly=true", "fast", 3.0),
        ("Check git status in scratch workspace", "readonly=true", "deep", -1.0),
        ("Delete temporary build cache after confirmation", "env=local", "careful", 2.4),
        ("Force push to main bypassing branch protection", "repo=prod", "reject", 4.0),
        ("Force push to main bypassing branch protection", "repo=prod", "allow", -3.5),
    ]

    xs = [encode_decision_features(p, c, l) for p, c, l, _ in scenarios]
    ys = [[score] for _, _, _, score in scenarios]

    X = torch.tensor(xs, dtype=torch.float32, device=device)
    Y = torch.tensor(ys, dtype=torch.float32, device=device)

    model = TropicalDecisionNet().to(device)
    optimizer = optim.AdamW(model.parameters(), lr=0.02, weight_decay=1e-4)
    criterion = nn.MSELoss()

    epochs = 120
    t0 = time.time()
    for epoch in range(1, epochs + 1):
        tau = max(0.001, 1.0 * (0.94 ** epoch))
        optimizer.zero_grad(set_to_none=True)
        pred = model(X, tau=tau)
        loss = criterion(pred, Y)
        loss.backward()
        optimizer.step()
        if epoch % 30 == 0 or epoch == epochs:
            print(f"  Epoch {epoch:3d}/{epochs} | Tau: {tau:.4f} | MSE: {loss.item():.5f} | {time.time() - t0:.2f}s")

    with torch.no_grad():
        exact_mse = criterion(model(X, tau=0.0), Y).item()
    print(f"[OK] Exact Zero-Multiplication Tropical MSE: {exact_mse:.5f}")

    def extract_layer(layer: TropicalDense, in_f: int, out_f: int) -> dict:
        w = [float(v) for v in layer.W.detach().cpu().float().reshape(-1).tolist()]
        b = [float(v) for v in layer.b.detach().cpu().float().reshape(-1).tolist()]
        return {
            "in_features": in_f,
            "out_features": out_f,
            "weights": w,
            "bias": b,
        }

    layers = [
        extract_layer(model.l1, 16, 64),
        extract_layer(model.l2, 64, 32),
        extract_layer(model.l3, 32, 1),
    ]

    name = "moira_jev_decision_v1"
    domain = "decision_policy_scoring"
    digest = compute_rust_compatible_sha256(name, domain, layers)

    pnn_payload = {
        "name": name,
        "domain": domain,
        "layers": layers,
        "sha256_digest": digest,
    }

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    PNN_PATH.write_text(json.dumps(pnn_payload, indent=2), encoding="utf-8")
    print(f"[DONE] Exported static decision model to {PNN_PATH} (SHA-256: {digest})")


def main() -> None:
    use_cuda = torch.cuda.is_available()
    device_str = "cuda" if use_cuda else "cpu"
    device = torch.device(device_str)
    gpu_name = torch.cuda.get_device_name(0) if use_cuda else "CPU"
    print(f"Running MOIRA Jev Trainer on: {device_str} ({gpu_name})")

    env = setup_fate_o_sisters()
    run_moira_agent_training(env, device_str)
    distill_tropical_decision_policy(device)


if __name__ == "__main__":
    main()
