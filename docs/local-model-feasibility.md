# Local-model feasibility gate

## Status

This contract governed the completed disposable experiment. The approved temporary download was measured and deleted; no runtime or model was integrated. The result is **partial runtime feasibility and no-go for integration**, recorded in [`spikes/001-local-model-runtime/README.md`](spikes/001-local-model-runtime/README.md). A new download, dependency, native provider, or AI activation requires a new gate.

## Why this gate exists

J.A.R.V.I.S needs a replaceable local conversation provider before broader assistant features. A provider abstraction alone does not prove that a model is usable, private, supportable, or small enough for the target Mac.

Measured development-machine profile on 2026-09-19:

- Apple silicon arm64, Apple M5;
- 16 GB unified memory;
- 14.12 GiB free disk at measurement time;
- current native app bundle: 3.27 MiB.

No hardware identifiers are retained in this document.

## Non-negotiable experiment boundaries

- No Ollama, background daemon, separately installed CLI, or required external service.
- No remote inference, telemetry, account, API key, browser credential, or prompt upload.
- No automatic model download. A later user-approved experiment must show exact model name, source, license, download size, installed size, and removal action before transfer.
- The model is optional app data, not embedded in the default application archive.
- Conversation content remains session-only during the experiment.
- The current scripted provider stays available as an immediate fallback.
- The native Tauri command surface stays closed until the runtime candidate and IPC contract receive a separate threat-model review.

## Candidate requirements

A candidate must provide all of the following before implementation review:

1. Apple-silicon arm64 support from a maintained upstream project.
2. A license permitting the intended personal use and redistribution mode.
3. A versioned, immutable model artifact with published SHA-256 verification.
4. A self-contained integration path that can ultimately live inside `J.A.R.V.I.S.app` or an app-managed signed helper.
5. Streaming generation, cancellation, deterministic resource limits, and structured error reporting.
6. A bounded context configuration; the first proof targets 4,096 tokens or less.
7. No runtime code download, arbitrary dynamic library search path, shell-string execution, or plugin loading.

## Resource and usability budgets

The first proof is rejected if any hard gate fails on the measured 16 GB target:

| Gate | Required result |
| --- | --- |
| Installed model data | at most 4 GiB |
| Extra temporary download space | preflight requires artifact size plus installed size plus 2 GiB safety margin |
| Steady-state J.A.R.V.I.S + model RSS | at most 6 GiB |
| Observed peak RSS | at most 8 GiB |
| Cold model-ready time | at most 10 seconds |
| First visible token after submit | at most 5 seconds once ready |
| Sustained generation | at least 8 tokens/second over a 256-token sample |
| Cancel acknowledgement | at most 250 ms; no late tokens or actions |
| UI responsiveness | no blocked interaction longer than 100 ms on the UI thread |
| Stability | ten continuous 256-token responses without crash or unbounded memory growth |
| Offline proof | all prompts work with outbound traffic denied after installation |

These are experiment budgets, not current performance claims.

## Safety and lifecycle acceptance

A viable proof must also demonstrate:

- model consent and disk preflight before installation;
- download to a temporary file, digest verification before activation, and atomic promotion;
- interruption recovery that never treats a partial artifact as installed;
- exact model/version disclosure in Settings;
- one-action disable and delete, followed by verified removal;
- cancellation ownership across close, provider switch, app shutdown, and rapid resubmit;
- output rendered as untrusted text, never HTML;
- no tool execution from generated text;
- no prompt, completion, or model-path value in logs;
- provider failure returns the existing safe UI error and cannot trigger an action;
- scripted-provider rollback without changing stored user preferences.

## Experiment sequence

1. Research two maintained self-contained runtimes and two model candidates against the requirements above; record primary-source evidence and reject unsuitable licenses early.
2. Build a disposable command-line benchmark outside the production app. It must have no network path and must record only timing/resource counters.
3. Run the exact benchmark matrix on the target Mac and preserve machine-readable results without prompts.
4. Choose or reject a candidate. A failed budget is a valid stop result.
5. Only after a candidate passes, propose the native IPC schema, process/isolation model, model installer lifecycle, and threat model for separate approval.
6. Implement the native provider with RED/GREEN tests, then expose an explicit Settings switch while retaining scripted fallback.

The first experiment completed steps 1–4 and stopped. Runtime throughput, memory, exact ten × 256-token stability, and network-denied inference were demonstrated, but model behavior, evidence-harness security, cold-start proof, native UI responsiveness, helper/IPC ownership, and installation lifecycle did not clear the contract. Steps 5–6 remain unauthorized.

## Rollback

Before provider activation, rollback is deleting the disposable benchmark and candidate artifact. After a future integrated proof, rollback must disable the provider, terminate its owned runtime, remove app-managed model data on request, and restore the scripted provider without schema migration.

## Explicitly deferred

- accepted production model selection;
- any additional model or runtime download;
- native inference code;
- model-management UI;
- persistent conversation history or memory;
- retrieval, tools, voice transcription, automation, and remote providers;
- production security or performance certification.
