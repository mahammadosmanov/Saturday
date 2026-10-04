# Spike 001: self-contained local conversation runtime

Date: 2026-09-19

## Question

Given the pinned `llama.cpp` v0.4.1 source commit and the official Qwen3-4B Q4_K_M GGUF, when inference runs locally on the measured 16 GB Apple M5 target, does the candidate satisfy the resource, performance, behavior, offline, lifecycle, and evidence gates required before J.A.R.V.I.S integration?

## Verdict: PARTIAL runtime feasibility; INVALIDATED integration acceptance

The candidate demonstrated useful local Metal performance inside the resource budget, including the exact ten × 256-token stability matrix under kernel-enforced network denial. It is **not approved for product integration**.

The scripted offline provider remains the only active provider. No model, runtime, native command, installer, helper, IPC surface, or preference was added to the app.

## Frozen candidate

- Runtime source: `ggml-org/llama.cpp` v0.4.1 commit `b29c606e28a01b1bc8c1351026a0fa6e616bf6c4`.
- Direct runtime binary used for behavior probes: `llama-completion`, SHA-256 `61367c1f67c9d609530126279294cdb02bf33f91d253e904e506a64ca3883f13`.
- Model: official `Qwen/Qwen3-4B-GGUF` at repository commit `bc640142c66e1fdd12af0bd68f40445458f3869b`.
- Artifact: `Qwen3-4B-Q4_K_M.gguf`, 2,497,280,256 bytes, SHA-256 `7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5`.
- Final evidence run: `68655077-46b9-4dcb-9c53-3d1b81218c7a`.
- Final evidence manifest SHA-256: `46adc611044af55fefc39a22dfe73c80a44339c75c1b727f5bce69e4dee760b5`.

## What worked

### Resource and throughput observations

- Exact stability matrix: ten continuous 256-token generations.
- Network boundary for that matrix: macOS `sandbox-exec` profile `(version 1)(allow default)(deny network*)`.
- Average generation: 37.618 tokens/second.
- Minimum sample: 37.2365 tokens/second.
- Stability peak RSS: 2,642,460,672 bytes, about 2.46 GiB.
- Stability swaps: zero.
- Supplemental sustained run: 617.30 seconds, 175 × 128-token generations, 36.338 tokens/second average, 2,658,713,600-byte peak RSS, zero swaps.
- The model artifact remained below the 4 GiB ceiling; observed runtime RSS remained below the 6 GiB steady-state and 8 GiB peak ceilings.

These measurements support only the narrow claim that pinned local Metal inference is technically feasible on this machine within the experiment’s resource and throughput budgets.

### Limited lifecycle observations

- Direct process cancellation after first output ended in 18.9 ms without forced kill.
- Missing and deliberately truncated model files were rejected.
- All 102 Chromium/WebKit UI tests and the frontend check completed while inference load was active.

These are limited observations. They do not prove a helper acknowledgement protocol, full process-tree ownership, absence of late actions, native UI frame pacing, app shutdown cleanup, provider switching, or installer rollback.

## What did not pass

### Model behavior

The original synthetic scorer marked 17/20 cases as passing. The failed cases were:

- string reversal;
- next-prime calculation;
- GiB-to-MiB conversion.

The original scorer also had independently identified acceptance weaknesses, so 17/20 is retained only as an informational observation, not a quality certification.

### Required evidence still missing

- Controlled cold-cache model-ready time and a separate post-ready first-token measurement.
- Native UI-thread stall measurement proving no interaction is blocked longer than 100 ms.
- A signed app-owned helper with framed private IPC and explicit cancel acknowledgement.
- Process-group ownership, descendant RSS accounting, whole-tree termination, and no-surviving-process proof.
- App-close, rapid-resubmit, provider-switch, and no-late-token/action lifecycle tests.
- Installer consent, atomic activation, interruption recovery, Settings disclosure, one-action disable/delete, and scripted-provider rollback.
- An independently accepted, strict, manifest-bound evidence harness.

## Independent review result

Two independent reviews rejected the disposable harness. Verified findings included:

- fail-open direct-PID socket/process monitoring;
- self-reported and fabricable acceptance fields;
- evidence not consistently bound to one immutable candidate/run;
- a downloader path that could bypass preflight limits;
- unsafe predictable temporary evidence paths and path-alias risks;
- persisted output excerpts and inadequate secret-field controls;
- mismatch between documented gates and verifier semantics;
- missing product cleanup/rollback and native UI responsiveness evidence.

The disposable harness, its tests, and its unbound evidence were removed rather than promoted into project tooling. The retained archive intentionally records `NO-GO_FOR_INTEGRATION` and contains no completions, stderr excerpts, local model paths, model weights, or runtime binaries.

## Surprises

- Upstream `llama-cli` v0.4.1 behaved as a client over an internal server path and opened a socket, so it was rejected as the private-stdio helper candidate.
- Upstream `llama-completion` provided direct inference without that client/server surface and was used for the limited behavior probes.
- The candidate had ample raw throughput, but basic deterministic failures and incomplete acceptance evidence still block integration. Performance alone is not assistant quality or security.
- A passing browser suite under concurrent inference proves functional regression resistance, not the 100 ms native UI-thread requirement.

## Cleanup and rollback

The approved experiment required deletion unless installation received separate approval. Cleanup was completed and verified:

- temporary model artifact removed;
- cloned runtime source and build outputs removed;
- helper and benchmark binaries removed;
- unsafe disposable harness and its tests removed;
- old unbound evidence removed;
- no experiment inference process remains;
- shipped app and scripted provider unchanged.

The retained redacted evidence is under `.rps/evidence/local-model-spike-final/`. Its inventory binds nine evidence files, and the archive’s explicit verdict is `NO-GO_FOR_INTEGRATION`.

## Recommendation for the real build

Do not integrate this model/runtime yet. Before another model download or native provider implementation:

1. specify and independently review a strict manifest-bound evidence schema;
2. design the signed helper, framed IPC, process-group ownership, kernel network denial, resource accounting, and cancellation acknowledgement contract;
3. build adversarial RED tests for evidence fabrication, symlink/path aliasing, process descendants, network-monitor failure, non-finite metrics, output leakage, and cleanup;
4. add controlled cold-start and native UI-thread instrumentation;
5. rerun the same candidate or a comparison candidate only through that accepted harness.

A later model comparison must use the same corpus and exact gates. No Q5/Q6 or alternative model is selected by this spike.
