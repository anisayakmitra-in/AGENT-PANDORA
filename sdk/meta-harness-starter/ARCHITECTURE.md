# Authority boundary

This package is metadata-only. `meta_composition.allowed_domains` is a closed
set and `max_handoffs` is a hard ceiling. An orchestration plan is rejected
before effect execution if it names any other Domain Harness or exceeds that
ceiling.

The Meta Harness owns no Genes, executes no artifact code, calls no tools,
adds no capabilities, approves nothing, and issues no permits. It cannot
replace `core-source`, Parliament, Shadow Council, ReferenceMonitor, package
trust, or the activation rules.

Admission verifies the deterministic package identity, exact artifact hash,
runtime compatibility, exact dependencies, composition members, and trust
evidence. Enabling changes only an exact lifecycle binding. Disable and
rollback preserve the compiled fallback and do not grant effect authority.

Parliament remains the policy authority and ReferenceMonitor remains the sole
issuer of scoped, one-shot permits.
