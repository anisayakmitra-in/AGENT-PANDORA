# Gene pack authority boundary

Each example artifact is an import-free WASM module with bounded input, output, memory, and fuel. Its package manifest may declare downstream effects, but that declaration only narrows what the Gene may propose. It never expands policy or runtime authority.

`static_guidance` declares no downstream capability and therefore cannot propose an effect. `bounded_read` declares only `filesystem.read`; traversal, absolute paths, symlink components, oversized reads, and mismatched permits fail in the existing filesystem boundary. `effect_request` declares `filesystem.write` and requires approval evidence. The example returns a typed JSON proposal and cannot call the filesystem executor itself.

The governed effect sequence remains:

1. Validate the Gene identity and capability against its signed contract.
2. Ask Parliament to decide under the active policy version.
3. Record and consume exact approval evidence when Parliament requires it.
4. Ask ReferenceMonitor for a scoped, expiring, one-shot permit.
5. Consume the permit in the existing executor and emit a receipt.

No example contains a native executable, import, network client, credential lookup, package installer, policy mutation, activation call, permit issuer, or evolution-lineage mutation.
