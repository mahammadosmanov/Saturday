# Prebuilt runtime stage-1 design

Date: 2026-09-20
Status: second remediation implemented; fresh independent review required before owner transfer approval
Authority: candidate-005 aggregate `c034041a5ea9b5203e5e8385b010079764ff3daf13be00d5163e934cc11d447c`.
Rejected harness: aggregate `f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0`.
Rejected replacement: aggregate `380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0`.

## Scope

Acquire only five exact Homebrew OCI bottle layers. Do not download a model, run Homebrew, execute formula/post-install code, activate product code, or persist runtime bytes after a failed run.

## Boundary

The stdlib-only Python harness performs metadata replay, live same-volume disk checks, GHCR bearer-token exchange, bounded HTTPS GET, one reviewed cross-origin redirect with Authorization stripped, SHA-256 verification, safe extraction, relocation inspection, and cleanup. It launches no child process during acquisition or extraction.

The launch path must recompute a closed 12-file harness manifest bound into the owner authorization, verify its sidecar and executing source identity, and reject revoked aggregates before consuming authorization. The capability binds the app-owned ledger and workspace-parent device/inode/UID identities. The ledger is locked across processes and records the workspace before network access, so a crash or fresh process cannot replay the capability and can delete registered residue. Unregistered lookalike directories are never deleted. The owner nonce is read from an inherited descriptor, never an argument.

One lifecycle owns all five exact layer transfers. It performs a fresh same-volume preflight immediately before each blob body, writes each layer descriptor-relatively, reopens the exact artifacts without following links, extracts them into one transaction, inspects the resulting runtime, completes the exact transfer set, and deletes artifacts/runtime/evidence before any receipt is publishable. No caller-supplied mapping can enter the receipt publication API.

Extraction rejects absolute/traversal paths, duplicate/case/Unicode collisions, devices/FIFOs/sockets, PAX/sparse/setuid/xattrs, escaping links, hard-link cycles, oversize files, excess entries, and aggregate expansion above 512 MiB. All writes are descriptor-relative, no-follow, exclusive, private, and confined to one random root.

## Output

A successful run emits a strict redacted stage-1 receipt: candidate/run IDs; upstream and layer hashes/sizes; extracted roster hashes/modes/link targets; Mach-O slice/load/RPATH inventory; license inventory; cleanup counters; and a `MODEL_TRANSFER_BLOCKED` verdict. It retains no URLs with bearer tokens, headers, paths, stdout/stderr, environment, or model data.

Receipt replay requires the external receipt digest and the durable ledger record from the completed run. Its semantic checks require complete content-type-to-Mach-O inventory equivalence, canonical system paths, dylib-only non-system dependency closure, LC_ID_DYLIB cardinality, verified CodeDirectory page hashes, formula-local license path/size/digest evidence, per-layer installed-size or expansion limits, all five layers, and cleanup counters derived exactly from the extracted roster plus downloaded artifacts. Hard-linked content is read and hashed once per inode.

The relocated runtime must be frozen and independently reviewed. A separate model capability, nonce, aggregate, disk preflight, and owner approval are mandatory before model transfer.

## Rollback

On every ordinary failure, close descriptors, delete the private root by retained handles, verify absence, publish only a fixed failure code, and leave the scripted provider active. After abrupt termination, the next ledger-locked launch removes only the exact workspace recorded in the durable marker before attempting a new run.
