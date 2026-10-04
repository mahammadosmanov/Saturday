# RPS alignment and current acceptance contract

## Latest naming correction

Owner requires ASCII **J.A.R.V.I.S**, not dotted İ. All runtime branding and native app metadata now use I; bundle identifier and saved-data keys unchanged. Current rebuilt bundle: `../src-tauri/target/release/bundle/macos/J.A.R.V.I.S.app`. Historical dotted-name archives below remain untouched and are not the new candidate. Updated heading test failed in both engines before the fix; afterward `npm run check`, `npm run test:e2e` (100 browser/8 unit tests), `npm run tauri -- build --bundles app` and strict signature verification passed. CI-mode attempt initially refused occupied port 1420; existing Vite PID 5429 was verified by cwd/command as this checkout before reuse. The full suite reported closed-AudioContext unhandled-rejection diagnostics despite passing assertions; investigate separately before claiming a warning-free lifecycle suite. This naming-only change does not resolve the existing storage, compatibility or native acceptance gaps.

Date: 2026-09-07. Profile: P1, local personal UI prototype. Protocol: pinned Hermes starter adaptation 0.1.0, unchanged.

## Authority, scope and outcome

The owner requested that everything follow RPS and later approved evolving J.A.R.V.I.S into a modular, security-first total assistant. `ARCHITECTURE.md` is the current architecture direction. The current executable remains a P1 local prototype: roadmap capabilities are not implemented, authorized, or security-certified merely by being documented. Runtime third-party plugins, AI providers, recording/transcription, filesystem mutation, automation, computer control, network access, credential handling, commits, pushes, and deployment each retain their specific implementation and authority gates. A local build is not a public release.

**Current contract:** this document indexes the current accepted scope. The original [design](superpowers/specs/2026-09-07-dashboard-design.md) and [plan](superpowers/plans/2026-09-07-dashboard.md) are historical baselines. Their sidebar/telemetry/clock/fake-voice requirements are superseded by [compact refinement](compact-refinement.md) and [local voice plus header cleanup](voice-reactive-core.md). Do not restore removed UI or disable approved microphone visualization to satisfy old instructions. Other safety and accessibility requirements are not silently waived.

## Product and architecture decisions

- User/problem: Rohtolos wants a compact cinematic desktop interface that reacts visibly to speech, with optional assistant capabilities later. Success now is usable local interaction, not AI intelligence.
- Scope: one continuously mounted core, direct composer, Workspace and Settings header controls, secondary drawers and command palette. No header emblem, UI PREVIEW, clock or pause button. Pause remains in Settings/commands; reduced motion remains authoritative.
- Tauri + React/TypeScript/Vite remains the implemented stack. The original design records the Electron/SwiftUI alternatives and rationale; this pass makes no new comparative performance claim.
- The native shell loads bundled assets with restrictive CSP and no command capabilities. Browser and native storage are separate. Local objectives/preferences persist; conversations/activity do not. No account, server, external provider or runtime download.
- Microphone input is opt-in, local ephemeral analysis only. Settings stores a device ID, never audio. Never change system defaults or silently substitute a missing input. Capture ownership and cleanup belong to `useMicrophone`; rendering consumes a mutable audio signal, not per-frame React state.
- Simulated diagnostic and text responses must remain labeled at point of use. No recording, transcription, upload, speaker playback, telemetry or computer control is authorized.

## Requirements and executable evidence

| Requirement | Current evidence | Status / limitation |
| --- | --- | --- |
| Compact composition and removed header items | `tests/refinement.spec.ts`, `tests/desktop-fit.spec.ts`, `tests/quality.spec.ts` | Automated pass; current screenshot/aesthetic sign-off not claimed |
| Command keyboard navigation, dialogs and focus return | `tests/commands.spec.ts`, `tests/quality.spec.ts`, `tests/lifecycle.spec.ts` | Automated pass |
| Saved tasks, settings, reset, corrupt/unavailable storage | `src/domain/preferences.test.ts`, `tests/workspace.spec.ts`, `tests/quality.spec.ts` | Automated pass; localStorage is not encrypted |
| Clearly scripted, cancellable diagnostics/conversation | `tests/instruments.spec.ts`, `tests/commands.spec.ts`, `tests/lifecycle.spec.ts` | Automated pass |
| Real local voice analysis, selection, permission/cancellation/disconnect handling | `src/domain/audio.test.ts`, `tests/microphone.spec.ts` | Synthetic MediaStream + real Web Audio pass; not physical microphone certification |
| Strong speech response settles in silence | outer-core pixel regression in `tests/microphone.spec.ts` | Automated pass; owner reported prior capture works, stronger response acceptance remains unknown |
| Animation identity, phase, pause, hidden/reduced motion | `tests/refinement.spec.ts`, `tests/instruments.spec.ts`, `tests/microphone.spec.ts` | Automated pass; not native frame-time certification |
| Offline/no external runtime requests and automated accessibility | `tests/quality.spec.ts` | Automated pass; not exhaustive security or manual screen-reader certification |
| Packaged macOS app and rollback | Native checks and candidate manifest below | Local ad-hoc build only; not notarized |

## Fresh verification

Executed in this alignment pass on macOS with Node v22.23.2, npm 10.9.8, rustc 1.97.1. Local date/environment: `artifacts/rps-environment.txt`.

- `npm run check`: clean lint, 8/8 unit tests, TypeScript check and production build pass.
- `npm run test:e2e`: 100/100 Chromium/WebKit tests pass. Combined raw output: `artifacts/rps-checks.txt`.
- Plist lint, `cargo fmt --manifest-path src-tauri/Cargo.toml --check`, `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`, and strict deep signature verification pass. Output: `artifacts/rps-native.txt`.
- Native packaging was successfully built in the preceding header-cleanup pass. This pass rechecks the existing signed bundle rather than claiming a fresh native packaging run.
- `artifacts/rps-candidate.json` identifies tested source/tests by SHA-256, confirms bundle version 0.3.1 and exact packaged executable equality with the clean-header archive, and records CRC/SHA-256 checks for current and immediate rollback archives.
- Browser geometry/screenshots under `artifacts/voice/` predate the latest header cleanup; do not label them current visual evidence.

## Findings and remaining gates

1. **Reconciled:** README pointed to an older archive and advertised a removed clock; PROJECT linked only superseded design/plan; state and historical notes blurred native-owner evidence with automated evidence. Current pointers and explicit supersession replace those contradictions without inventing historical approvals.
2. **Open accessibility requirement:** the original design requested 44px compact touch targets; current header buttons are 36px high and composer controls 36×40px (`src/minimal.css`). Automated WCAG checks do not prove this project-specific target. A target-size UI change requires owner approval; it is not silently waived or implemented by this documentation pass.
3. **Unknown / owner gate:** physical microphone Stop/release behavior and the latest cinematic response in the packaged WKWebView need explicit owner observation. Earlier owner confirmation that capture works is evidence of usability, not all native gates passing.
4. **Unknown / not certified:** current native aesthetic acceptance, measured animation frame-time/CPU/memory budget, manual screen-reader accessibility, other platforms and OS minimum compatibility. No full security/dependency vulnerability audit was performed.
5. **Version-control checkpoint absent:** branch `feat/cinematic-dashboard` has untracked project files and no commit baseline. `git diff` cannot establish a reviewed patch for the existing implementation. Source hashes aid identification, but do not replace a committed checkpoint. Commit/push approval remains required.
6. **Release boundary:** ad-hoc signing is for local use. Distribution/notarization and expanded integrations require separate design/authority. No Gatekeeper bypass, credentials, installation or publish action is part of this pass.

## Delivery and recovery

Current app: `src-tauri/target/release/bundle/macos/J.A.R.V.İ.S.app`.

Current archive: `artifacts/jarvis-dashboard-v0.3.1-clean-header-macos-arm64.zip`.
SHA-256: `3b9d9b3f0b17000efaa400c566ff459a5653f21a8d3684c40b305a0b97ce66cb`.

Immediate rollback: `artifacts/jarvis-dashboard-v0.3.1-cinematic-macos-arm64.zip`.
SHA-256: `80b1aea59d90a48af6907f3b90db7c15a05fd7786403a76880bf5893b679fcf0`.

Both ZIP CRC checks pass. To recover, quit this app, verify the selected archive hash, extract into a separate directory, and open that copy; never overwrite active source or unrelated data. This pass performs no migration or reset. It does not exercise an actual rollback launch or destructive restore. Artifacts are gitignored local evidence and must be preserved separately if sharing a handoff.

## Review and alignment limitations

The complete read-only review has now arrived and been incorporated. It raised stale-instance storage loss, which the parent confirmed by source inspection: `src/hooks/useWorkspace.ts:39-52` loads once per instance and writes the whole snapshot on changes, with no storage-event reconciliation. Two tabs or app instances sharing an origin can overwrite each other's saved tasks. This is a conditional code-level data-loss risk, not a reproduced native failure. Do not use concurrent writers pending a dedicated persistence fix and multi-instance regression.

Parent inspection also confirms that the mount effect at `src/hooks/useWorkspace.ts:42-44` persists parsed defaults immediately after unreadable/unsupported data is encountered; the original envelope is not preserved for recovery. A safe quarantine/recovery design and tests are needed before claiming malformed-data preservation. Neither behavior was changed in this documentation pass. Existing storage tests passing does not close these gaps. The reviewer also reports a source-backed in-memory reproduction of a transient read failure followed by a successful write: saved tasks were overwritten while the UI still said session-only. Parent confirmed the unconditional write path; the injected probe was not rerun by the parent and is not a demonstrated native failure.

Additional review findings, not waived by the green suite:
- Minimum-runtime compatibility risk: `Conversation.tsx:32` uses `messages.at(-1)` unconditionally, while Vite targets Safari 15 and the native minimum is macOS 12.0. Reviewer cites [WebKit Safari 15.4 release notes](https://webkit.org/blog/12445/new-webkit-features-in-safari-15-4/) for Array.at support. Parent verified the source/config mismatch; old macOS execution is untested. Audit `crypto.randomUUID()` usages in the same minimum-runtime pass.
- Missing microphone regressions: actual React unmount; capture A cancelled, B started, then late A resolve/reject; delayed/rejected AudioContext.resume; application-owned AudioContext closure separately from fixture oscillator cleanup. Missing coverage is not proof of a leak.
- Runtime interruption handling remains unknown: context state is checked at startup, not continuously during sampling; suspended context with a live track needs a native reproduction before asserting a defect.
- Offline/network assertions cover home and diagnostics, not active microphone capture or audio persistence. Source review found no recorder/upload path, but no exhaustive privacy certification is claimed.
- `playwright.config.ts:31-33` can reuse an existing localhost server. The prior passing run did not independently prove server checkout identity. Future authoritative runs must use an isolated, owned server (CI disables reuse), or record verified process cwd/served-source identity. Dev-server results do not certify production CSP/native origin.

No additional concrete Core defect or unauthorized microphone/recording/upload path was identified in the read-only review. This is bounded source review, not comprehensive runtime certification.

A protected `AGENTS.md` edit was blocked because its approval prompt timed out. It remains unchanged; no workaround was attempted. Existing AGENTS instructions already route through PROJECT and STATE, which now point to this contract. A direct additional AGENTS link remains optional and requires approval.

Validation of this documentation pass: all current/historical Markdown links and PROJECT pointers resolve; source/test hashes match the verified candidate; no instruction override files were found in the project root. No Git commit baseline exists. These checks establish internal handoff consistency, not complete product acceptance.

## 2026-09-20 prebuilt-runtime stage-1 remediation addendum

- The stage-1 harness under rejected aggregate `f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0` is superseded and must not authorize a transfer.
- The remediated 12-record harness is frozen at aggregate `380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0`; manifest SHA-256/sidecar `d0c06c5413c515ef5e1986de6cef7ec6251c6ca20a91c161a4de492972e8a6e3` replayed exactly with read-only manifest mode.
- `npm run test:prebuilt-stage1`: 43/43 pass, including an injected-transport five-layer lifecycle from current-manifest replay and durable capability consumption through metadata replay, per-transfer disk preflight, descriptor-relative download, bounded extraction, Mach-O/license/signature inventory, cleanup, sealed receipt publication, and fresh-process replay denial.
- `python3 -m py_compile ...` and the stage-1 CLI `--help` pass. `npm run check` is also fresh: Biome clean, 6 Vitest files / 20 tests pass, and TypeScript/Vite production build passes.
- No real connection factory was used by the lifecycle test. No bottle, runtime, model, or GGUF bytes were downloaded, and no authorization record/nonce was created in the repository.
- Implementation is locally verified, not approved for acquisition. Two fresh independent reviews and a separately frozen one-time owner capability remain required before bottle transfer; model transfer remains blocked behind a later freeze/review/approval.

### Second remediation

- Independent review rejected aggregate `380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0`; it is now revoked alongside `f5375688…` and cannot authorize transfer.
- The replacement is frozen at 12-record aggregate `1c49497d67bc21f3a1a3edf49bca82be180ca517fce3380ead4b99389c9e4bc0`, manifest SHA-256 `3db6545c5bcd944f6a2dd5f2d5506fa8cc58bd9a51a3b214f93f0d6b8915a8a3`, with manifest and sidecar mode `0444`.
- Authorization now binds private ledger and workspace-parent identities, uses a cross-process lock, and records exact workspace identity before transfer. Fresh-process recovery deletes only registered residue; a same-name unregistered directory is preserved.
- Receipt replay requires its external digest and matching durable run marker. Extraction enforces every layer's installed-size or expansion identity. Inspection deduplicates hard-linked content by inode, rejects noncanonical system paths, verifies CodeDirectory structure/page hashes, and preserves license file size/digest evidence without claiming content-based SPDX detection.
- Fresh checks: `npm run test:prebuilt-stage1` passes 60/60; `npm run check` passes Biome, 20/20 Vitest tests, TypeScript and Vite build; direct Python compilation, CLI help, `git diff --check`, and exact 12/12 manifest replay pass.
- No real network connection, bottle/runtime/model/GGUF transfer, commit, push, deployment, or release occurred. Two fresh independent reviews remain required before an owner approval request.

### Second-remediation review outcome

- Aggregate `1c49497d67bc21f3a1a3edf49bca82be180ca517fce3380ead4b99389c9e4bc0` was rejected and revoked. The caller could mint the authorization record, digest, nonce and alternate ledger because no owner-controlled trust anchor authenticated them.
- The two lifecycle findings are corrected in unfrozen source. Live disk preflight now runs after the final response headers and before the first body byte. Receipt publication now reconciles an exact ledger-bound destination after an interrupted `cleaned → published` transition without creating a workspace or touching the network.
- Fresh checks pass: 63/63 stage-1 tests and full `npm run check`. These checks do not close the owner-authentication blocker.
- Apple documents Secure Enclave P-256 keys as non-exportable and supports Keychain access controls that require user presence. A dormant signed native issuer using those controls is the recommended trust anchor, but creating that owner key and native approval surface requires explicit owner authorization. No key was created and no transfer occurred.

## Next bounded work

- Obtain owner decision for the 44px touch-target discrepancy before any UI change.
- Complete and record the native owner interaction check; keep synthetic and physical evidence separate.
- Before any broader release, agree measurable native performance/accessibility gates and perform a dedicated review. Do not treat this P1 alignment as production certification.
- Create a version-control checkpoint only after explicit commit approval. No active instruction authorizes a push.
