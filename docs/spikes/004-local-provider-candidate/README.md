# Spike 004 — Local provider candidate

Date: 2026-09-20
Status: **NO-GO — source-build candidate retired before any download**

## Final verdict

This candidate is rejected. The local macOS source build requires child processes, while process groups and inherited sentinel descriptors cannot contain a descendant that calls `setsid()` and closes inherited descriptors. Independent review reproduced a surviving descendant. Trusting pinned upstream source enough to waive that boundary would weaken the stated ultra-security model.

The downloader, build runner, evidence publisher, and their Python tests were deleted rather than retained as a misleading reproducibility path. The frozen candidate and review manifests remain only as historical identity records. No runtime source, model, or GGUF bytes were downloaded.

The next candidate must use an immutable prebuilt arm64 runtime with a published digest and no local source-build process tree, or a real OS job/VM boundary that proves descendant ownership.

## Decision

The model choice remains viable for a future runtime candidate, but this source-build acquisition path must not advance to transfer.

The prior Qwen3-4B candidate remains `NO-GO_FOR_INTEGRATION` at 17/20 behavior cases. Qwen3.5-4B is materially stronger in the official base-model benchmarks and remains within the 4 GiB artifact ceiling. Its selected GGUF is a pinned third-party Unsloth conversion, so exact GGUF metadata, quant provenance, behavior, and runtime compatibility must be proven rather than inferred from the base card.

## Frozen candidate

The machine-readable authority is [`candidate-manifest.json`](candidate-manifest.json), currently SHA-256 `8b178c15609ff3db702920de5cad218955cfe4cfaae4983de19acdc64e73960a`. Key values:

- base: `Qwen/Qwen3.5-4B` at `851bf6e806efd8d0a36b00ddf55e13ccb7b8cd0a`;
- quant: `unsloth/Qwen3.5-4B-GGUF` at `e87f176479d0855a907a41277aca2f8ee7a09523`;
- file: `Qwen3.5-4B-Q4_K_M.gguf`;
- size: `2,740,937,888` bytes (2.55 GiB);
- LFS SHA-256: `00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4`;
- runtime: `llama.cpp` v0.4.1 commit `b29c606e28a01b1bc8c1351026a0fa6e616bf6c4`;
- runtime tree: `950999fe62b7fe55f44ab5b7394e3c8542f37f12`;
- model license: Apache-2.0; runtime license: MIT.

## Why this candidate

Official Qwen metadata reports a 4B post-trained Qwen3.5 model with 262,144-token native context and stronger published results than the prior Qwen3-4B family. The experiment deliberately caps context at 4,096 tokens. `llama.cpp` v0.4.1 explicitly includes Qwen3.5 recurrent-layer metadata and GDN normalization fixes and is source-pinned because the stable GitHub release is mutable and has no stable macOS binary asset.

Alternatives were not selected:

- Qwen3-4B-Instruct-2507 is an improvement over the tested base but does not provide as large a claimed capability step as Qwen3.5.
- Google Gemma 3 4B offers an official QAT GGUF but is gated and uses Gemma-specific terms.
- Phi-4-mini remains factual-knowledge limited per Microsoft and lacks an official Microsoft GGUF found in this review.
- SmolLM3-3B is smaller and Apache-2.0 but has lower expected assistant quality and only eight listed languages.

## Preflight

Observed on this Mac immediately before freezing:

- architecture: arm64;
- unified memory: 16 GiB;
- free disk on `/private/tmp` at `2026-09-20T01:51:05+0400`: `26,417,360,896` bytes;
- model size: 2.55 GiB;
- exact pre-transfer minimum including source archive, expanded source, build output, model, and 10 GiB safety margin: `17,773,323,424` bytes.

The preflight performs no network request. The pre-approval candidate cannot enter run mode. After explicit owner approval, a superseding candidate binds the SHA-256 of a one-time random stdin nonce, replays its independently reviewed aggregate, rechecks the exact minimum on `/private/tmp` immediately before each transfer, downloads source before model, enforces exact HTTPS origins and redirect hosts, caps bytes and extraction, rejects links/special files, reproduces the pinned Git tree, builds with network and out-of-root writes denied, and deletes the entire private workspace through retained directory handles on success or failure. A changed redirect host, size, digest, revision, license, runtime tree, corpus, scorer, evidence schema, or reviewed tool rejects the run.

## Corrected pre-transfer contract

The first independent reviews rejected the metadata-only manifest. The current candidate fixes those findings by binding the non-thinking template, deterministic sampling and rendered-prompt identity, a 20-case corpus, scorer and inference-argv digests, exact license provenance, full acquisition/build limits, source-tree replay, verdict recomputation, native-main-thread load instrumentation, cleanup, and an external review aggregate. Adjacent sidecars are strict frozen records; execution additionally trusts only the independently reviewed aggregate named by the owner.

## Required evidence before integration

1. Independent review of this frozen manifest and the downloader/harness before transfer.
2. Explicit owner approval for the 2.55 GiB model plus bounded runtime-source transfer.
3. Exact model hash and GGUF metadata validation.
4. Runtime build identity, architecture, dependencies, and source/license proof.
5. 20/20 fixed v2 behavior cases. This corpus is newly frozen and must not be presented as byte-identical to the deleted v1 harness behind the prior 17/20 result.
6. Cold-ready, first-token, throughput, peak/steady RSS, UI-stall, cancellation, and ten-response stability gates.
7. OS-enforced network denial with a positive control, no listener, no surviving process, no raw prompts/completions/model paths in evidence.
8. Independent evidence replay and review with no unresolved P0-P2.
9. Separate approval before retaining model/runtime bytes or switching the product from the scripted provider.

## Source scope and omissions

Reviewed primary metadata: official Qwen base card/API/license, pinned Unsloth repository API/tree, pinned llama.cpp release/commit/license and tree metadata, official Microsoft and Google model cards, and ggml-org SmolLM3 metadata. Local pre-transfer code and synthetic tests cover corpus scoring, GGUF metadata parsing, Git-tree replay, redirect/size/digest enforcement, archive extraction, lifecycle bounds, evidence replay, sandbox controls, and the dormant Rust provider state machine. No runtime source archive, model, GGUF tensor payload, local inference result, benchmark result, or conversion recipe has been acquired or inspected. Published benchmark claims remain upstream claims, not J.A.R.V.I.S measurements.

## Sources

- <https://huggingface.co/Qwen/Qwen3.5-4B>
- <https://huggingface.co/api/models/Qwen/Qwen3.5-4B>
- <https://huggingface.co/api/models/unsloth/Qwen3.5-4B-GGUF>
- <https://huggingface.co/api/models/unsloth/Qwen3.5-4B-GGUF/tree/main?recursive=true>
- <https://api.github.com/repos/ggml-org/llama.cpp/releases/388619384>
- <https://api.github.com/repos/ggml-org/llama.cpp/commits/b29c606e28a01b1bc8c1351026a0fa6e616bf6c4>
- <https://huggingface.co/google/gemma-3-4b-it-qat-q4_0-gguf>
- <https://huggingface.co/microsoft/Phi-4-mini-instruct>
- <https://huggingface.co/api/models/ggml-org/SmolLM3-3B-GGUF>
