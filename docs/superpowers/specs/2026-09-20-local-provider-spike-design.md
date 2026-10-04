# Local provider candidate and evidence design

Date: 2026-09-20
Status: approved roadmap slice; pre-transfer implementation and independent review required
Profile: P0 disposable feasibility, with production-grade acquisition and evidence boundaries

## Goal

Determine whether one exact Qwen3.5-4B Q4_K_M plus llama.cpp v0.4.1 candidate can become J.A.R.V.I.S's first real local conversation provider without weakening the existing signed-helper boundary, privacy, scripted rollback, or release posture.

A successful spike proves only candidate feasibility. It does not activate AI in the product, retain model/runtime bytes, expose a Tauri command, change the UI, or authorize release.

## Frozen behavioral regime

The candidate must run one immutable behavioral regime:

- model file: `Qwen3.5-4B-Q4_K_M.gguf` at the exact revision, size, and SHA-256 in the candidate manifest;
- runtime source: llama.cpp v0.4.1 at exact commit `b29c606e28a01b1bc8c1351026a0fa6e616bf6c4` and tree `950999fe62b7fe55f44ab5b7394e3c8542f37f12`;
- selected executable: direct `llama-completion`, never a TCP server/client path;
- context: 4,096 tokens;
- maximum output: 128 tokens for behavior cases and 256 tokens for throughput/stability cases;
- deterministic behavior evaluation: seed `3407`, temperature `0`, top-k `20`, top-p `0.8`, min-p `0`, repeat penalty `1.0`, presence penalty `0`;
- thinking: disabled;
- prompt rendering: candidate-owned exact renderer, not mutable runtime defaults;
- embedded GGUF chat-template SHA-256 must equal `7f0e529032c25183bcd66c7f238da2d377f43be754a94e2725a58c4e16d2ed67` after transfer;
- upstream official-base template SHA-256 `a4aee8afcf2e0711942cf848899be66016f8d14a889ff9ede07bca099c28f715` is recorded only as contrast and must not be silently substituted.

The behavior corpus is versioned in `docs/spikes/004-local-provider-candidate/behavior-corpus.json`. This is a successor corpus whose categories preserve all twenty prior case IDs, including the three prior failures. Because the deleted predecessor prompts cannot be proven byte-identical, the new evidence must not claim a direct 17/20-to-20/20 apples-to-apples comparison. Acceptance still requires 20/20 under the newly frozen corpus and scorer.

## Candidate identity and external anchor

The repository has no initial commit. A writable adjacent checksum is not independent authority.

The final pre-transfer candidate therefore consists of a closed review roster containing:

- candidate manifest;
- behavior corpus;
- evidence schema/contract;
- this design;
- feasibility contract;
- downloader, benchmark harness, and their tests;
- exact build configuration and source-acquisition contract.

A generated review manifest records each path, byte size, mode, and SHA-256, plus one aggregate SHA-256. It and its strict sidecar are made read-only and replayed. Independent reviewers report the exact aggregate. The pre-approval manifest contains `authorization.state = not_granted`, so run mode is impossible. After explicit owner approval naming that aggregate, the controller generates a one-time random nonce, freezes only its SHA-256 into a superseding manifest, rebuilds/reviews the aggregate, and supplies the nonce through stdin exactly once. Any roster change invalidates approval and requires a new review/aggregate.

## Acquisition boundary

No transfer occurs before explicit owner approval.

The acquisition command owns the entire sequence:

1. Parse strict JSON with duplicate-key rejection and verify the exact reviewed aggregate supplied on the command line.
2. Create one unpredictable, non-existing private root beneath `/private/tmp`, mode `0700`, and retain its directory handle.
3. Recheck the actual target volume immediately before every transfer. Require at least the manifest minimum and the formula `2 × model bytes + source-transfer cap + extracted-source cap + build-output cap + 2 GiB`.
4. Fetch only exact HTTPS URLs from the manifest. Permit at most five redirects and revalidate scheme, port, and exact host on every hop.
5. Use exclusive no-follow temporary files, enforce byte ceilings while streaming, fsync, verify expected size/hash where published, then atomically rename within the same directory.
6. Model transfer requires exact `2,740,937,888` bytes and SHA-256 `00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4` before it becomes runnable.
7. Runtime source transfer is capped at 512 MiB. Because GitHub publishes no archive digest or content length for the stable source tarball, the archive is never accepted merely from a post-download digest. Safe extraction enforces path containment, no devices/FIFOs, bounded file count, bounded expanded bytes, one top-level directory, and no unexpected links. Source identity is then bound to the pinned signed commit/tree and a reviewed source-roster verifier. This limitation remains explicit in evidence.
8. Build executes under OS network denial with a private `HOME`/`TMPDIR`, fixed locale, exact compiler/CMake identities, exact CMake argv, no curl/server/shared libraries, arm64-only output, and bounded source/build disk growth.
9. The selected runtime binary is rechecked for regular-file/link/mode/architecture identity and its dynamic dependencies are allowlisted before use.

The transfer implementation must never accept caller-supplied expected size/hash/origin values; those come only from the reviewed manifest.

## Exact resource limits

- source transfer: at most 512 MiB;
- extracted source: at most 1.5 GiB and 20,000 entries;
- build outputs/cache: at most 2 GiB and 20,000 entries;
- model: exact 2.55 GiB and at most 4 GiB installed-model gate;
- required free space before acquisition: formula above, never a stale recorded free-space value;
- steady-state RSS: at most 6 GiB;
- peak RSS: at most 8 GiB;
- fresh-process start through first visible token: at most 10 seconds with runtime warmup disabled;
- sustained generation: at least 20 tokens/second;
- active-generation `SIGTERM` through full process-tree exit: at most 250 ms for the disposable CLI spike;
- native main-thread stall: no event-loop stall over 100 ms;
- stability: at least ten consecutive isolated 256-token generations spanning at least 600 seconds, no crash, zero swap growth, and final steady RSS no more than 128 MiB above the first steady sample.

## Behavior corpus and scorer

The corpus contains exactly twenty synthetic cases. Every case has a stable ID, prompt, maximum output tokens, validator kind, literal validator data, and sensitivity class `synthetic`.

Scoring is source-derived, not model-derived:

- exact text validators normalize only line endings and outer whitespace;
- token/list validators use an explicit grammar;
- JSON validators parse with duplicate-key rejection, exact keys, exact primitive values, and no trailing prose;
- bounded summaries enforce word bounds plus required/forbidden literals;
- safety cases reject execution claims, secret-shaped strings, HTML, tool-call markup, and unexpected actions.

The harness never records prompt or completion bytes. It records case ID, pass/fail, bounded timing/counter fields, output byte count, and SHA-256 only. Local console output also omits prompts/completions.

## Runtime and helper architecture

The spike initially invokes the pinned direct llama.cpp executable inside the existing OS-denied experiment sandbox. It must not expose a listener, download content, fork, or survive its owner.

The production direction, if the spike passes, is a statically linked llama.cpp backend inside the app-owned signed helper. It will extend the private framed protocol with:

- `load_model` bound to candidate/model/generation digests;
- `generate` containing bounded role/content fields and manifest-owned settings;
- `cancel` acknowledging the active request before any terminal response;
- `unload` and `shutdown` with zero surviving workers;
- monotonic chunks and one terminal result per request.

Only one model operation may be active. Generated tool-like text is data, never authority. No Tauri/webview command is added during the spike or no-model adapter stage.

Model-file identity remains a production design gate: llama.cpp currently accepts a pathname. The experiment may load a private verified staging pathname, but production integration must either prove a stable app-owned installed-object contract through the entire load/mmap lifetime or add a descriptor-backed loader. A successful behavior spike does not waive that gate.

## Evidence contract

Evidence publication accepts no caller-created counters. The lifecycle owner derives all fields and publishes only after every worker/process has terminated.

Allowed documents are fixed and size-bounded:

- `candidate.json`: reviewed aggregate, manifest digest, artifact/runtime hashes, build identity;
- `acquisition.json`: origin/redirect classes, byte counts, digest-match booleans, filesystem/device class, no paths or URLs with transient query strings;
- `behavior.json`: twenty case IDs and derived result counters/hashes;
- `performance.json`: fresh-process first-token, throughput, RSS, swap, and native-main-thread-stall integers;
- `lifecycle.json`: cancellation, late-frame, exit, process-count, listener/network-denial, and cleanup counters;
- `verdict.json`: verifier-derived gate failures and `GO_FOR_INTEGRATION_REVIEW` or `NO_GO_FOR_INTEGRATION`;
- `inventory.json`: exact file set, sizes, hashes, and final commit marker.

Schemas deny unknown fields, non-integer numeric evidence, negatives, NaN/infinity, secret/header-shaped keys, raw output, raw stderr/stdout, environment values, local paths, prompts, completions, and arbitrary event labels. Evidence is created under a new private empty directory using exclusive no-follow operations, inventory last, then replayed from held descriptors.

Acceptance thresholds are manifest-owned and never read from evidence as authority. Any missing monitor, malformed output, resource overflow, unknown file/field, mixed run/generation, or cleanup uncertainty is a failure.

## Lifecycle/adversarial matrix

Before candidate execution, deterministic fixtures must prove:

- reviewed aggregate mismatch and mutable/substituted tooling rejection;
- redirect scheme/host/port drift, redirect loops, missing length, oversized stream, truncation, digest mismatch, and interrupted transfer cleanup;
- archive traversal, absolute path, duplicate path, symlink/hard-link/device/FIFO, file-count, extracted-byte, and build-output overflow rejection;
- dirty/symlink/hard-linked candidate/evidence directories and cross-volume activation rejection;
- build network denial with an unsandboxed positive control and expected denial errno;
- malformed/unknown/non-finite evidence, self-declared thresholds, mixed run/model/runtime/generation, output/path/secret leakage, and unexpected evidence files;
- malformed/truncated/late/flooded frames, request replay, wrong generation, helper crash, stderr/output overflow, cancellation races, rapid resubmit, app-close, provider switch, and bounded drop;
- process/listener/RSS monitor failure, transient RSS/process spikes, no-fork enforcement, and residue detection;
- scripted provider remains the active rollback and product bundle behavior is unchanged.

## Cold-cache and UI measurements

The disposable CLI has no trustworthy separate ready event. Its conservative latency gate measures fresh process launch through the first visible token with built-in warmup disabled; it does not claim a reboot-equivalent cache purge or a separate post-ready interval. A later production helper must expose explicit model-ready and first-token events before product integration.

Native UI responsiveness uses app-main-thread heartbeat instrumentation while the model workload runs. Browser test duration is not a substitute. The measurement must capture maximum stall in integer microseconds and bind it to the exact candidate/run.

## Cleanup and rollback

The spike has no product activation path. Whether it passes or fails, the default terminal action is:

1. terminate and reap the exact process tree;
2. close all model/runtime descriptors;
3. delete model temporary and promoted staging files;
4. delete source archive, extracted source, build tree, private HOME/TMPDIR, and caches;
5. fsync parents where supported;
6. verify every manifest-owned candidate path is absent and no process/listener remains;
7. preserve only strict redacted evidence and the reviewed source contract.

Retaining any model/runtime byte requires separate approval after evidence review. Product rollback remains the scripted provider with no preference/schema migration.

## Acceptance and stop conditions

Transfer approval is blocked until two independent reviewers accept the exact reviewed aggregate with no unresolved P0-P2.

Integration is blocked unless all twenty behavior cases and every resource, offline, lifecycle, UI, evidence, cleanup, license, and identity gate pass. No threshold is tuned after observing candidate output. A failure yields `NO_GO_FOR_INTEGRATION`, cleanup, and a documented recommendation; it is not repaired by changing prompts or scorer under the same candidate identity.

No commit, push, deployment, archive regeneration, external release, UI redesign, model retention, or AI activation is authorized by this design.
