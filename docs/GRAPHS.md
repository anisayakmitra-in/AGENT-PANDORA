# Graph intelligence

`pandora-runtime::GraphIntelligenceEngine` builds deterministic, bounded graph
evidence from source material that a caller has already read through Pandora's
governed effect boundary. The engine does not open files, follow links, make
network requests, execute code, or grant permissions.

Four projections are available:

- `code` records files, import/module references, and source provenance.
- `knowledge` records document headings and explicit Markdown links.
- `review` records bounded maintenance markers and findings.
- `architecture` groups files by their top-level path layer and records
  dependency evidence.

Every snapshot is scoped to a tenant and workspace and includes a deterministic
SHA-256 digest. Inputs are relative paths only, have bounded size and count,
and require a provenance label. Nodes and edges are sorted before the digest is
calculated, so equivalent input order produces the same evidence.

Graph output is descriptive evidence. It cannot authorize an effect, override
Parliament, issue a permit, promote memory, or activate a package. Callers must
rebuild a snapshot when source content or provenance changes; the digest is not
a claim that the graph is complete or that a review finding is a vulnerability.
