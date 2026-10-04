# Local runtime and model candidate decision

Research date: 2026-09-19. The approved disposable spike has now completed. Source and model bytes were downloaded only into temporary staging, measured, independently reviewed, and deleted. Nothing was integrated into the product. The canonical outcome is [`spikes/001-local-model-runtime/README.md`](spikes/001-local-model-runtime/README.md): **partial runtime feasibility and no-go for integration**.

## Decision

The metadata phase selected `llama.cpp` and `Qwen/Qwen3-4B-GGUF` Q4_K_M for the first disposable spike. That selection is now historical experiment scope, not an accepted production candidate.

The second candidate in [`spikes/004-local-provider-candidate/`](spikes/004-local-provider-candidate/) is now `NO-GO` before transfer. Its local source-build path could not prove ownership of a descendant that calls `setsid()` and closes inherited descriptors. The unsafe acquisition/build/evidence harness was deleted; no payload was downloaded.

The current metadata-only replacement is [`spikes/005-prebuilt-runtime-candidate/`](spikes/005-prebuilt-runtime-candidate/): digest-pinned Homebrew arm64 bottles for `llama.cpp` 8447 and its dependency closure, with no local compilation. It still requires independent provenance review, a no-child OCI acquisition harness, private relocation/linkage proof, and fresh explicit transfer approval. Any accepted runtime must communicate over private framed standard input/output, terminate with its owner, expose no listener, auto-download nothing, and never survive as a daemon.

The one-time 2.326 GiB experiment download was approved and completed. The model, source, and binaries were deleted afterward. This document does not approve another download, dependency addition, or AI activation.

## Target constraints

The measurement host is arm64 Apple M5 with 16 GiB unified memory. At research time it had 14.12 GiB free disk. The existing app bundle was 3.27 MiB. The controlling budgets remain in [`local-model-feasibility.md`](local-model-feasibility.md):

- model data at or below 4 GiB;
- steady-state process RSS at or below 6 GiB;
- peak RSS at or below 8 GiB;
- 4,096-token initial context;
- first visible token within 5 seconds and at least 8 tokens/second for the fixed benchmark;
- cancellation acknowledged within 250 ms and helper exit within 2 seconds;
- explicit install, delete, provenance, and offline gates.

The Q4 artifact alone requires 7.105 GiB under the `2 × artifact + 2 GiB` rule. Candidate 005 raises the model-stage live preflight to `8,166,230,336` bytes (7.605 GiB) to include the frozen runtime/extraction allowance. Free space must be rechecked on the exact workspace volume immediately before every approved transfer.

## Runtime comparison

### Candidate A: llama.cpp

**Disposition: advance to an isolated spike.**

Primary-source facts:

- upstream is MIT licensed;
- upstream describes Apple Silicon as a first-class target using ARM NEON, Accelerate, and Metal;
- upstream supports GGUF quantization and documents CLI/server tools;
- latest stable release observed: `v0.4.1`, release ID `388619384`, published 2026-09-14;
- tag `v0.4.1` resolved to commit `b29c606e28a01b1bc8c1351026a0fa6e616bf6c4` when checked;
- GitHub marked the release `immutable: false` and its stable release carried no macOS executable asset, so J.A.R.V.I.S must not trust a moving tag or an unpinned community binary.

Why it leads:

- its Metal backend directly targets this Mac;
- official Qwen GGUF weights avoid an unreviewed conversion step;
- it can be source-built into a narrow helper without an installed service;
- GGUF has exact artifact size and SHA-256 metadata;
- the helper boundary provides crash/process-lifecycle containment and keeps model parsing out of the webview.

Important limitation: a separate helper is not automatically a least-privilege sandbox. The spike must prove filesystem, network, process, and lifecycle restrictions rather than claiming isolation from process separation alone.

### Candidate B: MLX Swift LM

**Disposition: retain as fallback; do not spike first.**

Primary-source facts:

- upstream is MIT licensed and built for LLM/VLM inference in Swift on Apple platforms;
- it supports quantized models, local weight paths, chat sessions, guided generation, and tool-calling integration;
- latest stable release observed: `3.31.4`, release ID `346965657`, tag commit `bd4b7434e6bdb588c7ef55706ff8904cb7fd4c57`;
- GitHub marked that release `immutable: false` and published no binary assets;
- upstream examples add Swift Hugging Face downloader/tokenizer packages.

Why it is second:

- it would add a Swift helper or mixed Swift/Rust integration to a currently Rust/Tauri native shell;
- its downloader/tokenizer dependency chain is broader than the first GGUF spike requires;
- candidate weights would need a separately pinned MLX repository, file manifest, hashes, and license review;
- it is Apple-only, while `llama.cpp` preserves a future path to other desktop targets.

MLX may outperform or integrate better later. That must be measured on the same prompts, context, thermal duration, and memory gates before changing the decision.

## Model comparison

### Selected metadata candidate: Qwen3-4B Q4_K_M

Official repository: `Qwen/Qwen3-4B-GGUF`.

Frozen metadata observed from the Hugging Face API:

- repository commit: `bc640142c66e1fdd12af0bd68f40445458f3869b`;
- license: Apache-2.0;
- gated: false;
- architecture: `qwen3`;
- model file: `Qwen3-4B-Q4_K_M.gguf`;
- exact size: `2,497,280,256` bytes, or 2.326 GiB;
- published LFS SHA-256: `7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5`;
- official card describes a 4B post-trained model, native 32,768-token context, thinking/non-thinking modes, multilingual support, and agent/tool integration;
- the repository metadata includes a tool-call-aware chat template.

Why Q4_K_M rather than a larger quantization:

- Q5_K_M is 2.691 GiB and Q6_K is 3.079 GiB, but neither has earned its extra disk/memory cost on this machine;
- Q4_K_M gives the safest first chance of meeting the 6 GiB steady-state memory ceiling;
- quality, speed, repetition behavior, tool-call validity, and factual reliability remain unverified until a local benchmark runs.

The model must never directly execute tools. Any emitted tool call is untrusted text that must pass schema validation, capability checks, user approval policy, and action ownership outside the model runtime.

### Rejected for the first spike: Phi-4-mini-instruct

Microsoft's official model is MIT licensed, ungated, 3.8B parameters, multilingual, and documents instruction/function-calling uses. However, no official Microsoft GGUF repository was found in this review. Available GGUF files are third-party conversions, adding converter-version, quantization, and publisher provenance that the first spike does not need. Reconsider only if Microsoft publishes a pinned official GGUF or J.A.R.V.I.S builds and independently verifies its own conversion pipeline.

### Rejected for the first spike: Gemma 3 4B

Google publishes a 4B Q4 GGUF, but access to the base model requires acceptance of Gemma-specific terms and the model is marked with the Gemma license rather than Apache-2.0 or MIT. It adds a terms/redistribution gate without a demonstrated advantage for this first feasibility test.

### Superseded observation: newer Qwen families

The 2026-09-19 search found no official `Qwen3.5-4B-GGUF`. The 2026-09-20 successor review found the official Qwen3.5 base model and a pinned, ungated third-party Unsloth GGUF under the 4 GiB ceiling. Because it is not an official Qwen conversion, provenance and quantization remain explicit experiment risks rather than assumed facts.

## Required spike sequence

1. Re-read the official release, tag-to-commit mapping, source license, model repository commit, model file size, and LFS SHA-256.
2. Create a candidate manifest before fetching any source or weight.
3. Obtain explicit approval for the 2.326 GiB model download and expected 6.652 GiB free-space preflight.
4. Download to a temporary staging directory with redirect-host checks, byte limits, atomic rename, and SHA-256 verification.
5. Build the helper from the exact source commit with exact argv and dependency manifest; record binary architecture and SHA-256.
6. Run the fixed 20-prompt benchmark and record complete raw timing/resource evidence without retaining private prompt content.
7. Inject cancellation, malformed-frame, wrong-digest, truncated-model, helper-crash, app-close, and network-denial failures.
8. Delete the staged model/helper and verify removal unless the candidate passes and installation is separately approved.
9. Keep the scripted provider as the default rollback until all gates pass and an independent review accepts the candidate.

## Primary sources

- llama.cpp repository and runtime claims: <https://github.com/ggml-org/llama.cpp>
- llama.cpp MIT license: <https://raw.githubusercontent.com/ggml-org/llama.cpp/master/LICENSE>
- llama.cpp latest release API: <https://api.github.com/repos/ggml-org/llama.cpp/releases/latest>
- llama.cpp tags API: <https://api.github.com/repos/ggml-org/llama.cpp/tags?per_page=5>
- MLX Swift LM repository: <https://github.com/ml-explore/mlx-swift-lm>
- MLX Swift LM MIT license: <https://raw.githubusercontent.com/ml-explore/mlx-swift-lm/main/LICENSE>
- MLX Swift LM latest release API: <https://api.github.com/repos/ml-explore/mlx-swift-lm/releases/latest>
- Qwen official model card: <https://huggingface.co/Qwen/Qwen3-4B-GGUF>
- Qwen repository metadata: <https://huggingface.co/api/models/Qwen/Qwen3-4B-GGUF>
- Qwen recursive tree metadata: <https://huggingface.co/api/models/Qwen/Qwen3-4B-GGUF/tree/main?recursive=true>
- Microsoft official Phi model card: <https://huggingface.co/microsoft/Phi-4-mini-instruct>
- Google official Gemma GGUF card: <https://huggingface.co/google/gemma-3-4b-it-qat-q4_0-gguf>
