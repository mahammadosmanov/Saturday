# J.A.R.V.I.S architecture

## Status

Accepted direction; implementation is incremental. The current released behavior remains the compact local UI. Items marked **future gate** are not implemented or authorized merely because they appear here.

## Product direction

J.A.R.V.I.S is intended to become a local-first personal assistant whose features can be added, upgraded, disabled, and maintained independently. Security and recoverability take precedence over feature count.

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

## Runtime shape

```text
React shell
  ├─ Built-in module registry (trusted, static imports)
  ├─ Command contributions
  ├─ Provider-neutral conversation runtime
  │    └─ Scripted offline provider (current)
  ├─ Panel contributions
  └─ Existing domain hooks/components

Tauri shell
  ├─ No application commands yet
  └─ Dormant app-owned health helper (native self-test only)
```

The Tauri command surface remains empty during the registry foundation slice. Native capabilities are added one at a time, with an explicit threat model and tests at the IPC boundary.

### Experimental private-helper foundation

The Rust crate now contains a non-product helper boundary exercised only by a feature-gated deterministic fixture. It provides strict length-framed JSON, exact nested schemas, run/generation challenges, fixed resource budgets with independent monitoring, bounded nonblocking response delivery, default kernel denial of network and child-process creation, process-group termination for test-only descendant modes, sealed lifecycle receipts, descriptor-relative evidence I/O, and verified cleanup. The fixture is not bundled, no Tauri command exposes the boundary, and the scripted conversation provider remains active.

The fixture remains non-production. Its pathname limitation is addressed separately by the signed app-owned candidate below rather than by weakening the experimental acceptance boundary.

### Signed app-owned helper candidate

The app bundle now contains one dormant `jarvis-runtime-helper` nested executable and a sealed identity resource. Ordinary startup does not launch it. A native-only `--verify-signed-helper` mode verifies the resource hash, launches with fixed no-network/no-fork policy, mutually requires a live non-ad-hoc Apple team identity through Security.framework before sending framed input, then exercises health, cancellation, shutdown, bounded stderr, and process-group cleanup. The webview still has no invoke handler, shell plugin, capability, or helper permission.

The reproducible helper build uses an explicit source allowlist and embeds its generation identifier. The local candidate is finalized after nested signing so the outer resource seal binds the signed helper hash. Apple Development signing proves this local architecture only; Developer ID distribution, hardened-runtime release certification, notarization, and XPC/audit-token authentication for future privileged services remain mandatory release gates. No model or assistant capability runs in this helper yet.

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
3. **Local conversation runtime**: provider-neutral interface is active with the scripted offline provider. The first disposable `llama.cpp`/Qwen spike showed partial runtime feasibility but ended **no-go for integration**; see [`docs/spikes/001-local-model-runtime/README.md`](docs/spikes/001-local-model-runtime/README.md). No local model or native inference authority is active.
4. **Voice**: push-to-talk STT, then TTS; wake word only after privacy and false-activation certification.
5. **Memory**: explicit capture, searchable local history, provenance, deletion/export, bounded retention.
6. **Files and knowledge**: user-selected folders, read-only indexing first, write actions behind preview/approval.
7. **Automation**: declarative recipes, dry-run previews, per-action approval, idempotency and rollback.
8. **Remote providers**: opt-in only, exact disclosure of data sent, keychain-backed credentials, per-provider kill switch.
9. **Computer control**: least-privilege adapters, visible plan, approval gates, emergency stop, action receipts.
10. **External plugins**: only after the external-plugin gate passes.

## Verification and release

Every slice requires:

- a failing behavior test before production code;
- focused and broad tests;
- TypeScript/lint/build checks;
- Rust formatting/clippy/native build for native changes;
- offline/no-external-request checks unless the slice explicitly adds a reviewed network capability;
- migration and rollback evidence for persisted data;
- `.rps/STATE.md` checkpoint with unverified gates left explicit.

No architecture document, green frontend suite, or local ad-hoc signature alone constitutes a production security certification.
