# Authoring Custom Harnesses and Genes

Pandora is designed as an extensible, governed agent operating system. Developers can create, package, admit, and enable custom **Domain Harnesses**, **Meta Harnesses**, and **Genes** without modifying the core constitutional runtime.

This guide provides a step-by-step walkthrough for building, packaging, admitting, and running custom extensions.

---

## 1. Core Concepts: The Harness and Gene Hierarchy

Pandora enforces a strict separation of concerns across four extension types:

| Kind | Role | Owns Genes? | Runnable Directly? | Primary Responsibility |
| :--- | :--- | :---: | :---: | :--- |
| **Source Harness** | System Augmentation | No | No | Binds immutable constitutional runtime services (`core-source`). Core-owned only. |
| **Domain Harness** | Domain Execution | Yes | Yes | Specializes in a technical domain (e.g., frontend, rust-dev, vlsi, data-analysis). Owns 1-32 Genes. Declares route hints for Shadow Council. |
| **Meta Harness** | Composition Only | No | No | Coordinates multiple Domain Harnesses. Enforces a maximum handoff limit and a declared set of allowed domains. |
| **Gene** | Capability Unit | N/A | No | An atomic, bounded execution unit. Implemented either as a compiled WebAssembly (`.wasm`) module or a declarative plan generator. |

```text
User Intent
    │
    ▼
Shadow Council (Route Hints Matching)
    │
    ▼
Domain Harness (e.g., my-org/rust-dev)
    │
    ├── Gene 1: workspace.search (Wasm)
    └── Gene 2: cargo.check (Wasm)
```

---

## 2. Quickstart: Using Starter Packages

Pandora provides pre-built, verified reference starter templates in the `sdk/` directory:

- [Domain Harness Starter](../sdk/domain-harness-starter/README.md): Reference Domain Harness package.
- [Meta Harness Starter](../sdk/meta-harness-starter/README.md): Reference Meta Harness composition.
- [Gene Pack](../sdk/gene-pack/README.md): Reference WebAssembly Gene package with `.wasm` binaries.

You can copy these directories directly or use the built-in scaffolding CLI.

---

## 3. Creating a Custom Domain Harness

### Step 1: Scaffold the Package

Use the `pandora package scaffold` command to generate a clean, validated directory structure:

```bash
pandora package scaffold domain-harness --output ./my-domain-harness
cd ./my-domain-harness
```

This generates:
- `pandora.package.json`: The strict package manifest.
- `domain-harness.artifact`: The deterministic metadata artifact.
- `README.md`: Local development notes.
- `ARCHITECTURE.md`: Authority and security boundaries.

### Step 2: Configure `pandora.package.json`

Open `pandora.package.json` and customize your package metadata:

```json
{
  "id": "my-org/rust-specialist",
  "version": "1.0.0",
  "kind": "domain_harness",
  "publisher": "my-org",
  "content_hash": "sha256:<calculated-sha256-hash>",
  "dependencies": [
    {
      "id": "workspace.read",
      "version": "0.1.0",
      "optional": false
    }
  ],
  "compatibility": {
    "runtime": "pandora>=2.0.0-alpha.0, <3.0.0"
  },
  "license": "Apache-2.0",
  "trust": {
    "level": "unverified",
    "signature": null,
    "public_key": null
  },
  "domain_routing": {
    "hints": [
      "cargo",
      "clippy",
      "rust",
      "tokio"
    ]
  }
}
```

#### Key Manifest Fields:
- **`id`**: Namespaced identifier (`<vendor>/<name>`).
- **`version`**: Strict SemVer (`1.0.0`, not `1.0`).
- **`kind`**: Must be `domain_harness`.
- **`publisher`**: Organization or author name string.
- **`content_hash`**: The `sha256:<digest>` checksum of the package artifact.
- **`domain_routing.hints`**: 1 to 32 canonical keywords, sorted in ascending alphabetical order. When a user runs a task containing these keywords, **Shadow Council** scores the longest match to auto-route to your Harness.
- **`dependencies`**: The list of Gene or Domain packages this Harness requires.

### Step 3: Compute Artifact Hash

Generate the SHA-256 checksum of your artifact file:

**Linux / macOS:**
```bash
sha256sum domain-harness.artifact
```

**Windows (PowerShell):**
```powershell
(Get-FileHash -Algorithm SHA256 .\domain-harness.artifact).Hash.ToLower()
```

Update the `"content_hash"` field in `pandora.package.json` with `sha256:<digest>`.

---

## 4. Creating a Custom Gene (WebAssembly)

Genes execute bounded actions. A WebAssembly Gene runs inside a memory-capped, fuel-metered Wasm sandbox (`pandora-runtime::WasmExecutor`).

### Step 1: Gene Manifest

A Gene package declares its capabilities and target metadata:

```json
{
  "id": "my-org/cargo-check-gene",
  "version": "1.0.0",
  "kind": "gene",
  "publisher": "my-org",
  "content_hash": "sha256:<calculated-sha256-hash>",
  "dependencies": [],
  "compatibility": {
    "runtime": "pandora>=2.0.0-alpha.0, <3.0.0"
  },
  "license": "Apache-2.0",
  "trust": {
    "level": "unverified",
    "signature": null,
    "public_key": null
  },
  "gene_contract": {
    "execution": "effect_request",
    "capabilities": [
      "filesystem.read",
      "process.execute"
    ],
    "approval_required": true
  }
}
```

### Step 2: WebAssembly Constraints
- Compiled to `wasm32-wasip1` or `wasm32-unknown-unknown`.
- Maximum memory allocation: 16 MB (`MAX_WASM_MEMORY_BYTES`).
- Bounded execution fuel: Stops infinite loops deterministically.
- All effects generate `OperationRequest` records submitted to **Parliament**; the Wasm sandbox itself cannot bypass the **ReferenceMonitor**.

---

## 5. Creating a Custom Meta Harness

A Meta Harness coordinates multi-domain plans without executing tools directly.

### Step 1: Scaffold the Meta Harness

```bash
pandora package scaffold meta-harness --output ./my-meta-harness
cd ./my-meta-harness
```

### Step 2: Configure Composition Rules

In `pandora.package.json`:

```json
{
  "id": "my-org/fullstack-meta",
  "version": "1.0.0",
  "kind": "meta_harness",
  "publisher": "my-org",
  "content_hash": "sha256:<calculated-sha256-hash>",
  "dependencies": [
    {
      "id": "coding-domain",
      "version": "0.1.0",
      "optional": false
    },
    {
      "id": "research-domain",
      "version": "0.1.0",
      "optional": false
    }
  ],
  "compatibility": {
    "runtime": "pandora>=2.0.0-alpha.0, <3.0.0"
  },
  "license": "Apache-2.0",
  "trust": {
    "level": "unverified",
    "signature": null,
    "public_key": null
  },
  "meta_composition": {
    "allowed_domains": [
      "coding-domain",
      "research-domain"
    ],
    "max_handoffs": 4
  }
}
```

- **`max_handoffs`**: Prevents runaway loops across subagents (e.g., maximum 4 transitions).
- **`allowed_domains`**: Strict allowlist of domains this coordinator can delegate to.


---

## 6. Admission, Verification, and Enablement

Pandora maintains an inert local package registry. A downloaded or created package cannot execute until it passes explicit admission and enablement.

### Step 1: Admit the Package

Admitting verifies the package signature, SemVer compatibility, dependency resolution, and hashes into `packages.sqlite3`:

```bash
pandora package admit \
  --manifest ./my-domain-harness/pandora.package.json \
  --artifact ./my-domain-harness/domain-harness.artifact
```

### Step 2: Inspect Package Trust

Verify that the package was admitted cleanly:

```bash
pandora package list
pandora package inspect my-org/rust-specialist 1.0.0
```

### Step 3: Enable the Package

Admitted packages remain inert until explicitly enabled:

```bash
# Preview changes with dry-run
pandora package enable my-org/rust-specialist 1.0.0 --dry-run

# Enable the harness
pandora package enable my-org/rust-specialist 1.0.0 --yes
```

### Step 4: Verify Runtime Routing

Test that the Shadow Council now routes tasks matching your declared route hints:

```bash
# Inspect the active harness catalog
pandora harness list
pandora harness inspect my-org/rust-specialist

# Execute a task using your new harness
pandora run "Check the tokio cargo dependencies in this repository"
```

---

## 7. Package Lifecycle & Rollback Commands

Pandora provides full deterministic lifecycle management for packages:

| Action | Command | Purpose |
| :--- | :--- | :--- |
| **Disable** | `pandora package disable <id> <version> --yes` | Immediately disables a package without deleting its verified artifact. |
| **Rollback** | `pandora package rollback <id> --yes` | Rolls back to the previously active verified version. |
| **Lock** | `pandora package lock` | Generates a deterministic lockfile of active package bindings. |
| **Verify Lock** | `pandora package verify-lock` | Audits that current runtime state exactly matches the lockfile. |
| **Remove** | `pandora package remove <id> <version> --yes` | Safely purges an unreferenced package artifact. |

For full specification details on package storage and validation triggers, see [Harness Packages](HARNESSES.md) and [WebAssembly Genes](WASM.md).
