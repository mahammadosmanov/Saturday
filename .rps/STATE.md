# Current state

Updated: 2026-10-04.

## Saturday rename status

- Active project identity: Saturday; frontend storage, package, Tauri app, helper, native storage namespace, and candidate/schema identities use Saturday-derived values.
- Rename branch: `feat/saturday-rename`; frontend slice committed locally, native slice in progress. The original checkpoint and renamed source remain local; no remote is configured.
- Verification observed: frontend check passed (20 Vitest tests, TypeScript/Vite build); focused Chromium UI tests passed (7); all-feature Rust tests passed (105 across six suites); Python scripts passed (116); Rust formatting and all-target/all-feature Clippy passed.
- Native helper compilation passed for `aarch64-apple-darwin`. A complete app bundle has not been built after the rename.
- Frozen candidate review inputs and their derived manifests were rebaselined for Saturday at the owner's direction. Independent security review is still required before publishing.
- Public GitHub target and exact repository slug remain unconfirmed; no public repository or push exists.

## Prior project state (historical as of 2026-09-27)

## Goal and profile

- Direction: evolve J.A.R.V.I.S into a local-first, security-first personal assistant with modular, reversible features.
- Current executable: compact local prototype with trusted built-in modules and a provider-neutral conversation boundary.
- Active conversation provider: scripted offline preview only. No AI model, remote provider, model runtime, native inference command, credential, transcript store, or external plugin is active.
- Canonical architecture: [`../ARCHITECTURE.md`](../ARCHITECTURE.md).
- Current acceptance contract: [`../docs/rps-alignment.md`](../docs/rps-alignment.md).

## Implemented product slices

### Trusted built-in modules

- Compile-time trusted-module registry with ID/version/API validation, duplicate rejection, ordered activation, failed-state recording, and contribution cleanup.
- Nine command-palette commands are contributed by four trusted built-ins: `core.navigation`, `core.workspace`, `core.voice`, and `core.appearance`.
- Settings discloses built-in module ID, version, trusted status, and active state. It has no third-party installer.

### Provider-neutral conversation boundary

- `ConversationRuntime` normalizes bounded input and delegates through an injected provider contract.
- Requests carry `AbortSignal`; ownership checks suppress late replies/actions after close or cancellation.
- Current scripted replies remain behavior-compatible; provider failure returns a fixed safe response without action.
- The rejected knowledge-graph action remains absent.

### Native boundary

- Tauri application command surface is empty.
- No capability broker, plugin host, filesystem/network/process authority, secret store, AI runtime, or arbitrary plugin execution is active.
- Ordinary startup does not launch the bundled helper; only the native CLI self-test can launch it.

### Private-helper experiment foundation

- Canonical result: [`../docs/spikes/002-private-helper-boundary/README.md`](../docs/spikes/002-private-helper-boundary/README.md).
- Verdict: **GO for the non-shipping experiment harness; NO-GO for a production helper or model integration**.
- Strict framed protocol, exact nested schemas, run/generation challenges, fixed budgets, no-fork/no-network default sandbox, independent resource monitoring, cancellation/late-frame fencing, sealed lifecycle receipts, descriptor-relative evidence I/O, and RAII cleanup are implemented in the Rust library.
- The deterministic helper is compiled only by Cargo feature `fixture-helper`; the integration test also requires that feature. The release app contains no fixture helper and exposes no Tauri command; the separate dormant signed candidate is described below.
- Three independent review rounds were resolved. Final review: PASS with no P0/P1/P2 defect in the non-shipping harness.
- The experiment remains non-shipping; its pathname limitation is superseded for the dormant production candidate by live same-team validation.

### Signed app-owned helper candidate

- Canonical result: [`../docs/spikes/003-signed-app-helper/README.md`](../docs/spikes/003-signed-app-helper/README.md).
- Verdict: **GO for the dormant local Apple Development signed-helper boundary; NO-GO for model integration or distribution release**.
- Tauri bundles exactly one `jarvis-runtime-helper` plus a sealed identity resource. The fixture helper remains excluded.
- Security.framework validates the directly owned live peer against the current process's non-ad-hoc team before `Hello`; the helper validates its immediate parent before reading stdin.
- The native-only self-test verifies current-source generation, exact staged pre-sign bytes, signed-helper hash, hardened runtime, exact entitlements, strict framed lifecycle, cancellation, bounded output, clean exit, and verified cleanup.
- The webview still has no invoke handler, shell plugin, capability entry, or helper permission.
- Independent lifecycle review finished with no unresolved P0-P2. Packaging findings were fixed through outer-seal preflight, exact staged-byte signing, locked/rechecked generation, hardened runtime, empty helper entitlements, and immutable rejection regressions.
- Remaining release gates: Developer ID signing, notarization, distribution lifecycle, and XPC/audit-token authentication before privileged services.

## Local-model spike 001

- Contract: [`../docs/local-model-feasibility.md`](../docs/local-model-feasibility.md).
- Candidate research: [`../docs/local-model-candidates.md`](../docs/local-model-candidates.md).
- Canonical result: [`../docs/spikes/001-local-model-runtime/README.md`](../docs/spikes/001-local-model-runtime/README.md).
- Verdict: **PARTIAL runtime feasibility; NO-GO for integration**.
- Frozen candidate: `llama.cpp` v0.4.1 commit `b29c606e28a01b1bc8c1351026a0fa6e616bf6c4`, direct `llama-completion` SHA-256 `61367c1f67c9d609530126279294cdb02bf33f91d253e904e506a64ca3883f13`, official Qwen3-4B Q4_K_M GGUF at repository commit `bc640142c66e1fdd12af0bd68f40445458f3869b`, 2,497,280,256 bytes, SHA-256 `7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5`.
- Exact stability observation under macOS kernel network denial: ten continuous 256-token generations, 37.618 tokens/second average, 37.2365 minimum, 2,642,460,672-byte peak RSS, zero swaps.
- Supplemental duration observation: 617.30 seconds, 175 × 128-token generations, 36.338 tokens/second average, 2,658,713,600-byte peak RSS, zero swaps.
- Original synthetic scorer reported 17/20 cases; reverse, next-prime, and GiB-to-MiB failed. Its scorer/harness was independently rejected, so this count is informational only.
- Integration blockers: an approved production model candidate, controlled cold-ready/post-ready latency, native 100 ms UI-thread budget, model installer lifecycle, Settings disclosure/removal, rollback, and signed-candidate evidence using the accepted boundary.

## Independent review and cleanup

- Two independent reviews rejected the disposable spike harness for fail-open process/network monitoring, fabricable verifier semantics, unbound evidence, preflight bypass, unsafe temp/path alias behavior, raw-output retention, gate mismatches, and missing cleanup/product-lifecycle proof.
- The unsafe disposable harness and its tests were removed instead of retained as project tooling.
- Temporary model, source checkout, runtime/build binaries, and old unbound evidence were deleted. No experiment inference process remains.
- The shipped app and scripted provider were not modified by the model experiment.
- Redacted final archive: [`evidence/local-model-spike-final/`](evidence/local-model-spike-final/).
- Evidence run ID: `68655077-46b9-4dcb-9c53-3d1b81218c7a`.
- Manifest SHA-256: `46adc611044af55fefc39a22dfe73c80a44339c75c1b727f5bce69e4dee760b5`.
- Archive verification checked ten regular non-symlink JSON files, finite values, shared run/manifest identity, nine inventory hashes, no raw excerpts/local paths, and explicit `NO-GO_FOR_INTEGRATION`.

## Local-provider source-build candidate 004

- Canonical result: [`../docs/spikes/004-local-provider-candidate/README.md`](../docs/spikes/004-local-provider-candidate/README.md).
- Verdict: **NO-GO before transfer**. No runtime source, model, or GGUF bytes were downloaded.
- The Qwen3.5-4B Q4_K_M metadata choice remains plausible, but the pinned local `llama.cpp` source-build path cannot meet whole-tree process ownership on macOS: a build descendant can call `setsid()` and close inherited descriptors, escaping process-group and sentinel cleanup.
- Multiple independent review rounds also hardened the retained dormant Rust model-provider state machine and native main-thread probe. The unsafe Python acquisition/build/evidence harness and its tests were deleted rather than retained.
- The next runtime candidate must use an immutable prebuilt arm64 artifact with a published digest and no local source-build process tree, or a real VM/job boundary that can prove descendant ownership.

## Prebuilt runtime candidate 005 / stage 1

- Canonical candidate: [`../docs/spikes/005-prebuilt-runtime-candidate/README.md`](../docs/spikes/005-prebuilt-runtime-candidate/README.md). No bottle, runtime, model, or GGUF bytes were downloaded.
- Candidate metadata aggregate `c034041a5ea9b5203e5e8385b010079764ff3daf13be00d5163e934cc11d447c` remains unchanged. Harness aggregates `f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0` and `380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0` were rejected and are revoked in code.
- Second-remediation aggregate `1c49497d67bc21f3a1a3edf49bca82be180ca517fce3380ead4b99389c9e4bc0` was independently rejected and is now revoked. Its manifest SHA-256 was `3db6545c5bcd944f6a2dd5f2d5506fa8cc58bd9a51a3b214f93f0d6b8915a8a3`; current source no longer matches it.
- The two repairable review findings are corrected: disk preflight now runs after final blob headers and immediately before body reading, and exact receipt publication can reconcile a `cleaned` or `published` durable marker without a workspace or network retry.
- Remaining P1 blocker: the CLI caller can still mint the authorization record, expected digest, nonce, and alternate private ledger. Stage 1 needs an owner-controlled trust source unavailable to the invoking process. The recommended next slice is a dormant signed app-owned authorization issuer using Secure Enclave/Keychain user-presence signing, with no webview exposure and no transfer authority until separately approved.
- Status: **NO-GO for transfer; current bytes are unfrozen pending an owner decision on the native authorization trust anchor**. Model transfer remains absent and blocked.

## Security corrections retained

- Fixed-key XOR/plain-SQLite “secure storage,” premature native commands, duplicate plugin UI, and disabled accessibility linting remain removed.
- Runtime third-party plugins remain blocked by the external-plugin security gate.
- Microphone capture remains user-triggered, local, ephemeral, and non-transcribing.

## Social Studio

- User-selected scope: plan, publish, and analytics; connect accounts individually; one calendar across channels.
- Implemented a full-window Social Studio with Planner, Posts, Publishing, Channels, Inbox, and Insights views, plus a cross-channel weekly calendar and post composer.
- Planner drafts are stored locally under `jarvis.social-studio.v1`; the interface supports search, filters, editing, and deletion. Calendar search filters planned items; event cards open in the composer; every day supports adding another post. The upcoming list omits past-dated items. Storage errors and cross-window conflicts are surfaced.
- Includes 18 major platforms: YouTube, Instagram, Facebook, X, LinkedIn, TikTok, Threads, Pinterest, Reddit, Bluesky, Telegram, Mastodon, Snapchat, WhatsApp Channels, Google Business Profile, Twitch, Discord, and Tumblr.
- Account OAuth, publishing, inbox data, and analytics are not connected or active. The user’s requested product scope does not enable external account access; follow the existing integration/authority gates before implementing live providers.
- UI follows the supplied Rohtolos Design System: restrained monochrome surfaces, strong contrast, and compact, task-focused hierarchy. JARVIS does not use Rohtolos’ mark as app branding.
- Current `npm run check`: lint clean; 6 Vitest files / 20 tests pass; TypeScript and Vite production build pass.
- Current Social Studio browser tests: 8 tests pass across Chromium/WebKit, including axe checks across each view and composer.
- Current `npx playwright test --project=chromium --project=webkit`: 110/110 pass.
- Next safe action: define an individually authorized OAuth/account capability and provider-specific permissions before implementing live connection, publishing, inbox, or analytics.

## Verification status

- Current `npm run check`: lint clean; 6 Vitest files / 20 tests pass; TypeScript and Vite production build pass.
- Current `npm run test:prebuilt-stage1`: 63/63 candidate, transport, extraction, inventory, adjacent-preflight, receipt-recovery, crash-recovery, cleanup, and injected end-to-end lifecycle tests pass.
- Current source is intentionally unfrozen. The last 12-record manifest is revoked and stale after P2 corrections.
- Current `npx playwright test --project=chromium --project=webkit`: 110/110 Chromium/WebKit tests pass.
- Current default Rust tests: 45 unit tests pass and the fixture suite is correctly skipped.
- Current all-feature Rust tests: 45 unit + 13 adversarial integration tests pass.
- Current `cargo fmt --check` and all-target/all-feature Clippy with warnings denied: pass.
- Current Python helper-build/release suite: 13 tests pass; direct compilation passes.
- Current native release build: pass; local Apple Development signed bundle finalized at `src-tauri/target/release/bundle/macos/J.A.R.V.I.S.app`.
- Signed verifier: one helper, exact current-source generation, exact outer/nested hardened-runtime signatures, matching non-empty team, app-only audio entitlement, empty helper entitlements, sealed binary hash, and self-test pass. Not notarized.
- Fresh ordinary launch verified as PID `47644`; no helper process launched.
- Release containment scan: exactly one dormant production helper; no fixture helper, invoke handler, shell plugin, capability entry, or helper permission.
- Residue scan: no helper process/private temporary root, `.gguf`, model staging directory, or old unsafe harness remains.
- Repository still has no initial commit; all project files remain untracked on `feat/cinematic-dashboard`. No commit or push occurred.

## Open product risks

- Local-storage multi-writer overwrite and malformed-envelope recovery remain unresolved; avoid concurrent writers.
- Local/remote AI, transcription, memory, file indexing, automation, computer control, provider credentials, and network access are not implemented.
- No full security audit, native performance certification, physical microphone certification, notarization, commit, push, or deployment exists.

## Next safe action

Obtain the owner's architecture decision for a dormant signed app-owned authorization issuer backed by Secure Enclave/Keychain user presence. Do not freeze or review another transfer candidate until that trust anchor exists. Do not execute acquisition or download a model/runtime without a separately frozen one-time capability and fresh explicit owner approval. Keep the helper dormant and inaccessible to the webview.
