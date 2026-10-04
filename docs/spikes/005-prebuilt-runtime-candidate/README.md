# Spike 005 — Prebuilt arm64 runtime candidate

Date: 2026-09-20
Status: **metadata-only candidate; no bottle, runtime, model, or GGUF downloaded**

## Decision

Replace the rejected local source-build path with the published Homebrew `llama.cpp` `0.4.1` `arm64_golden_gate` OCI bottle and the exact four runtime layers declared by its bottle `Tab`. Keep the Qwen3.5-4B Q4_K_M model candidate unchanged pending quality certification.

This avoids the unsolved macOS descendant-escape problem: candidate preparation performs no local compilation. Inference remains a single no-fork process under kernel network denial. It is a disposable spike candidate, not yet a production bundling choice.

## Frozen metadata

[`candidate-metadata.json`](candidate-metadata.json) binds:

- exact OCI index, target-manifest, config, layer, transfer-size, installed-size, version, target and license identities for `llama.cpp`, `ggml`, `libomp`, and `openssl@3`, plus the target-independent `ca-certificates` layer;
- the runtime bottle's embedded Homebrew dependency closure and upstream llama.cpp commit;
- the pinned Qwen3.5-4B Q4_K_M file, byte count, LFS digest, template digest, and Apache-2.0 provenance;
- a `67,108,864` byte transfer ceiling, `536,870,912` byte extraction ceiling, exact private-volume preflight, and descriptor-relative archive policy;
- at most one GHCR-to-`pkg-containers.githubusercontent.com:443` redirect, with authorization stripped before crossing origins and any second redirect rejected;
- separate purpose-bound bottle and model capability records, with model transfer structurally blocked until the relocated runtime receives a new freeze, review, and owner approval;
- a hard prohibition on local runtime source builds;
- mandatory Mach-O architecture and dynamic-dependency verification before any model transfer;
- mandatory kernel network/fork denial during inference and complete post-spike deletion.

## Required proof before approval

1. Independently verify all formula/bottle metadata against Homebrew and GHCR without downloading blobs.
2. Implement a no-child Python OCI downloader that keeps anonymous bearer tokens in memory only, revalidates hosts, bounds bytes, and verifies each digest before extraction.
3. Safely extract only regular files/directories and internally resolving bottle links into a private descriptor-owned root.
4. Validate all executable/library Mach-O slices, dynamic dependencies, RPATHs, licenses, and relocation behavior before allowing the model transfer.
5. Reuse the fixed behavior corpus and lifecycle gates, but publish evidence only through strict cross-document semantic replay.
6. Freeze the exact reviewed stage-1 downloader/extractor and obtain explicit owner approval before any bottle transfer.
7. After relocation inspection, freeze the exact observed CLI/library roster, harness, corpus, verifier, evidence schema and one-time capability; require two fresh reviews and a second owner approval before model transfer.

## Current omissions

- Bottle content and dynamic linkage have not been inspected because transfer is not authorized.
- Relocation outside `/opt/homebrew` is not proven.
- No runtime executable has run.
- No model quality, latency, throughput, memory, native-UI load, cancellation, or stability result exists.
- No product provider is activated; the scripted provider remains the rollback and only active provider.

Any failed runtime relocation, dependency closure, signature, license, sandbox, or lifecycle check is an immediate `NO-GO`, followed by deletion of every downloaded byte.
