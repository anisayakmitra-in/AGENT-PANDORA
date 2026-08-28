# Prompt caching

Pandora has two separate cache layers. Neither layer grants authority or creates
an execution path.

## Context assembly cache

`ContextEngine` caches an exact assembled context only when every included
fragment is public or internal, has complete provenance, and is still valid for
the exact tenant, workspace, session, provider, model, policy, projection, token
budget, and classification boundary. Sensitive, secret, expired, or incomplete
context bypasses the cache.

The context receipt reports `hit`, `miss`, or `bypass` and binds the
manifest digest used to produce the assembly.

## Provider prefix cache

Agent runs may request provider-side caching for the stable system prefix only
when the context receipt is cacheable and provenance-complete. The cache
directive is part of the canonical provider authorization payload, so changing
it after Parliament and the reference monitor authorize a call invalidates the
permit.

For Anthropic Messages, Pandora places a five-minute ephemeral cache breakpoint
on the system block. Dynamic user tasks, assistant turns, tool calls, and tool
results remain outside that explicit breakpoint. OpenAI-compatible providers
retain their provider-managed automatic prefix caching behavior; Pandora does
not send non-standard cache fields to generic compatible endpoints. Gemini
usage metadata is normalized when the provider reports cached-content tokens.

Prompt-cache eligibility is deliberately denied when the system context contains
sensitive memory, enabled Skill material classified as sensitive, secrets, or
incomplete provenance.

## Evidence

Normalized usage reports:

- total prompt and completion tokens;
- prompt tokens read from a provider cache;
- prompt tokens written into a provider cache.

Provider-call metrics expose the same cache read/write counters. These values
feed observability and efficiency evidence only. They cannot affect Parliament,
issue a permit, select a Gene, activate an evolution candidate, or bypass an
approval.
