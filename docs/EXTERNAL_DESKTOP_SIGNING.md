# External desktop-signing boundary

Pandora does not treat a GitHub environment attached to the tag-controlled
release workflow as a sufficient desktop-signing boundary. A tag can change the
workflow, build hooks, package scripts, and helper code that run beside a
secret. Release-candidate and stable publication therefore remain fail-closed
until a separately administered signing service is selected and verified.

## Request contract

`scripts/external_signing_request.py` defines the provider-neutral request that a
secretless build job may hand to an external signer:

- exact release tag and source commit;
- exact target triple and platform;
- required vendor checks (`authenticode` for Windows; `codesign` and
  `notarization` for macOS);
- one or more final package names, byte counts, and SHA-256 digests;
- `source_execution: forbidden` and `rebuild: forbidden` policy fields; and
- a deterministic `request_id` derived from the canonical request body.

The request contains no credentials, source tree, provider URL, shell command,
or free-form instruction. A provider must reject requests that do not match its
own independently administered tag, repository, workflow, target, artifact
count, and digest policy. The request ID is an idempotency/binding key, not a
proof that a provider accepted or signed anything.

## Required provider response

A provider adapter must return an authenticated envelope bound to the exact
`request_id`, source commit, target, platform, and signed-artifact digests. The
envelope must include independently verifiable evidence for every required
check, including signer identity, timestamp/TSA information, final package
identity, and notarization result where applicable. The repository-side
verifier must validate the provider's trust root or attestation; a boolean
`verified` field supplied by the requestor is not evidence.

The provider must operate on the prebuilt package bytes. It must not check out
or execute the tagged source, run npm/Cargo/Tauri hooks, or receive signing
credentials in a general build environment. A separate clean verification
runner must re-download the exact signed artifacts, validate their digests and
vendor evidence, and only then may the publication gate proceed.

## Release state

The checked-in `release-scope.json` remains `cli-only`; no desktop artifact is
published by the current beta line. The existing secretless release gate must
remain blocking for RC and stable until the external provider, trust root,
adapter, and real platform evidence have been independently reviewed. No
provider credentials, signing operation, tag, or publication is part of this
contract-only change.
