# Stage 1 native owner-authorization design

Date: 2026-09-20
Status: approved design; documentation only; implementation, key provisioning, transfer, and release remain unperformed

## Decision

Add a dormant native owner-authorization subsystem to the signed J.A.R.V.I.S macOS app. A non-exportable P-256 private key must be generated in the Secure Enclave and stored through Security.framework under the stable application tag `dev.rohtolos.jarvis.stage1-owner-authorization.v1`. Every private-key use must require macOS user presence through Touch ID or the device password.

The subsystem has native command-line modes only. It is not exposed through Tauri IPC, JavaScript, the webview, ordinary startup, or the bundled runtime helper. Its only transfer-related operation creates one signed, short-lived, one-time stage-1 authorization and immediately launches the exact reviewed Python stage-1 harness from the signed app bundle. The private key, authentication context, password, biometric result, and raw credential material never leave Security.framework or enter a child process.

The Python harness must verify the authorization signature against an immutable public-key trust anchor sealed into the signed app before its first filesystem mutation or network request. It must then consume the generation exactly once in an app-owned durable ledger, use only app-owned workspace and receipt locations, preserve the existing crash-recovery and receipt-reconciliation guarantees, and retain `MODEL_TRANSFER_BLOCKED` as an invariant.

No software key fallback is allowed. Secure Enclave absence, unsupported access control, key mismatch, failed user presence, signature failure, expired authority, wrong app-owned directory identity, or failure to retrieve the same key after a same-identity rebuild is a hard failure.

## Scope

This slice may:

- add a Security.framework C bridge and a safe Rust wrapper for Secure Enclave provisioning, public-key retrieval, signing, continuity proof, and authenticated deletion;
- add dormant native CLI modes for provisioning, continuity proof, stage-1 issuance/execution, and deletion;
- add a tracked public-key trust-anchor resource and a frozen stage-1 binding resource;
- bundle the exact reviewed Python stage-1 runtime files as signed app resources;
- make the Python harness verify the native signature through Security.framework using a minimal `ctypes` adapter rather than a new cryptographic implementation;
- move the canonical stage-1 ledger, workspace parent, and receipts under app-owned Application Support;
- add strict TDD coverage for provisioning orchestration, issuance, validation, one-time consumption, expiry, crash recovery, receipt recovery, deletion, rollback, and bundle verification;
- perform a local Apple Development two-phase bootstrap and continuity proof after implementation and before any authorization can be used.

This slice may not:

- add a Tauri invoke handler, shell plugin, JavaScript command, capability entry, UI control, or ordinary-startup action;
- expose the private key, authentication context, nonce bytes, device password, biometric result, bearer token, authorization signature, or local paths in logs or retained evidence;
- use `security`, OpenSSL, shell scripts, or a subprocess to create, retrieve, sign with, or delete the key;
- create or accept a software P-256 fallback;
- accept caller-selected candidate, review manifest, trust anchor, ledger, workspace parent, receipt directory, receipt filename, Python script, verifier library, or network origin;
- download a bottle, runtime, model, or GGUF while implementing or certifying this slice;
- enable model transfer, inference, product activation, remote providers, or plugins;
- treat Apple Development signing as Developer ID distribution or notarization;
- commit, push, deploy, publish, or provision a key as part of writing this design.

## Existing boundary retained

The current app already has:

- an empty Tauri command surface;
- one dormant signed nested helper used only by `--verify-signed-helper`;
- a Security.framework process-identity bridge;
- a deterministic helper source-generation and bundle-finalization pipeline;
- a stdlib-only stage-1 Python harness with exact candidate replay, five-layer transport bounds, descriptor-relative extraction, durable one-time consumption, crash cleanup, receipt recovery, and a model-transfer block.

This design extends those boundaries rather than replacing them. The signed helper remains unrelated to stage-1 authorization and stays dormant. The Python transfer implementation remains the only stage-1 transport/extraction engine; the native app adds owner-controlled authority and a sealed launch boundary.

## Authority and threat model

### Protected authority

The only authority to mint a valid stage-1 capability is possession and successful user-presence use of the Secure Enclave private key whose public key is pinned into the reviewed final app and the bundled Python harness.

The caller may invoke the native CLI, run the Python interpreter, control its ambient environment, create arbitrary files elsewhere, and supply arbitrary command-line text to a directly invoked script. None of those actions may create valid authority. The approved path uses no caller-supplied paths or capability fields.

### Assumptions

- The user runs an intact locally Apple Development signed app candidate whose outer signature verifies strictly.
- The provisional and final candidates use the same bundle identifier, `dev.rohtolos.jarvis`, and the same Apple Development signing identity.
- The exact final source/resource candidate is independently reviewed before any stage-1 transfer approval.
- The operating system, Security.framework, Keychain, Secure Enclave, Touch ID/device-password policy, and Apple code-signing enforcement are trusted platform dependencies.

### Explicit non-claims

- An application tag is a lookup identifier, not authority by itself.
- A public-key hash is identity evidence, not a secret.
- A local Apple Development signature is not a distribution trust policy.
- A Python process is not a sandbox boundary. It is trusted only because the native app launches exact outer-sealed bytes and the Python code independently verifies the signed authorization before effects.
- This design does not defend a user who intentionally modifies and runs a different harness while ignoring failed review or signature checks.

## Platform key contract

### Key attributes

The C bridge must request exactly:

- key type: ECSECPrimeRandom;
- key size: 256 bits;
- token: `kSecAttrTokenIDSecureEnclave`;
- permanence: true;
- application tag: UTF-8 bytes of `dev.rohtolos.jarvis.stage1-owner-authorization.v1`;
- accessibility: `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`;
- access-control flags: `kSecAccessControlPrivateKeyUsage | kSecAccessControlUserPresence`.

The v1 design does not add a caller-selectable keychain access group and does not add a software-token retry. Security.framework must return a Secure Enclave key for the exact query or the operation fails closed.

### Algorithms and representations

- Public key: 65-byte uncompressed ANSI X9.63 P-256 representation returned from the public `SecKey` by `SecKeyCopyExternalRepresentation`.
- Key ID: lowercase SHA-256 of those 65 public-key bytes.
- Signature algorithm: `kSecKeyAlgorithmECDSASignatureMessageX962SHA256`.
- Signature representation: DER-encoded ANSI X9.62 ECDSA signature returned by Security.framework.
- Authorization signing input: ASCII domain separator `JARVIS-STAGE1-OWNER-AUTHORIZATION-V1\0` followed by the exact canonical payload bytes.

Neither Rust nor Python implements elliptic-curve arithmetic, DER signing, or signature verification. Rust calls the C Security.framework bridge for private-key operations. Python calls Security.framework directly through a small, fixed `ctypes` adapter for public-key construction and `SecKeyVerifySignature`.

### User-presence rule

Provisioning is complete only after the new or existing key signs a fixed bootstrap challenge and that signature verifies against its exported public key. Issuance signs the exact authorization message. Continuity proof signs a fixed proof challenge. Deletion first signs a deletion challenge bound to the pinned key ID and current ledger summary, verifies that signature, then deletes the exact key item and verifies that the item is no longer retrievable.

This makes each consequential key lifecycle operation depend on private-key use and therefore on the key's user-presence access control. A canceled prompt, lockout, unavailable passcode/biometrics, or authentication failure returns a fixed redacted error and performs no transfer.

### Secure Enclave unavailability

The following all produce `secure-enclave-unavailable` and no fallback:

- `kSecAttrTokenIDSecureEnclave` is unsupported or unavailable;
- key generation returns a non-Secure-Enclave item;
- the exact private key cannot be found by application tag;
- access-control creation or user-presence evaluation is unavailable;
- the public key is malformed or not 65-byte uncompressed P-256;
- Security.framework reports an unsupported signature algorithm.

## Two-phase local Apple Development bootstrap

The public key cannot be pinned before the device-bound private key exists, while the final app must pin the public key before it can issue authority. The approved bootstrap is therefore a two-phase signed flow.

### Phase A: provisional signed app

1. The repository contains a tracked trust-anchor resource in the exact unprovisioned form:

   ```json
   {"schema":"jarvis.local-provider.stage1-owner-public-key.v1","state":"unprovisioned"}
   ```

2. Build the app from reviewed source and sign it with a non-ad-hoc Apple Development identity, bundle identifier `dev.rohtolos.jarvis`, and the current exact app entitlement set.
3. Record the provisional app's designated requirement and strict signature result.
4. Run only `--stage1-owner-key-provision` from that signed app.
5. The native bridge creates or retrieves the exact Secure Enclave key, requires user presence by signing the bootstrap challenge, verifies the proof, and writes only a canonical public bootstrap record to the app-owned bootstrap directory. The private key is never exportable.
6. A deterministic pinning script validates that record and replaces the tracked trust-anchor resource with the exact pinned state:

   ```json
   {"algorithm":"ecdsa-p256-sha256-x962","key_id":"<sha256-of-x963-public-key>","public_key_x963_base64":"<canonical-base64>","schema":"jarvis.local-provider.stage1-owner-public-key.v1","state":"pinned"}
   ```

If the bootstrap user-presence proof is canceled or fails after Security.framework created the key, no bootstrap record is written and no authority is issued. A later provision attempt must retrieve the same exact tagged key and repeat the proof. Automatic unauthenticated deletion is forbidden; cleanup uses the authenticated delete mode.

The angle-bracket values above describe runtime-generated fields, not editable placeholders: the pinning script must derive both fields from the one validated public-key byte string and write canonical JSON atomically.

### Phase B: final signed candidate

1. Rebuild every source-generation-dependent native artifact after the pinned resource changes.
2. Freeze the exact stage-1 binding and reviewed runtime resource roster.
3. Build and sign the final app with the same bundle identifier and the same Apple Development signing identity.
4. Prove both directions of designated-requirement compatibility between provisional and final candidates.
5. Run `--stage1-owner-key-prove` from the final app. It must retrieve the original private key by the stable application tag, export the same public key, match the pinned key ID byte-for-byte, require user presence, and verify a proof signature.
6. Remove the build output, rebuild the same final source cleanly, sign it with the same identity, repeat designated-requirement checks, and run the proof again.

If either final build cannot retrieve and use the same key, or if the designated requirements are not mutually satisfied, the result is **STOP**. Do not issue authority, do not add a software key, do not silently change access groups, and do not transfer any artifact. Redesign the keychain ownership model and repeat independent review.

## Native command-line surface

The exact accepted native modes are:

- `--stage1-owner-key-provision`
- `--stage1-owner-key-prove`
- `--stage1-owner-authorization-issue`
- `--stage1-owner-key-delete`
- the existing `--verify-signed-helper`
- the existing strict main-thread diagnostic mode.

Each owner-authorization mode accepts no additional argument. Unknown, repeated, mixed, or extra arguments exit with a fixed argument error before Tauri builds a window.

Ordinary startup with no owner-authorization flag remains unchanged. It performs no keychain query, user-presence prompt, directory creation, Python launch, network request, stage-1 recovery, or helper launch.

### Provision

`--stage1-owner-key-provision`:

- requires a valid non-ad-hoc signed app candidate;
- creates or retrieves only the exact tagged Secure Enclave key;
- performs the bootstrap user-presence proof;
- atomically writes the public bootstrap record under Application Support with mode `0600`;
- prints only `JARVIS_STAGE1_OWNER_KEY_PROVISIONED` on success;
- never issues a transfer capability.

### Prove

`--stage1-owner-key-prove`:

- requires a pinned trust-anchor resource;
- retrieves the exact key, compares the public bytes and key ID with the pinned anchor, signs a fixed random challenge under user presence, and verifies it;
- writes no capability, ledger marker, workspace, or receipt;
- prints only `JARVIS_STAGE1_OWNER_KEY_CONTINUITY_OK` on success.

### Issue and execute

`--stage1-owner-authorization-issue`:

- requires a pinned trust anchor and an exact frozen binding resource;
- resolves the bundled Python harness and review root only relative to the running `.app` executable;
- validates the outer bundle/resource layout before prompting;
- opens the canonical ledger, workspace parent, and receipt parent;
- generates one 16-byte random generation and one 32-byte random nonce;
- assembles and signs one exact five-artifact authorization after user presence;
- launches `/usr/bin/python3` directly, never through a shell, with `-B -E -s`, working directory `/`, a fixed minimal environment, and the exact bundled script; the signed script directory is the only non-standard-library import root;
- passes the authorization envelope, nonce, and canonical directory descriptors through fixed inherited file descriptors, never argv or environment;
- waits for the bounded Python result, performs process cleanup, and emits only a fixed success token plus the public receipt digest or a fixed redacted error code.

The Python child receives no key, keychain handle, authentication context, password, biometric data, signing prompt, private-key bytes, or caller-selected path.

### Delete

`--stage1-owner-key-delete`:

- acquires the canonical ledger lock and refuses deletion while an active stage-1 child or unrecovered workspace exists;
- requires the pinned public key to match the retrievable private key;
- signs and verifies the deletion challenge under user presence;
- deletes only the exact tagged Secure Enclave key;
- verifies the key is absent;
- preserves completed receipts and durable ledger records;
- removes only the non-secret bootstrap public-key copy from Application Support;
- prints only `JARVIS_STAGE1_OWNER_KEY_DELETED` on success.

The tracked pinned public-key resource remains in source and in already-built apps. After device-key deletion, those candidates fail closed until a separately reviewed re-bootstrap updates the pinned resource.

## Canonical app-owned storage

The root is derived from the effective user's account record, not `HOME`, command-line input, or environment variables:

```text
~/Library/Application Support/dev.rohtolos.jarvis/Stage1OwnerAuthorization/
```

Exact children:

```text
bootstrap/owner-public-key.json
ledger/.jarvis-stage1.lock
ledger/.jarvis-stage1-generation-<32-lowercase-hex>.json
workspace/jarvis-prebuilt-<32-lowercase-hex>/
receipts/stage1-<32-lowercase-hex>.json
```

Requirements:

- app-owned subdirectories are real directories, effective-UID owned, mode `0700`, opened with no-follow semantics, and retained by descriptor;
- marker, bootstrap, and receipt files are regular, single-link, effective-UID owned, mode `0600`, and created exclusively;
- no caller chooses or overrides any path or filename;
- the signed payload binds device, inode, and UID for the ledger, workspace parent, and receipt parent, plus the exact receipt filename;
- the Python harness compares those signed identities with `fstat` on inherited descriptors before its first mutation;
- paths are absent from receipts, stdout, stderr, and retained evidence.

## Frozen binding resource

The final app contains one canonical resource with schema `jarvis.local-provider.stage1-owner-binding.v1`. It is generated from reviewed repository files and then independently reviewed.

To avoid a self-referential hash, freezing is deliberately two-layered:

1. Freeze a base harness review manifest over the exact source/runtime roster, including the pinned public-key resource, but excluding the derived binding resource, generated bundle copies, signatures, and build outputs.
2. Generate the binding deterministically from that already-frozen manifest, its sidecar, the candidate metadata/review, and the exact resource roster.
3. Compute the binding resource SHA-256 at runtime and include that digest in the signed authorization payload.
4. Independently review the derived binding and final signed app candidate as exact artifacts. The outer app seal binds the binding bytes, but the base manifest never attempts to include a hash of a file that contains the manifest's own aggregate.

The binding resource binds:

- candidate ID `qwen35-4b-q4km-homebrew-llamacpp-0.4.1`;
- candidate SHA-256 `08767112eaa0111244e5bb38381f8373615627ddb2af788a8fdd82464aa3a592`;
- candidate metadata review aggregate `c034041a5ea9b5203e5e8385b010079764ff3daf13be00d5163e934cc11d447c`;
- the fresh, non-revoked stage-1 harness review-manifest SHA-256 and review aggregate produced after this implementation is complete;
- the exact resource roster copied into the signed app;
- the exact five transfer artifacts and limits below;
- `model_transfer` equal to `blocked`.

The final harness aggregate must not equal any revoked aggregate:

- `f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0`;
- `380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0`;
- `1c49497d67bc21f3a1a3edf49bca82be180ca517fce3380ead4b99389c9e4bc0`.

The issuer and Python validator both reject those values even if a malformed binding resource contains one.

## Exact authorized artifacts and limits

The payload contains exactly this ordered artifact array:

1. `llama.cpp` `0.4.1`; repository `homebrew/core/llama.cpp`; layer SHA-256 `eb6fb1624c1d7efc90d94092d95b120fbdcff8ab3c0fe148ba6108f9c44d3a2e`; transfer bytes `7,707,135`; installed bytes `19,291,937`.
2. `ggml` `0.24.0`; repository `homebrew/core/ggml`; layer SHA-256 `31e217a029d28065ffff52cdb09fa975a56717216b663fe95a4ff84dcac15640`; transfer bytes `1,350,818`; installed bytes `4,775,140`.
3. `libomp` `23.1.1`; repository `homebrew/core/libomp`; layer SHA-256 `41257106da80d8d2f35bd2effc1877292b3462b00ebd5e85108c56dd0fdff879`; transfer bytes `597,172`; installed bytes `1,811,000`.
4. `openssl@3` `3.6.4`; repository `homebrew/core/openssl/3`; layer SHA-256 `51eb22ee51d7b6e678ef5dc2be234adb46f9f541ac1d98e5ed08c9a7535789dc`; transfer bytes `10,965,156`; installed bytes `37,759,406`.
5. `ca-certificates` `2026-08-13_1`; repository `homebrew/core/ca-certificates`; layer SHA-256 `37f5c1413f65c05d86e1279d76699f5d27ad77f8126f5776db13f3d64ee3d010`; transfer bytes `112,184`; maximum extracted bytes `16,777,216`; post-install execution forbidden.

Global limits:

- artifact count: exactly `5`;
- verified layer total: exactly `20,732,465` bytes;
- transfer ceiling: `67,108,864` bytes;
- maximum total extracted bytes: `536,870,912`;
- maximum archive entries: `20,000`;
- minimum free bytes before each body read: `2,818,572,288`;
- network origins and redirect policy remain exactly those in the current candidate;
- formula and post-install execution remain forbidden.

The issuer derives this array from the frozen binding resource. Python derives the expected array independently from the bundled candidate metadata and requires exact equality. Count-only or total-only matching is insufficient.

## Signed authorization format

### Canonical envelope

```json
{
  "algorithm": "ecdsa-p256-sha256-x962",
  "key_id": "<64-lowercase-hex>",
  "payload_base64": "<canonical-payload-bytes>",
  "schema": "jarvis.local-provider.stage1-owner-authorization-envelope.v1",
  "signature_base64": "<DER-ECDSA-signature>"
}
```

The envelope itself and decoded payload must both equal the repository's canonical JSON encoding: sorted object keys, no insignificant whitespace, ASCII-only strings, no duplicate keys, no non-finite values, and no trailing newline. Base64 must use the standard alphabet with required padding and round-trip exactly.

### Canonical payload

The payload schema is `jarvis.local-provider.stage1-owner-authorization-payload.v1` and has exactly these top-level keys:

```text
artifacts
binding_sha256
candidate
expires_at_unix
issued_at_unix
generation
ledger
limits
model_transfer
nonce_sha256
purpose
receipt
review
schema
state
workspace_parent
```

Required values:

- `state`: `granted`;
- `purpose`: `stage1_bottle_transfer`;
- `generation`: 32 lowercase hexadecimal characters generated from 16 random bytes;
- `issued_at_unix`: integer seconds;
- `expires_at_unix`: exactly `issued_at_unix + 300`;
- `nonce_sha256`: lowercase SHA-256 of the separate 32-byte nonce;
- `binding_sha256`: lowercase SHA-256 of the exact derived binding resource sealed into the app;
- `candidate`: exact ID, SHA-256, and candidate review aggregate;
- `review`: exact fresh harness manifest SHA-256 and aggregate;
- `artifacts`: the exact ordered five records above;
- `limits`: the exact global values above;
- `ledger`, `workspace_parent`, and `receipt`: exact signed descriptor identities; `receipt` also contains `stage1-<generation>.json`;
- `model_transfer`: `blocked`.

Python checks the signature and all schema semantics before checking or creating a ledger marker. It rejects an authorization when the current Unix time is earlier than `issued_at_unix` or not earlier than `expires_at_unix`. Expiry is checked once, immediately before one-time consumption; an already-consumed lifecycle may finish after the five-minute window.

## Python verification boundary

The bundled Python runtime must perform this order:

1. Read bounded envelope and nonce bytes from fixed inherited descriptors.
2. Load the pinned public-key resource only from the script's exact signed resource directory.
3. Parse strict canonical envelope and payload bytes.
4. Recompute and compare the key ID.
5. Construct a public `SecKey` from the pinned X9.63 bytes and call `SecKeyVerifySignature` with `kSecKeyAlgorithmECDSASignatureMessageX962SHA256` over the domain-separated payload.
6. Validate generation, nonce hash, issue/expiry times, candidate identity, candidate review aggregate, fresh harness review identity, exact five artifacts, all limits, model block, and exact receipt name.
7. Replay the bundled frozen binding, candidate, harness manifest, sidecar, resource roster, and executing source identity.
8. Compare signed ledger/workspace/receipt identities with `fstat` on the inherited descriptors.
9. Only then acquire the durable ledger lock and perform the first filesystem mutation.
10. Register the one-time generation and workspace before any network request.

The Security.framework adapter has no generic library/path parameter and loads only the operating-system Security and CoreFoundation frameworks by absolute system framework path. Tests inject a verifier interface above this adapter; production has no permissive verifier.

## One-time lifecycle and recovery

### State machine

The durable generation marker uses these states:

```text
issued-in-memory -> authorized -> cleaned -> published
                              \-> failed_cleaned
                              \-> recovered
```

`issued-in-memory` is not persisted. The first durable state is created with `O_CREAT | O_EXCL` after all signature and semantic validation passes and before network access.

The marker binds:

- generation;
- authorization-envelope SHA-256;
- candidate and review identities;
- run ID once assigned;
- workspace registration while present;
- cleanup digest;
- receipt digest and exact destination registration;
- final state.

A second process using the same generation or envelope fails before network access. A new owner-present issuance always creates a new generation and nonce; it never revives a consumed capability.

### Crash recovery

Under the canonical ledger lock, each launch:

- removes only an exact workspace registered in a durable marker and matching parent/root device, inode, UID, mode, and deterministic generation name;
- never deletes an unregistered lookalike directory;
- changes an `authorized` marker with recovered residue to `recovered`;
- reconciles a valid `cleaned` receipt to `published` without creating a workspace or using the network;
- rejects missing, altered, hard-linked, symlinked, noncanonical, wrong-destination, or semantically invalid receipts;
- preserves one-time consumption even after abrupt termination.

### Receipt

The receipt remains the existing strict redacted stage-1 receipt. Its authorization section changes to include the generation, key ID, envelope digest, binding digest, purpose, and nonce hash. It contains no signature bytes, public-key bytes, authorization envelope, local path, user identity, prompt result, environment, headers, bearer token, model data, or raw output.

The receipt verdict remains exactly `MODEL_TRANSFER_BLOCKED`.

## Build, resource, and signing effects

### Source generation

The deterministic native source-generation allowlist must include every owner-authorization Rust/C source, the strict CLI parser, the tracked trust anchor, the frozen binding, bundle preparation script, Python verifier, stage-1 harness, relevant tests, package scripts, Tauri configuration, entitlements, and bundle verifier/finalizer changes.

Changing the public key, binding, harness, verifier, CLI surface, resource map, or lifecycle logic therefore changes source generation and requires rebuilding the helper and app. A stale helper identity or resource bundle is rejected.

### Bundle resources

The final app contains exactly one stage-1 resource root under:

```text
Contents/Resources/jarvis/stage1/
```

It contains the pinned public key, frozen binding, exact executable Python harness, candidate metadata and sidecar, candidate review manifest and sidecar, fresh harness review manifest and sidecar, and the exact closed review roster needed for replay. The bundle verifier rejects missing, duplicate, extra, symlinked, writable, hard-linked, or hash-mismatched entries.

### Signing

The existing finalizer continues to sign the nested helper first and the outer app last. The outer resource seal therefore binds the owner trust anchor, binding, Python harness, manifests, and review root as well as the nested helper identity.

The app entitlement set remains the current exact microphone entitlement for this design. No keychain-sharing entitlement is added speculatively. Cross-build key retrieval is accepted only through the mandatory designated-requirement and continuity proof. Failure triggers STOP and redesign rather than an entitlement or software-key workaround inserted during implementation.

## Error and output policy

Native and Python public failures use fixed codes only. They must not include OSStatus text, CFError descriptions, prompts, paths, tags, key IDs, signatures, nonce bytes, manifest contents, headers, tokens, or exception strings.

Approved public success output:

- `JARVIS_STAGE1_OWNER_KEY_PROVISIONED`
- `JARVIS_STAGE1_OWNER_KEY_CONTINUITY_OK`
- `JARVIS_STAGE1_OWNER_AUTHORIZATION_OK:<receipt-sha256>`
- `JARVIS_STAGE1_OWNER_KEY_DELETED`

Approved error namespace:

```text
arguments
bundle
binding
secure-enclave-unavailable
key-not-found
key-mismatch
user-cancelled
authentication
signature
expired
canonical-storage
already-consumed
recovery
python-launch
stage1-failed
delete-refused
```

Internal errors may be classified for tests but must map to this redacted public set.

## TDD and acceptance requirements

Every behavior is implemented as a vertical RED → minimal GREEN → focused regression cycle. Required evidence includes:

- strict CLI parsing and ordinary-startup non-action;
- exact Security.framework query construction through an injected backend;
- no software fallback after Secure Enclave errors;
- provisioning idempotence and bootstrap proof;
- pinned-key mismatch rejection;
- exact canonical payload/envelope bytes shared by Rust and Python fixtures;
- real Security.framework verification of a software-generated ephemeral P-256 test signature, without Keychain or Secure Enclave use;
- wrong key, changed payload, malformed DER, noncanonical JSON, duplicate keys, wrong base64, expiry, future issue time, nonce mismatch, wrong generation, revoked review, wrong artifacts/limits, and model-unblocked rejection;
- zero ledger/workspace/receipt mutation and zero transport calls on every pre-consumption rejection;
- exact inherited descriptor identity enforcement;
- one-time generation exclusion across processes;
- workspace registration before network;
- abrupt-exit recovery and unregistered-lookalike preservation;
- cleaned/published receipt recovery without network;
- user-presence-gated deletion orchestration, active-run refusal, exact key deletion, and absence proof;
- bundle resource/source-generation/finalizer/verifier checks;
- provisional → final → clean-final designated-requirement compatibility and same-key retrieval;
- ordinary signed app launch with no prompt, no Python process, no helper process, no Application Support mutation, and no network;
- fresh two-review independent exact-candidate assessment with no unresolved P0, P1, or P2 before any transfer request.

Tests and build verification do not provision a real key unless the owner explicitly runs the signed manual bootstrap command. Automated tests use fakes or ephemeral in-memory/software Security.framework keys and must not touch Keychain or Secure Enclave.

## Rollback and deletion

Code rollback and device-key deletion are separate actions.

Before removing the feature from source on a device where provisioning occurred:

1. use the still-signed compatible app to acquire the canonical ledger lock;
2. recover or refuse active stage-1 state;
3. invoke `--stage1-owner-key-delete`;
4. satisfy user presence;
5. verify the exact tagged key is absent;
6. preserve completed receipts and ledger records;
7. remove only empty transient bootstrap/workspace directories;
8. then remove the owner-authorization code, resources, scripts, tests, package scripts, resource mappings, and source-generation entries.

If authenticated deletion cannot complete, rollback stops. Do not strand a live private key and then remove the only reviewed deletion path. If a completed transfer receipt exists, preserve its receipt and ledger provenance even after key deletion.

## Final gate

Implementation may be locally built and tested only after separate execution approval. Even after all automated checks and the two-phase continuity proof pass, stage-1 transfer remains blocked until:

1. the exact final source/resource/app candidate is frozen;
2. the fresh harness manifest and aggregate are non-revoked and replay successfully;
3. two independent reviewers check the same exact signed candidate and continuity evidence;
4. every verified P0-P2 finding is resolved;
5. the owner explicitly approves one stage-1 bottle transfer using the signed CLI ceremony.

Model transfer remains blocked after stage 1 and requires a separate relocated-runtime freeze, separate capability, separate nonce/generation, independent review, and fresh owner approval.
