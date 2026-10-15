# Saturday architecture

## Status

The compact UI and native-only bilingual voice implementation are in active integration. Frontend and Tauri source include a versioned strict protocol, native session supervisor, capture, local inference interfaces, speech, asset lifecycle and owner authorization. This describes source state, not a verified runtime: clean integrated compilation, reviewed role artifacts, frozen assets, owner trust, physical capture/inference/speech, signing and installed-app proof remain pending.

## Product direction

Saturday is intended to become a local-first personal assistant whose features can be added, upgraded, disabled, and maintained independently. Security and recoverability take precedence over feature count.

## Invariants

1. **Local by default.** No network, process launch, filesystem write, recording, transcription, or automation without a feature-specific contract and user-visible authorization.
2. **Deny by default.** A feature receives only declared capabilities. Missing capability means no access, not a fallback.
3. **No secret values in the webview.** Future provider credentials belong in the operating-system keychain and are referenced through opaque handles.
4. **No fake security.** Plain SQLite, XOR, hard-coded keys, and an in-process hash chain must not be described as encrypted or tamper-proof.
5. **No arbitrary plugin execution.** Runtime third-party plugins stay disabled until the external-plugin gate below is satisfied.
6. **Reversible delivery.** Feature enablement, schema migration, model installation, and automation require rollback paths.
7. **Auditable sensitive actions.** Future filesystem writes, process launches, remote requests, and computer control must produce user-readable records without logging secrets.
8. **The existing safety contract remains active.** Microphone input is user-triggered and ephemeral; simulations remain labeled; accessibility and offline tests remain gates.

## Current modular boundary: trusted built-ins

The first platform slice uses compile-time TypeScript modules. These modules are part of the signed application and share its process; they are an organizational and lifecycle boundary, **not a sandbox**.

Each built-in module has:

- a stable reverse-domain ID;
- a semantic version and core API version;
- declared command and panel contributions;
- `activate(context)` and optional `deactivate()` lifecycle hooks;
- deterministic duplicate-ID rejection;
- cleanup ownership for everything it registers;
- no direct native authority beyond capabilities explicitly present in its context.

The registry is intentionally small. Existing UI components remain shared until a vertical feature slice proves a useful boundary; duplicating `App.tsx` into a plugin is prohibited.

Bulk `activateAll()` is transactional for modules activated during that call: if a later activation fails, earlier activations from that call are disposed in reverse order and returned to retryable `inactive`. The failing module remains `failed` with its activation error, while modules active before the call are untouched. Rollback continues through disposer failures and rethrows the original activation error.

## Voice implementation boundary

```text
React shell
  ├─ Trusted built-in command registry
  ├─ Native voice-session hook and accessible conversation/setup UI
  └─ Strict raw-byte, versioned voice protocol client

Tauri shell
  ├─ Window-derived caller authorization and strict raw request decoder
  ├─ Bounded supervisor, snapshots, event polling and cleanup
  ├─ Native capture, local ASR/LLM worker and speech interfaces
  └─ Asset catalog, trust authorization and lifecycle controls
```

The browser preview has no assistant provider, microphone capture, Web Speech, scripted replies, or cloud fallback. `tests/fixtures/voice/native-protocol-driver.ts` is a browser-only protocol test double; its output is not native capability or readiness evidence.

Source-level implementation remains distinct from operational readiness. Build and source/resource seals, role-worker runtime artifacts, exact approved models, owner-personal trust setup, physical microphone and audible speech tests, measured local inference, signing and installed-app verification are separate gates. Never describe a snapshot-provided readiness field or static resource catalog as proof that those gates passed.

## Capability design

Capabilities are coarse enough to audit and narrow enough to revoke. Proposed namespaces are requirements, not implemented APIs:

- `microphone.capture`
- `audio.speak`
- `filesystem.read:<bookmark>`
- `filesystem.write:<bookmark>`
- `network.request:<origin>`
- `process.run:<approved-tool>`
- `secret.use:<provider>`
- `automation.execute:<recipe>`

Path strings supplied by a plugin are not authority. Future filesystem access must use user-selected security-scoped bookmarks or equivalent platform handles. Network authority is exact-origin allowlisting; redirects are revalidated. Process authority uses fixed executables and structured arguments, never shell strings.

## External-plugin gate (future)

Installable third-party plugins remain off until all of the following exist and are verified:

1. signed package format and trusted publisher policy;
2. immutable package digest and rollback copy;
3. manifest schema and API compatibility checks;
4. isolated execution boundary (separate helper process or reviewed WASM runtime);
5. resource limits and crash containment;
6. capability mediation entirely outside the untrusted runtime;
7. explicit install/upgrade permission review;
8. uninstall that preserves or exports user data safely;
9. malicious-plugin test corpus and independent security review.

A Web Worker alone does not satisfy this gate because it shares the origin and browser privileges.

## Data and secrets

- Non-sensitive preferences may continue in validated local storage while its known multi-writer and corruption-recovery gaps are addressed.
- Secrets use macOS Keychain through a minimal native command returning opaque success/failure, not secret text where avoidable.
- Sensitive databases require an approved encryption-at-rest design whose key is not stored beside the database.
- Memory/vector indexes must have deletion, export, retention, provenance, and rebuild semantics before use.

## Feature roadmap

Each item is a separate vertical slice with its own approval and verification:

1. **Trusted registry foundation**: lifecycle, duplicate rejection, contribution cleanup, static built-ins.
2. **Module manager UI**: list trusted built-ins, status, version, enable/disable where safe; no installer.
3. **Local bilingual voice**: native session, strict IPC, capture, ASR/LLM worker, speech, trust and asset source paths are implemented and being integrated. Reviewed runtime artifacts/assets, owner setup, native and physical runtime evidence remain pending; no cloud or scripted fallback is active.
4. **Memory**: explicit capture, searchable local history, provenance, deletion/export, bounded retention.
5. **Files and knowledge**: user-selected folders, read-only indexing first, write actions behind preview/approval.
6. **Automation**: declarative recipes, dry-run previews, per-action approval, idempotency and rollback.
7. **Remote providers**: opt-in only, exact disclosure of data sent, keychain-backed credentials, per-provider kill switch.
8. **Computer control**: least-privilege adapters, visible plan, approval gates, emergency stop, action receipts.
9. **External plugins**: only after the external-plugin gate passes.

## Verification and release

Every slice requires:

- a failing behavior test before production code;
- focused and broad tests;
- TypeScript/lint/build checks;
- Rust formatting/clippy/native build for native changes;
- offline/no-external-request checks unless the slice explicitly adds a reviewed network capability;
- migration and rollback evidence for persisted data;
- Keep a current verification record with unverified release gates explicit.

The source workflow in `.github/workflows/ci.yml` runs least-privilege checks on pushes and pull requests: frontend lint/unit/build, Chromium and WebKit browser tests, the Python regression suite, and macOS arm64 Rust fixture tests/formatting/clippy plus a clean-source, locally ad-hoc-signed app build. On macOS, it fetches locked Rust dependencies, then builds the app before Python and Rust tests so Tauri's source hooks generate the ignored public bootstrap resources and runtime-helper binary those tests consume. It does not reuse private generated artifacts. CI does not provision or authenticate an owner, use release credentials, publish artifacts, or perform Developer ID signing, hardened-runtime certification, or notarization.

Hosted CI has not been executed or claimed as passing; a hosted result requires an authorized push. Passing source checks or a local ad-hoc build does not certify distribution or replace physical-device verification.

No architecture document, green frontend suite, or local ad-hoc signature alone constitutes a production security certification.
