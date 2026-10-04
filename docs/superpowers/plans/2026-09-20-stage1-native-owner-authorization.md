# Stage 1 Native Owner Authorization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a dormant signed macOS owner-authorization issuer backed by a user-presence-gated Secure Enclave P-256 key, then require the bundled Python stage-1 harness to verify and consume one exact short-lived authorization before any filesystem mutation or network transfer.

**Architecture:** Security.framework owns all private-key operations behind a narrow C bridge and Rust wrapper. The signed app exposes native CLI modes only, pins the exported public key through a two-phase Apple Development bootstrap, signs a canonical exact-candidate authorization, and launches the exact bundled Python harness with authorization, nonce, and app-owned directory descriptors on fixed inherited file descriptors. Python verifies the signature through Security.framework against the sealed public-key resource, validates the frozen five-artifact binding, consumes one generation durably, and retains the existing cleanup/receipt/model-block guarantees.

**Tech Stack:** macOS Security.framework and CoreFoundation; C FFI; Rust 2021, `serde`, `serde_json`, `sha2`, `getrandom`, `libc`, Tauri 2; Python 3 standard library and `ctypes`; existing unittest, Cargo, Clippy, Tauri, `codesign`, and package-script toolchain.

**Spec:** `docs/superpowers/specs/2026-09-20-stage1-native-owner-authorization-design.md`

## Global Constraints

- Documentation approval does not authorize implementation execution, key provisioning, Touch ID/device-password prompts, Keychain/Secure Enclave access, bottle/model download, commit, push, deploy, or publication.
- **Commit steps are forbidden and intentionally omitted.** Do not run `git add`, `git commit`, `git push`, or create a release.
- Use strict RED → minimal GREEN → focused regression for every behavior. A test that was not observed failing for the expected reason does not count as TDD evidence.
- Bundle identifier remains exactly `dev.rohtolos.jarvis`.
- Secure Enclave application tag is exactly `dev.rohtolos.jarvis.stage1-owner-authorization.v1`.
- Key type is non-exportable Secure Enclave P-256; private-key use requires `kSecAccessControlPrivateKeyUsage | kSecAccessControlUserPresence`; no software key fallback exists.
- Public key is 65-byte uncompressed ANSI X9.63 P-256. Signature algorithm is `kSecKeyAlgorithmECDSASignatureMessageX962SHA256`; signature bytes are Security.framework DER/X9.62 output.
- Authorization domain separator is exact ASCII `JARVIS-STAGE1-OWNER-AUTHORIZATION-V1\0`.
- Authorization TTL is exactly 300 seconds; generation is 16 random bytes encoded as 32 lowercase hex; nonce is exactly 32 random bytes and is never placed in argv, environment, logs, or evidence.
- Native owner modes accept no caller-supplied path or value. No Tauri invoke handler, shell plugin, JavaScript capability, UI control, ordinary-startup action, or helper action is added.
- Canonical storage root is the effective account's `Library/Application Support/dev.rohtolos.jarvis/Stage1OwnerAuthorization`; never trust `HOME`.
- Ledger, workspace parent, and receipt parent are app-owned mode-`0700` directories passed by inherited descriptor. Receipt filename is `stage1-<generation>.json` and is signed.
- Python must verify signature, expiry, nonce, binding, exact five artifacts/limits, model block, resource replay, and descriptor identities before its first filesystem mutation or network call.
- Current candidate identity remains `qwen35-4b-q4km-homebrew-llamacpp-0.4.1` with SHA-256 `08767112eaa0111244e5bb38381f8373615627ddb2af788a8fdd82464aa3a592` and candidate-review aggregate `c034041a5ea9b5203e5e8385b010079764ff3daf13be00d5163e934cc11d447c`.
- Reject revoked harness aggregates `f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0`, `380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0`, and `1c49497d67bc21f3a1a3edf49bca82be180ca517fce3380ead4b99389c9e4bc0`.
- Exact transfer set is five layers totaling `20,732,465` bytes, capped at `67,108,864` transfer bytes, `536,870,912` extracted bytes, `20,000` entries, and `2,818,572,288` free bytes immediately before each body read.
- Ordered artifact 1: `llama.cpp` `0.4.1`, `homebrew/core/llama.cpp`, layer `eb6fb1624c1d7efc90d94092d95b120fbdcff8ab3c0fe148ba6108f9c44d3a2e`, transfer `7,707,135`, installed `19,291,937` bytes.
- Ordered artifact 2: `ggml` `0.24.0`, `homebrew/core/ggml`, layer `31e217a029d28065ffff52cdb09fa975a56717216b663fe95a4ff84dcac15640`, transfer `1,350,818`, installed `4,775,140` bytes.
- Ordered artifact 3: `libomp` `23.1.1`, `homebrew/core/libomp`, layer `41257106da80d8d2f35bd2effc1877292b3462b00ebd5e85108c56dd0fdff879`, transfer `597,172`, installed `1,811,000` bytes.
- Ordered artifact 4: `openssl@3` `3.6.4`, `homebrew/core/openssl/3`, layer `51eb22ee51d7b6e678ef5dc2be234adb46f9f541ac1d98e5ed08c9a7535789dc`, transfer `10,965,156`, installed `37,759,406` bytes.
- Ordered artifact 5: `ca-certificates` `2026-08-13_1`, `homebrew/core/ca-certificates`, layer `37f5c1413f65c05d86e1279d76699f5d27ad77f8126f5776db13f3d64ee3d010`, transfer `112,184`, maximum extracted `16,777,216` bytes, post-install forbidden.
- `MODEL_TRANSFER_BLOCKED` remains mandatory. No runtime/model payload is downloaded while implementing, testing, bootstrapping, freezing, signing, or reviewing this slice.
- The app entitlement set remains the current exact microphone entitlement. Do not add a speculative keychain access group. If same-key retrieval does not survive the required same-designated-requirement rebuild proof, STOP and redesign.
- No key, password, biometric result, LA/Security context, signature, nonce, bearer token, local path, or raw OS error appears in logs or retained evidence.
- Preserve the signed helper behavior and its ordinary-startup dormancy.

---

## File map

### Create

- `src-tauri/native/owner_authorization.c` — exact Security.framework key create/query/sign/verify/delete bridge; no fallback.
- `src-tauri/src/platform/owner_authorization.rs` — safe Rust FFI wrapper and fixed redacted platform errors.
- `src-tauri/src/owner_authorization/mod.rs` — module exports and shared constants.
- `src-tauri/src/owner_authorization/canonical.rs` — strict ASCII canonical JSON and base64/hex constraints.
- `src-tauri/src/owner_authorization/binding.rs` — pinned public-key and frozen-binding schemas; exact artifact replay.
- `src-tauri/src/owner_authorization/paths.rs` — effective-account Application Support resolution and descriptor-owned directories.
- `src-tauri/src/owner_authorization/cli.rs` — one exact native-mode parser covering existing and new modes.
- `src-tauri/src/owner_authorization/issuer.rs` — provision/prove/issue/delete orchestration behind injected traits.
- `src-tauri/src/owner_authorization/process.rs` — fixed-FD Python launcher, bounded result, deadline, and cleanup.
- `src-tauri/resources/stage1-owner-public-key.json` — tracked unprovisioned then pinned trust anchor.
- `src-tauri/resources/stage1-owner-binding.json` — tracked unfrozen then exact derived binding.
- `src-tauri/tests/stage1_owner_authorization.rs` — native CLI/process integration tests with fake key and fixture Python child.
- `scripts/stage1_owner_authorization.py` — strict envelope/payload parser and Security.framework public verification adapter.
- `scripts/stage1_owner_roster.py` — one exact base review roster used by freeze/prepare/tests.
- `scripts/pin_stage1_owner_public_key.py` — validates canonical app-owned bootstrap output and atomically pins the tracked source resource.
- `scripts/freeze_stage1_owner_authorization.py` — freezes the non-self-referential base manifest and derives the binding.
- `scripts/prepare_stage1_owner_authorization_bundle.py` — verifies frozen inputs and copies exact resources into the generated bundle root.
- `scripts/verify_stage1_owner_authorization_bundle.py` — strict bootstrap/final bundle verifier and designated-requirement helper.
- `scripts/tests/test_stage1_owner_authorization.py` — Python signature/schema/effect-order tests.
- `scripts/tests/test_pin_stage1_owner_public_key.py` — public-key bootstrap/pinning tests.
- `scripts/tests/test_freeze_stage1_owner_authorization.py` — circularity-free freeze and exact artifact tests.
- `scripts/tests/test_prepare_stage1_owner_authorization_bundle.py` — generated resource-root tests.

### Modify

- `src-tauri/build.rs:1-12` — compile the second native bridge and preserve Security/CoreFoundation links.
- `src-tauri/Cargo.toml:8-43` — expose modules/tests without a new crypto dependency.
- `src-tauri/src/lib.rs:1-3` — export `owner_authorization`.
- `src-tauri/src/platform/mod.rs:1-2` — export the native owner bridge.
- `src-tauri/src/main.rs:3-39` — dispatch strict native modes before ordinary Tauri startup.
- `src-tauri/tauri.conf.json:6-60` — prepare resources before dev/build and bundle the exact generated stage-1 resource root.
- `scripts/prebuilt_runtime_stage1.py:9-1704` — consume verified owner authority, fixed FDs, canonical app-owned locations, generation ledger, and updated receipt authorization.
- `scripts/tests/test_prebuilt_runtime_stage1_candidate.py:1-332` — exact signed authorization and pre-effect rejection tests.
- `scripts/tests/test_prebuilt_runtime_stage1_transport.py:1-207` — assert transport remains unreachable before valid signed consumption.
- `scripts/tests/test_prebuilt_runtime_stage1_extract.py:1-365` — preserve extraction invariants under deterministic generation workspaces.
- `scripts/tests/test_prebuilt_runtime_stage1_runner.py:1-296` — fixed-FD end-to-end, crash, one-time, and receipt-recovery tests.
- `scripts/build_native_helper.py:24-46,220-270` — close source generation over the owner-authorization source/resource surface.
- `scripts/finalize_signed_helper_bundle.py:31-158` — require prepared owner resources before final outer signing.
- `scripts/verify_signed_helper_bundle.py:24-158` — compose the new owner bundle verifier with existing signed-helper verification.
- `scripts/tests/test_build_native_helper.py:16-79` — source-generation changes and resource inputs.
- `scripts/tests/test_signed_helper_bundle.py:20-204` — bootstrap/final bundle and no-command-surface regressions.
- `package.json:9-21` — add owner-auth test/freeze/prepare/verify scripts and sequence preparation before helper/app build.
- `docs/spikes/005-prebuilt-runtime-candidate/harness-review-manifest.json` — fresh base manifest generated only after all roster source is final; excludes itself, its sidecar, derived binding, generated copies, signatures, and build outputs.
- `docs/spikes/005-prebuilt-runtime-candidate/harness-review-manifest.json.sha256` — fresh manifest digest.
- `ARCHITECTURE.md:62-66,121-133` — record the dormant owner-authority boundary and unchanged model gate after implementation.
- `.rps/STATE.md:85-125` — record actual verification and next safe action after implementation; never pre-claim completion.

### Explicitly unchanged

- `src-tauri/Entitlements.plist` — remains app-only microphone entitlement.
- `src-tauri/HelperEntitlements.plist` — remains empty.
- frontend files and tests — no UI change.
- model/provider activation — remains absent.

---

### Task 1: Lock schemas, canonical bytes, and cross-language fixtures

**Files:**
- Create: `src-tauri/src/owner_authorization/mod.rs`
- Create: `src-tauri/src/owner_authorization/canonical.rs`
- Create: `scripts/stage1_owner_authorization.py`
- Create: `scripts/tests/test_stage1_owner_authorization.py`
- Modify: `src-tauri/src/lib.rs:1-3`
- Modify: `src-tauri/Cargo.toml:36-43`

**Interfaces:**
- Produces Rust constants: `OWNER_KEY_TAG`, `PUBLIC_KEY_SCHEMA`, `BINDING_SCHEMA`, `ENVELOPE_SCHEMA`, `PAYLOAD_SCHEMA`, `SIGNING_DOMAIN`, `AUTHORIZATION_TTL_SECONDS`.
- Produces Rust functions:
  - `canonical_json_bytes(value: &serde_json::Value) -> Result<Vec<u8>, CanonicalError>`
  - `decode_lower_hex<const N: usize>(value: &str) -> Result<[u8; N], CanonicalError>`
  - `decode_canonical_base64(value: &str, maximum: usize) -> Result<Vec<u8>, CanonicalError>`
- Produces Python functions:
  - `canonical_json_bytes(value: object) -> bytes`
  - `decode_json_object(payload: bytes, maximum_bytes: int) -> dict`
  - `decode_canonical_base64(value: str, maximum_bytes: int) -> bytes`
- Fixture payload and envelope bytes must be byte-identical in Rust and Python.

- [ ] **Step 1: Write RED Rust canonicalization tests**

Add tests in `canonical.rs` that construct the exact fixture below and assert one literal byte string:

```rust
let payload = serde_json::json!({
    "schema": PAYLOAD_SCHEMA,
    "state": "granted",
    "purpose": "stage1_bottle_transfer",
    "generation": "00112233445566778899aabbccddeeff",
    "issued_at_unix": 1_795_000_000_i64,
    "expires_at_unix": 1_795_000_300_i64,
    "nonce_sha256": "11".repeat(32),
    "binding_sha256": "22".repeat(32),
    "model_transfer": "blocked"
});
assert_eq!(
    canonical_json_bytes(&payload).unwrap(),
    br#"{"binding_sha256":"2222222222222222222222222222222222222222222222222222222222222222","expires_at_unix":1795000300,"generation":"00112233445566778899aabbccddeeff","issued_at_unix":1795000000,"model_transfer":"blocked","nonce_sha256":"1111111111111111111111111111111111111111111111111111111111111111","purpose":"stage1_bottle_transfer","schema":"jarvis.local-provider.stage1-owner-authorization-payload.v1","state":"granted"}"#
);
```

Also assert rejection of non-ASCII strings, floating-point values, booleans where integers are required by callers, invalid lowercase hex, unpadded/noncanonical base64, oversize payloads, and duplicate JSON keys on the Python side.

- [ ] **Step 2: Run the focused Rust test and verify RED**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::canonical -- --nocapture
```

Expected: FAIL because the module/functions/constants do not exist.

- [ ] **Step 3: Write RED Python fixture tests**

```python
class CanonicalOwnerAuthorizationTests(unittest.TestCase):
    def test_fixture_is_exact_ascii_sorted_json(self):
        payload = {
            "schema": auth.PAYLOAD_SCHEMA,
            "state": "granted",
            "purpose": "stage1_bottle_transfer",
            "generation": "00112233445566778899aabbccddeeff",
            "issued_at_unix": 1795000000,
            "expires_at_unix": 1795000300,
            "nonce_sha256": "11" * 32,
            "binding_sha256": "22" * 32,
            "model_transfer": "blocked",
        }
        self.assertEqual(auth.canonical_json_bytes(payload), EXPECTED_PAYLOAD)

    def test_duplicate_keys_nonfinite_nonascii_and_noncanonical_base64_fail(self):
        with self.assertRaises(ValueError):
            auth.decode_json_object(b'{"a":1,"a":2}', 1024)
        with self.assertRaises(ValueError):
            auth.decode_json_object(b'{"a":NaN}', 1024)
        with self.assertRaises(ValueError):
            auth.canonical_json_bytes({"a": "é"})
        with self.assertRaises(ValueError):
            auth.decode_canonical_base64("YQ", 8)
```

- [ ] **Step 4: Run the focused Python test and verify RED**

Run:

```bash
python3 -m unittest scripts.tests.test_stage1_owner_authorization.CanonicalOwnerAuthorizationTests -v
```

Expected: FAIL because `scripts.stage1_owner_authorization` does not exist.

- [ ] **Step 5: Implement the minimal canonical layer**

Use recursive key sorting and an explicit allowed JSON domain: null is forbidden, floats are forbidden, strings must be printable ASCII without control characters, arrays retain order, integer range is signed 64-bit, and object keys are unique printable ASCII. Do not use a cryptographic dependency.

Define constants in Rust and Python with exact matching values. Implement strict standard base64 by decode → re-encode equality and strict lowercase hex by exact length/alphabet.

- [ ] **Step 6: Run focused GREEN checks**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::canonical -- --nocapture
python3 -m unittest scripts.tests.test_stage1_owner_authorization.CanonicalOwnerAuthorizationTests -v
```

Expected: PASS with no warnings or extra output.

- [ ] **Step 7: Add a cross-language fixture command**

Add a Python test that invokes a small Rust unit-test fixture only through checked-in literal bytes, not by spawning product code. Both languages must compare against the same literal `EXPECTED_PAYLOAD`; neither may derive the expected bytes from the other implementation.

- [ ] **Step 8: Run task regression**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization -- --nocapture
python3 -m unittest scripts.tests.test_stage1_owner_authorization -v
```

Expected: PASS.

**Checkpoint:** inspect the diff for schema/name drift. Do not commit.

---

### Task 2: Implement canonical Application Support directories and inherited descriptor identities

**Files:**
- Create: `src-tauri/src/owner_authorization/paths.rs`
- Test: unit tests in `paths.rs`
- Modify: `src-tauri/src/owner_authorization/mod.rs`
- Modify later consumer seams only after this task is green.

**Interfaces:**
- Produces:

```rust
pub struct DirectoryIdentity {
    pub device: u64,
    pub inode: u64,
    pub uid: u32,
}

pub struct Stage1Directories {
    pub root: OwnedFd,
    pub bootstrap: OwnedFd,
    pub ledger: OwnedFd,
    pub workspace_parent: OwnedFd,
    pub receipt_parent: OwnedFd,
    pub ledger_identity: DirectoryIdentity,
    pub workspace_parent_identity: DirectoryIdentity,
    pub receipt_parent_identity: DirectoryIdentity,
}

pub trait AccountHomeResolver {
    fn effective_user_home(&self) -> Result<PathBuf, PathError>;
}

pub fn open_stage1_directories(
    resolver: &impl AccountHomeResolver,
) -> Result<Stage1Directories, PathError>;
```

- Production resolver uses `geteuid` + `getpwuid_r`, never `HOME`.
- All app-owned components are descriptor-relative, no-follow, effective-UID owned, and mode `0700`.

- [ ] **Step 1: Write RED tests for account-home and directory policy**

Cover:

```rust
#[test]
fn ignores_hostile_home_environment_and_uses_account_record() { /* fake resolver */ }

#[test]
fn creates_exact_private_children_and_returns_distinct_identities() { /* temp root */ }

#[test]
fn rejects_symlink_wrong_owner_wrong_mode_regular_file_and_alias_roles() { /* fixtures */ }
```

Use an injected resolver rooted in a private temporary directory. Do not access the real user's Application Support in automated tests.

- [ ] **Step 2: Verify RED**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::paths -- --nocapture
```

Expected: FAIL because the path API is absent.

- [ ] **Step 3: Implement descriptor-owned path creation**

Use `openat`, `mkdirat`, `fstat`, `fchmod`, `O_DIRECTORY`, `O_NOFOLLOW`, and `O_CLOEXEC`. Validate every created/opened app-owned directory after opening. Preserve parent descriptors for later fixed-FD inheritance. Reject any pre-existing unsafe component; never repair permissions silently except on a directory created by the current call.

The exact relative tree is:

```text
Library/Application Support/dev.rohtolos.jarvis/Stage1OwnerAuthorization/
  bootstrap/
  ledger/
  workspace/
  receipts/
```

- [ ] **Step 4: Run GREEN and a hostile-environment regression**

```bash
HOME=/private/tmp/attacker cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::paths -- --nocapture
```

Expected: PASS; tests continue using injected roots and production code never reads `HOME`.

- [ ] **Step 5: Add fixed filename helpers**

```rust
pub fn generation_marker_name(generation: &str) -> Result<String, PathError>;
pub fn workspace_name(generation: &str) -> Result<String, PathError>;
pub fn receipt_name(generation: &str) -> Result<String, PathError>;
```

Expected exact outputs for generation `00112233445566778899aabbccddeeff`:

```text
.jarvis-stage1-generation-00112233445566778899aabbccddeeff.json
jarvis-prebuilt-00112233445566778899aabbccddeeff
stage1-00112233445566778899aabbccddeeff.json
```

- [ ] **Step 6: Run task regression**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::paths -- --nocapture
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

Expected: PASS.

**Checkpoint:** no production path may accept a `PathBuf` from CLI or environment. Do not commit.

---

### Task 3: Add the Security.framework Secure Enclave bridge and fail-closed Rust wrapper

**Files:**
- Create: `src-tauri/native/owner_authorization.c`
- Create: `src-tauri/src/platform/owner_authorization.rs`
- Modify: `src-tauri/src/platform/mod.rs:1-2`
- Modify: `src-tauri/build.rs:1-12`
- Modify: `src-tauri/Cargo.toml:32-43`
- Test: Rust wrapper unit tests with an injected bridge result seam

**Interfaces:**

C ABI, with CoreFoundation-owned output objects released only by the bridge release function:

```c
typedef const void *jarvis_cf_data_ref;

int32_t jarvis_owner_key_provision(
    const uint8_t *tag, size_t tag_len,
    const uint8_t *challenge, size_t challenge_len,
    jarvis_cf_data_ref *public_key_out,
    jarvis_cf_data_ref *signature_out
);

int32_t jarvis_owner_key_copy_public(
    const uint8_t *tag, size_t tag_len,
    jarvis_cf_data_ref *public_key_out
);

int32_t jarvis_owner_key_sign(
    const uint8_t *tag, size_t tag_len,
    const uint8_t *message, size_t message_len,
    const char *prompt_utf8,
    jarvis_cf_data_ref *public_key_out,
    jarvis_cf_data_ref *signature_out
);

int32_t jarvis_owner_key_delete_after_proof(
    const uint8_t *tag, size_t tag_len,
    const uint8_t *challenge, size_t challenge_len,
    const char *prompt_utf8
);

void jarvis_owner_cf_release(const void *value);
```

Rust wrapper:

```rust
pub struct KeyProof {
    pub public_key_x963: Vec<u8>,
    pub signature_der: Vec<u8>,
}

pub enum OwnerKeyError {
    SecureEnclaveUnavailable,
    KeyNotFound,
    DuplicateKey,
    UserCancelled,
    AuthenticationFailed,
    InvalidPublicKey,
    InvalidSignature,
    System,
}

pub trait OwnerKeyBackend: Send + Sync {
    fn provision(&self, challenge: &[u8]) -> Result<KeyProof, OwnerKeyError>;
    fn copy_public_key(&self) -> Result<Vec<u8>, OwnerKeyError>;
    fn sign(&self, message: &[u8], prompt: &str) -> Result<KeyProof, OwnerKeyError>;
    fn delete_after_proof(&self, challenge: &[u8], prompt: &str) -> Result<(), OwnerKeyError>;
}
```

- [ ] **Step 1: Write RED Rust orchestration tests against a fake backend**

Tests must prove:

- Secure Enclave unavailable returns exactly `SecureEnclaveUnavailable` and is not retried against software storage.
- Provision accepts only 65 bytes beginning with `0x04` and a nonempty DER signature.
- Sign returns the public key each time so callers can compare it with the pinned anchor.
- User cancellation and authentication failure remain distinct internally but later map to redacted public codes.
- Delete calls one proof-sign operation and one exact delete operation in that order; no delete occurs when proof fails.

- [ ] **Step 2: Verify RED**

```bash
cargo test --manifest-path src-tauri/Cargo.toml platform::owner_authorization -- --nocapture
```

Expected: FAIL because the bridge/wrapper is absent.

- [ ] **Step 3: Implement the C query exactly**

Provision attributes must contain:

```text
kSecAttrKeyType = kSecAttrKeyTypeECSECPrimeRandom
kSecAttrKeySizeInBits = 256
kSecAttrTokenID = kSecAttrTokenIDSecureEnclave
kSecPrivateKeyAttrs:
  kSecAttrIsPermanent = true
  kSecAttrApplicationTag = exact tag bytes
  kSecAttrAccessControl = SecAccessControlCreateWithFlags(
      kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
      kSecAccessControlPrivateKeyUsage | kSecAccessControlUserPresence)
```

Exact lookup/delete query must include `kSecClassKey`, private key class, EC key type, Secure Enclave token ID, and exact application tag. Never issue a second query without `kSecAttrTokenIDSecureEnclave`.

Provision behavior:

1. query exact key;
2. if found, use it rather than creating a duplicate;
3. if not found, create exact key;
4. copy public key and X9.63 bytes;
5. sign the bootstrap challenge with user presence;
6. verify the signature using the public `SecKey`;
7. return only public key and signature.

Delete behavior signs and verifies the supplied deletion challenge first, then runs `SecItemDelete` with the exact query and confirms a subsequent query returns item-not-found.

Release every CF/Sec object on every path. Never return `CFError` text.

- [ ] **Step 4: Compile bridge and run GREEN**

```bash
cargo test --manifest-path src-tauri/Cargo.toml platform::owner_authorization -- --nocapture
cargo check --manifest-path src-tauri/Cargo.toml --all-targets --all-features
```

Expected: PASS. These tests use fakes and compilation only; they must not invoke Keychain or Secure Enclave.

- [ ] **Step 5: Add a source-policy regression**

In `scripts/tests/test_build_native_helper.py`, read `owner_authorization.c` and assert:

```python
self.assertIn("kSecAttrTokenIDSecureEnclave", source)
self.assertIn("kSecAccessControlPrivateKeyUsage", source)
self.assertIn("kSecAccessControlUserPresence", source)
self.assertNotIn("kSecAttrTokenIDAppleKeyStore", source)
self.assertNotIn("openssl", source.casefold())
```

Also assert there is exactly one key-generation call site and no query retry with a reduced attribute set.

- [ ] **Step 6: Run task regression**

```bash
python3 -m unittest scripts.tests.test_build_native_helper -v
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

Expected: PASS.

**Checkpoint:** automated work has not queried the real Keychain. Do not commit.

---

### Task 4: Parse and dispatch dormant native modes without ordinary-startup effects

**Files:**
- Create: `src-tauri/src/owner_authorization/cli.rs`
- Create: `src-tauri/src/owner_authorization/issuer.rs`
- Modify: `src-tauri/src/main.rs:3-39`
- Modify: `src-tauri/src/platform/main_thread_probe.rs:28-45` only to delegate parsing, not behavior
- Test: unit tests in `cli.rs` and `issuer.rs`

**Interfaces:**

```rust
pub enum NativeMode {
    Ordinary,
    VerifySignedHelper,
    MeasureMainThread(PathBuf),
    Stage1OwnerKeyProvision,
    Stage1OwnerKeyProve,
    Stage1OwnerAuthorizationIssue,
    Stage1OwnerKeyDelete,
}

pub fn parse_native_mode(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<NativeMode, NativeModeError>;

pub trait OwnerAuthorizationActions {
    fn provision(&self) -> Result<(), OwnerAuthorizationError>;
    fn prove(&self) -> Result<(), OwnerAuthorizationError>;
    fn issue_and_run(&self) -> Result<String, OwnerAuthorizationError>;
    fn delete(&self) -> Result<(), OwnerAuthorizationError>;
}
```

- [ ] **Step 1: Write RED parser tests**

Assert exact one-flag forms succeed and every mixed/repeated/extra form fails:

```rust
assert_eq!(parse(["jarvis"]), NativeMode::Ordinary);
assert_eq!(parse(["jarvis", "--stage1-owner-key-provision"]), NativeMode::Stage1OwnerKeyProvision);
assert!(parse(["jarvis", "--stage1-owner-key-provision", "extra"]).is_err());
assert!(parse(["jarvis", "--stage1-owner-key-prove", "--verify-signed-helper"]).is_err());
```

Retain exact existing behavior for `--verify-signed-helper` and `--measure-main-thread <output>`.

- [ ] **Step 2: Verify RED**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::cli -- --nocapture
```

Expected: FAIL because unified parsing does not exist.

- [ ] **Step 3: Implement strict dispatch before Tauri build**

`main` must parse arguments first. Owner modes call native actions and exit before `tauri::Builder::default()`. Ordinary mode must not construct `Stage1Directories`, query the key, read owner resources, recover a ledger, or launch Python.

Public output mapping:

```rust
Stage1OwnerKeyProvision => "JARVIS_STAGE1_OWNER_KEY_PROVISIONED"
Stage1OwnerKeyProve => "JARVIS_STAGE1_OWNER_KEY_CONTINUITY_OK"
Stage1OwnerAuthorizationIssue => format!("JARVIS_STAGE1_OWNER_AUTHORIZATION_OK:{digest}")
Stage1OwnerKeyDelete => "JARVIS_STAGE1_OWNER_KEY_DELETED"
```

Errors print only `JARVIS_STAGE1_OWNER_AUTHORIZATION_ERROR:<fixed-code>` and exit `78`; argument errors exit `64`.

- [ ] **Step 4: Add ordinary-startup non-action tests**

Use a fake `OwnerAuthorizationActions` with atomic call counters. Parse ordinary startup and assert every counter remains zero. Add a source regression that `invoke_handler` remains absent and Tauri capabilities remain empty.

- [ ] **Step 5: Run GREEN**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::cli -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::issuer -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib -- --nocapture
```

Expected: PASS.

**Checkpoint:** no real owner action has run. Do not commit.

---

### Task 5: Implement public bootstrap records and deterministic trust-anchor pinning

**Files:**
- Create: `src-tauri/resources/stage1-owner-public-key.json`
- Create: `scripts/pin_stage1_owner_public_key.py`
- Create: `scripts/tests/test_pin_stage1_owner_public_key.py`
- Create: pinned/public record parsing in `src-tauri/src/owner_authorization/binding.rs`
- Modify: `src-tauri/src/owner_authorization/issuer.rs`
- Modify: `package.json:9-21`

**Interfaces:**

Initial tracked resource bytes are exact:

```json
{"schema":"jarvis.local-provider.stage1-owner-public-key.v1","state":"unprovisioned"}
```

Bootstrap record schema and exact keys:

```text
algorithm
application_tag
key_id
public_key_x963_base64
schema
```

Pinned resource exact keys:

```text
algorithm
key_id
public_key_x963_base64
schema
state
```

Rust:

```rust
pub enum PublicKeyAnchor {
    Unprovisioned,
    Pinned { key_id: [u8; 32], public_key_x963: [u8; 65] },
}

pub fn parse_public_key_anchor(bytes: &[u8]) -> Result<PublicKeyAnchor, BindingError>;
pub fn bootstrap_record_bytes(public_key_x963: &[u8]) -> Result<Vec<u8>, BindingError>;
```

Python pinning entry point accepts no CLI path:

```python
def pin_from_canonical_bootstrap() -> dict:
    ...
```

- [ ] **Step 1: Write RED Rust and Python tests**

Rust tests reject unknown keys, wrong state, wrong algorithm, wrong tag, noncanonical JSON/base64, wrong X9.63 prefix/length, and mismatched key ID.

Python tests construct a fake effective-account home using injected functions and assert:

- bootstrap file must be regular, single-link, UID-owned, mode `0600`, bounded, and canonical;
- pin output is canonical and atomically replaces only `src-tauri/resources/stage1-owner-public-key.json`;
- key ID is independently recomputed from decoded bytes;
- symlink/hardlink/wrong-mode/wrong-tag/wrong-key-ID inputs fail without altering the tracked resource;
- no private key or signature field is accepted.

- [ ] **Step 2: Verify RED**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::binding -- --nocapture
python3 -m unittest scripts.tests.test_pin_stage1_owner_public_key -v
```

Expected: FAIL.

- [ ] **Step 3: Implement provision orchestration with a fake backend first**

Provision flow in `issuer.rs`:

```rust
let challenge = bootstrap_challenge(random_32_bytes()?);
let proof = key_backend.provision(&challenge)?;
validate_x963_public_key(&proof.public_key_x963)?;
verify_proof_with_platform(&proof, &challenge)?;
let record = bootstrap_record_bytes(&proof.public_key_x963)?;
write_bootstrap_record_exclusive_or_same(&directories.bootstrap, &record)?;
```

`write_bootstrap_record_exclusive_or_same` is idempotent only for byte-identical public identity. A different existing record is `key-mismatch`.

- [ ] **Step 4: Implement the pinning script**

Resolve the account home through `pwd.getpwuid(os.geteuid()).pw_dir`, open the exact Application Support path no-follow, validate the bootstrap file, produce the pinned resource, and atomically replace the tracked resource with mode `0644`. The script prints only canonical public metadata:

```json
{"key_id":"<64-hex>","pinned":true}
```

No command accepts an alternate input or output path.

- [ ] **Step 5: Run GREEN**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::binding -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::issuer -- --nocapture
python3 -m unittest scripts.tests.test_pin_stage1_owner_public_key -v
```

Expected: PASS without touching the real Application Support tree because tests inject the root.

- [ ] **Step 6: Add package script without running it**

Add:

```json
"pin:stage1-owner-public-key": "python3 scripts/pin_stage1_owner_public_key.py"
```

Do not execute this script during implementation until the separately authorized Phase A ceremony.

**Checkpoint:** tracked anchor remains `unprovisioned` until explicit owner-approved provisioning. Do not commit.

---

### Task 6: Verify ECDSA in Python through Security.framework before effects

**Files:**
- Modify: `scripts/stage1_owner_authorization.py`
- Modify: `scripts/tests/test_stage1_owner_authorization.py`

**Interfaces:**

```python
class SignatureVerifier(Protocol):
    def verify(self, public_key_x963: bytes, message: bytes, signature_der: bytes) -> bool: ...

class SecurityFrameworkSignatureVerifier:
    def verify(self, public_key_x963: bytes, message: bytes, signature_der: bytes) -> bool: ...

@dataclasses.dataclass(frozen=True)
class ParsedEnvelope:
    key_id: str
    payload_bytes: bytes
    payload: dict
    signature_der: bytes
    envelope_sha256: str
```

The production verifier loads only:

```text
/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation
/System/Library/Frameworks/Security.framework/Security
```

- [ ] **Step 1: Write RED parser and signature-order tests**

Create a fake verifier recording calls. Assert `parse_and_verify_envelope`:

1. rejects malformed envelope before calling verifier;
2. recomputes key ID from pinned public bytes;
3. calls verifier once with `SIGNING_DOMAIN + payload_bytes`;
4. rejects false verification;
5. returns a frozen `ParsedEnvelope` only after success.

Mutation tests change one byte in payload, signature, key ID, algorithm, schema, or base64 and require rejection.

- [ ] **Step 2: Verify RED**

```bash
python3 -m unittest scripts.tests.test_stage1_owner_authorization.SignatureEnvelopeTests -v
```

Expected: FAIL because verification is unimplemented.

- [ ] **Step 3: Implement strict envelope parsing with injected verifier**

Exact envelope keys are `algorithm`, `key_id`, `payload_base64`, `schema`, `signature_base64`. Require canonical envelope bytes and canonical decoded payload bytes. Bound envelope to 64 KiB, payload to 48 KiB, signature to 256 bytes, and public key to exactly 65 bytes.

- [ ] **Step 4: Write RED real Security.framework verification test**

The test creates a non-permanent software P-256 key through Security.framework, signs a fixed in-memory message, exports only its public key, and calls `SecurityFrameworkSignatureVerifier.verify`. It must not set `kSecAttrIsPermanent`, `kSecAttrApplicationTag`, `kSecAttrTokenIDSecureEnclave`, or touch Keychain.

Assert valid signature passes and changed message/signature/public key fail.

- [ ] **Step 5: Implement minimal `ctypes` adapter**

Bind only the CoreFoundation functions needed to build/release `CFData`, `CFNumber`, and `CFDictionary`, then bind `SecKeyCreateWithData`, `SecKeyVerifySignature`, and the fixed Security constants. Build a public EC 256-bit `SecKey` from the X9.63 bytes. Release every object in `finally`. Return `False` on signature mismatch; raise a fixed `SignatureVerificationError` on platform/setup failure without exposing CFError text.

- [ ] **Step 6: Run GREEN**

```bash
python3 -m unittest scripts.tests.test_stage1_owner_authorization.SignatureEnvelopeTests -v
python3 -m unittest scripts.tests.test_stage1_owner_authorization.SecurityFrameworkVerificationTests -v
```

Expected: PASS on macOS without Keychain/Secure Enclave mutation.

- [ ] **Step 7: Prove pre-effect ordering**

Patch `os.open`, `os.mkdir`, `fcntl.flock`, and the transport factory to raise `AssertionError` if called. Feed each invalid envelope case and assert rejection occurs without any patched effect.

- [ ] **Step 8: Run task regression**

```bash
python3 -m unittest scripts.tests.test_stage1_owner_authorization -v
```

Expected: PASS.

**Checkpoint:** Python uses Security.framework, not handwritten ECDSA or OpenSSL subprocesses. Do not commit.

---

### Task 7: Freeze the exact non-circular binding and resource roster

**Files:**
- Create: `scripts/stage1_owner_roster.py`
- Create: `scripts/freeze_stage1_owner_authorization.py`
- Create: `scripts/tests/test_freeze_stage1_owner_authorization.py`
- Create initial: `src-tauri/resources/stage1-owner-binding.json`
- Modify: `src-tauri/src/owner_authorization/binding.rs`
- Modify: `scripts/stage1_owner_authorization.py`
- Modify: `package.json:9-21`

**Interfaces:**

Initial tracked binding bytes are exact:

```json
{"schema":"jarvis.local-provider.stage1-owner-binding.v1","state":"unfrozen"}
```

`BASE_REVIEW_ROSTER` contains every execution/review source required by the spec, including the pinned anchor, but excludes:

- `harness-review-manifest.json`;
- `harness-review-manifest.json.sha256`;
- `stage1-owner-binding.json`;
- `src-tauri/generated/**`;
- `src-tauri/target/**`;
- `src-tauri/binaries/**`;
- signatures and app bundles.

Freeze API:

```python
def freeze(repo_root: Path) -> dict:
    """Atomically write the base manifest, sidecar, then derived binding."""
```

Binding parser:

```rust
pub struct FrozenBinding {
    pub binding_sha256: [u8; 32],
    pub candidate: CandidateBinding,
    pub review: ReviewBinding,
    pub artifacts: [ArtifactBinding; 5],
    pub limits: LimitBinding,
    pub model_transfer: ModelTransferState,
    pub resource_roster: Vec<ResourceRecord>,
}
```

- [ ] **Step 1: Write RED circularity and exact-artifact tests**

Assert:

- base roster contains no manifest, sidecar, binding, generated, target, binary, or `.app` path;
- every roster entry is a regular non-symlink single-link file inside repo root;
- records are path-sorted and contain exact bytes/mode/SHA-256;
- aggregate is over canonical sorted records;
- sidecar is exact `<manifest-sha256>  harness-review-manifest.json\n`;
- binding is generated only after manifest/sidecar and includes their identities;
- binding includes candidate ID/SHA/candidate-review aggregate;
- binding exact five artifacts and limits match candidate metadata;
- binding state is `frozen`, model transfer is `blocked`, and aggregate is not revoked;
- changing any roster file makes prepare fail rather than silently regenerate.

Use the five literal artifact digests/sizes and limits from the design as independent expected values.

- [ ] **Step 2: Verify RED**

```bash
python3 -m unittest scripts.tests.test_freeze_stage1_owner_authorization -v
```

Expected: FAIL.

- [ ] **Step 3: Implement deterministic freeze**

Freeze order:

1. validate pinned anchor state;
2. validate candidate metadata and its current three-file review manifest;
3. hash exact base roster;
4. atomically write base harness manifest;
5. atomically write sidecar;
6. derive resource roster for final bundle;
7. atomically write canonical binding with `state: "frozen"`;
8. re-read all three outputs and verify identities.

The binding does not include its own digest. Native/Python compute its SHA-256 at load time and include that digest in the signed payload.

- [ ] **Step 4: Implement Rust/Python binding replay**

Both languages require exact keys, exact order, exact artifact literals, exact limits, pinned candidate identity, non-revoked review identity, and `model_transfer == "blocked"`. Rust and Python tests load the same fixture binding bytes and compare all fields.

- [ ] **Step 5: Run GREEN**

```bash
python3 -m unittest scripts.tests.test_freeze_stage1_owner_authorization -v
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::binding -- --nocapture
python3 -m unittest scripts.tests.test_stage1_owner_authorization -v
```

Expected: PASS with fixture roots; do not freeze the live repository yet.

- [ ] **Step 6: Add explicit package script without running it**

```json
"freeze:stage1-owner-authorization": "python3 scripts/freeze_stage1_owner_authorization.py"
```

This command is run only after implementation/source is stable; every later roster edit invalidates its output and independent review.

**Checkpoint:** no live manifest/binding freeze yet. Do not commit.

---

### Task 8: Validate exact owner payload semantics and signed directory identities

**Files:**
- Modify: `src-tauri/src/owner_authorization/issuer.rs`
- Modify: `scripts/stage1_owner_authorization.py`
- Modify: `scripts/tests/test_stage1_owner_authorization.py`
- Modify: `scripts/tests/test_prebuilt_runtime_stage1_candidate.py:131-205`

**Interfaces:**

Rust payload builder:

```rust
pub struct IssueContext<'a> {
    pub binding: &'a FrozenBinding,
    pub directories: &'a Stage1Directories,
    pub now_unix: i64,
    pub generation: [u8; 16],
    pub nonce: [u8; 32],
}

pub struct SignedAuthorization {
    pub envelope_bytes: Vec<u8>,
    pub nonce: [u8; 32],
    pub generation_hex: String,
    pub receipt_name: String,
}

pub fn build_and_sign_authorization(
    context: IssueContext<'_>,
    key_backend: &impl OwnerKeyBackend,
    anchor: &PublicKeyAnchor,
) -> Result<SignedAuthorization, OwnerAuthorizationError>;
```

Python:

```python
@dataclasses.dataclass(frozen=True)
class VerifiedOwnerAuthorization:
    payload: dict
    envelope_sha256: str
    generation: str
    receipt_name: str
    nonce_sha256: str


def verify_owner_authorization(
    envelope_bytes: bytes,
    nonce: bytes,
    anchor_bytes: bytes,
    binding_bytes: bytes,
    directory_stats: Mapping[str, os.stat_result],
    now_unix: int,
    verifier: SignatureVerifier,
) -> VerifiedOwnerAuthorization:
    ...
```

- [ ] **Step 1: Write RED Rust payload tests**

Use fixed generation/nonce/time/directory identities and a fake signer. Assert exact payload values:

```text
generation = 00112233445566778899aabbccddeeff
issued_at_unix = 1795000000
expires_at_unix = 1795000300
receipt.name = stage1-00112233445566778899aabbccddeeff.json
model_transfer = blocked
```

Assert artifact order and all literal values match the spec. Assert the message given to the fake signer is exactly `SIGNING_DOMAIN || canonical_payload`.

- [ ] **Step 2: Verify RED**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::issuer::tests::builds_exact_stage1_payload -- --exact --nocapture
```

Expected: FAIL.

- [ ] **Step 3: Implement minimal Rust builder**

Use `getrandom` only at the caller boundary; the pure builder receives fixed bytes for testability. Compare the public key returned by every sign operation with the pinned anchor before accepting the signature. Build the canonical envelope only after successful comparison.

- [ ] **Step 4: Write RED Python semantic mutation matrix**

Start from a valid fake-signed fixture and mutate each independently:

- issue time in future;
- expiry not exactly +300;
- expired at `now == expires_at_unix`;
- wrong generation length/case;
- nonce length/hash mismatch;
- wrong binding digest;
- candidate ID/SHA/review aggregate mismatch;
- revoked harness aggregate;
- harness manifest digest mismatch;
- reordered/missing/extra/changed artifact;
- wrong per-layer installed/extraction limit;
- wrong global count/total/cap/free-space/entry/extraction limit;
- wrong ledger/workspace/receipt device/inode/UID;
- wrong receipt name;
- `model_transfer` not `blocked`;
- extra payload key.

For every case, patch mutation/network functions and assert zero effects.

- [ ] **Step 5: Verify RED**

```bash
python3 -m unittest scripts.tests.test_stage1_owner_authorization.OwnerAuthorizationSemanticTests -v
```

Expected: FAIL.

- [ ] **Step 6: Implement Python semantic verifier**

Validate signature first, then canonical/semantic fields, then `fstat` identities. Return `VerifiedOwnerAuthorization`; never return raw signature or public key.

- [ ] **Step 7: Replace the old caller-mintable `AuthorizationGate` input contract in tests**

Update candidate tests so no test can construct authority by supplying a record plus expected digest. Tests obtain `VerifiedOwnerAuthorization` only through the verifier with an injected accepting signature verifier. Add this hard regression:

```python
self.assertFalse(hasattr(stage1, "AuthorizationGate"))
self.assertFalse(hasattr(stage1, "build_parser"))
for option in ("--authorization", "--authorization-sha256", "--authorization-ledger", "--receipt"):
    with self.subTest(option=option), mock.patch.object(sys, "argv", ["prebuilt_runtime_stage1.py", option, "attacker"]):
        self.assertEqual(stage1.main(), 1)
```

Each invocation must return the fixed redacted failure before opening any caller path; patch `os.open` with an assertion sentinel for these cases.

- [ ] **Step 8: Run GREEN**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::issuer -- --nocapture
python3 -m unittest scripts.tests.test_stage1_owner_authorization.OwnerAuthorizationSemanticTests -v
python3 -m unittest scripts.tests.test_prebuilt_runtime_stage1_candidate -v
```

Expected: PASS.

**Checkpoint:** no transfer can be authorized from caller-created JSON. Do not commit.

---

### Task 9: Refactor stage-1 consumption to fixed FDs, one generation, and app-owned receipt publication

**Files:**
- Modify: `scripts/prebuilt_runtime_stage1.py:32-319,1242-1704`
- Modify: `scripts/tests/test_prebuilt_runtime_stage1_candidate.py`
- Modify: `scripts/tests/test_prebuilt_runtime_stage1_transport.py`
- Modify: `scripts/tests/test_prebuilt_runtime_stage1_extract.py`
- Modify: `scripts/tests/test_prebuilt_runtime_stage1_runner.py`

**Interfaces:**

Replace caller-selected launch inputs with:

```python
AUTHORIZATION_FD = 3
NONCE_FD = 4
LEDGER_FD = 5
WORKSPACE_PARENT_FD = 6
RECEIPT_PARENT_FD = 7

@dataclasses.dataclass(frozen=True)
class Stage1Resources:
    root: Path
    candidate: Path
    candidate_review_manifest: Path
    harness_review_manifest: Path
    harness_review_sidecar: Path
    binding: Path
    trust_anchor: Path


def run_stage1(
    resources: Stage1Resources,
    authorization: VerifiedOwnerAuthorization,
    ledger_fd: int,
    workspace_parent_fd: int,
    receipt_parent_fd: int,
    *,
    connection_factory=None,
    run_id_factory=lambda: str(uuid.uuid4()),
) -> str:
    ...
```

The production `main()` resolves resources relative to `__file__`, reads only fixed FDs 3-7, and accepts no command-line options.

- [ ] **Step 1: Write RED fixed-FD and no-path CLI tests**

Assert direct execution with missing fixed FDs returns `PREBUILT_STAGE1_FAILED`; arbitrary CLI arguments are rejected; no path option exists. Create temporary inherited FDs in tests and prove the entry point reads from those descriptors rather than `HOME`, cwd, or argv.

- [ ] **Step 2: Verify RED**

```bash
python3 -m unittest scripts.tests.test_prebuilt_runtime_stage1_runner.Stage1RunnerTests.test_cli_uses_only_fixed_inherited_descriptors -v
```

Expected: FAIL under the old path-based CLI.

- [ ] **Step 3: Replace authorization marker identity**

Use deterministic names based on the signed generation:

```python
def generation_marker_name(generation: str) -> str:
    return ".jarvis-stage1-generation-" + generation + ".json"
```

Marker schema becomes `jarvis.local-provider.stage1-owner-ledger.v1` and includes exact keys:

```text
authorization_sha256
binding_sha256
candidate_id
candidate_sha256
cleanup_sha256
generation
receipt_destination
receipt_sha256
review_aggregate_sha256
review_manifest_sha256
run_id
schema
state
workspace
```

Create with `O_EXCL` after validation and before network. A present marker always prevents replay, regardless of state.

- [ ] **Step 4: Make workspace/receipt descriptor-relative**

`PrivateWorkspace.create_at(parent_fd, generation, required_bytes)` creates only `jarvis-prebuilt-<generation>` under the inherited workspace parent. Receipt publication uses inherited receipt parent fd plus signed filename. Remove `_canonical_workspace_parent()` and every production path argument.

- [ ] **Step 5: Write RED one-time and crash tests**

Cover:

- concurrent process exclusion on the inherited ledger descriptor;
- same envelope/generation replay in a fresh process rejected before network;
- a new valid generation succeeds after old crash recovery;
- registered abrupt-exit workspace is removed by exact descriptor identities;
- unregistered lookalike generation directory is preserved;
- changed workspace inode or receipt parent identity fails;
- cleaned/published receipt recovery performs no workspace creation or network;
- wrong receipt filename/digest/content/hardlink/symlink fails without effects.

- [ ] **Step 6: Verify RED**

```bash
python3 -m unittest \
  scripts.tests.test_prebuilt_runtime_stage1_candidate \
  scripts.tests.test_prebuilt_runtime_stage1_runner -v
```

Expected: FAIL until the generation ledger and fd APIs exist.

- [ ] **Step 7: Implement minimal GREEN while preserving transport/extraction contracts**

Do not alter the exact GHCR origins, one redirect, content-length enforcement, adjacent disk preflight, five transfers, extraction limits, Mach-O/license inspection, or cleanup counts except where deterministic workspace/receipt naming changes expected tests.

Update receipt authorization to exactly:

```text
authorization_sha256
binding_sha256
generation
key_id
nonce_sha256
purpose
```

Do not include envelope, signature, public key, paths, timestamps, or raw authorization payload.

- [ ] **Step 8: Run focused GREEN**

```bash
npm run test:prebuilt-stage1
```

Expected: all stage-1 candidate, transport, extraction, and runner tests pass with no network access because transport tests use injected fakes.

- [ ] **Step 9: Run effect-order regression**

Add a test where signature verification, expiry, binding, each directory identity, and nonce validation fail separately while `create_stage1_workspace`, ledger marker creation, and connection factory are assertion-raising sentinels. Every case must return before those sentinels.

- [ ] **Step 10: Re-run full stage-1 suite**

```bash
npm run test:prebuilt-stage1
```

Expected: PASS.

**Checkpoint:** caller-selectable ledger/workspace/receipt paths are gone. Do not commit.

---

### Task 10: Launch the exact bundled Python harness with fixed inherited descriptors

**Files:**
- Create: `src-tauri/src/owner_authorization/process.rs`
- Create: `src-tauri/tests/stage1_owner_authorization.rs`
- Modify: `src-tauri/src/owner_authorization/issuer.rs`
- Modify: `src-tauri/src/owner_authorization/mod.rs`

**Interfaces:**

```rust
pub struct Stage1ChildRequest<'a> {
    pub script: &'a Path,
    pub envelope: &'a [u8],
    pub nonce: &'a [u8; 32],
    pub ledger_fd: BorrowedFd<'a>,
    pub workspace_parent_fd: BorrowedFd<'a>,
    pub receipt_parent_fd: BorrowedFd<'a>,
}

pub trait Stage1ProcessRunner {
    fn run(&self, request: Stage1ChildRequest<'_>) -> Result<String, Stage1ProcessError>;
}

pub struct NativeStage1ProcessRunner;
```

Production command is exact:

```text
/usr/bin/python3 -B -E -s <outer-sealed Contents/Resources/jarvis/stage1/prebuilt_runtime_stage1.py>
```

The child working directory is `/`. The fixed signed script directory is the only non-standard-library import root, allowing the harness to import the adjacent outer-sealed `stage1_owner_authorization.py` while `-E` ignores Python environment variables and `-s` disables the user site.

No other argument or inherited environment is permitted.

- [ ] **Step 1: Write RED process tests**

Use a fixture Python script under a temporary fake bundle that reads FDs 3-7 and emits a known 64-hex digest. Tests assert:

- envelope arrives only on fd 3;
- nonce arrives only on fd 4;
- directory identities on fds 5-7 match the parent descriptors;
- argv contains only interpreter flags and exact script path;
- environment is exactly `LANG=C`, `LC_ALL=C`, `PATH=/usr/bin:/bin` or is empty except the fixed locale, according to the implemented direct-spawn requirement;
- stdout is one newline-terminated 64-lowercase-hex digest and at most 128 bytes;
- stderr is empty on success and bounded on failure;
- timeout, extra stdout, malformed digest, late output, signal exit, or surviving child process fails and is cleaned up.

- [ ] **Step 2: Verify RED**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test stage1_owner_authorization --features runtime-helper -- --nocapture
```

Expected: FAIL because the launcher is absent.

- [ ] **Step 3: Implement direct spawn without shell**

Use `std::process::Command` with `/usr/bin/python3`. Create two pipes with `pipe2(O_CLOEXEC)` for envelope/nonce. In `pre_exec`, `dup2` read ends/directories to 3-7, clear close-on-exec only for 3-7, start a new process group, close all unrelated child ends, and set a restrictive umask. Parent writes bounded envelope/nonce, closes write ends, captures bounded output, enforces a fixed deadline, and kills/reaps the process group on every failure.

Do not pass the signing key, prompt, authentication context, public key, signature, nonce, directory paths, or bearer data in argv/environment.

- [ ] **Step 4: Run GREEN**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test stage1_owner_authorization --features runtime-helper -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Connect `issue_and_run`**

Order in `issuer.rs`:

1. resolve/verify signed bundle resources;
2. parse pinned anchor and frozen binding;
3. open canonical directories;
4. run native recovery preflight without network;
5. generate generation/nonce;
6. build/sign exact payload under user presence;
7. invoke `Stage1ProcessRunner` immediately;
8. return receipt digest only.

If spawn fails after signing, the in-memory authorization is discarded. Retrying requires a fresh user-presence signature, generation, and nonce.

- [ ] **Step 6: Add fake end-to-end native test**

Inject fake key backend, fixed clock/random source, temporary directories, fixture bundle, and fixture Python script. Assert exact signer message, fixed FD transfer, receipt digest result, and no ordinary Tauri startup.

- [ ] **Step 7: Run native regression**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --all-features -- --nocapture
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

Expected: PASS.

**Checkpoint:** no real user presence or transfer occurred. Do not commit.

---

### Task 11: Prepare and verify exact signed bundle resources without hash circularity

**Files:**
- Create: `scripts/prepare_stage1_owner_authorization_bundle.py`
- Create: `scripts/tests/test_prepare_stage1_owner_authorization_bundle.py`
- Create: `scripts/verify_stage1_owner_authorization_bundle.py`
- Modify: `src-tauri/tauri.conf.json:6-60`
- Modify: `scripts/build_native_helper.py:24-46,220-270`
- Modify: `scripts/finalize_signed_helper_bundle.py:31-158`
- Modify: `scripts/verify_signed_helper_bundle.py:84-158`
- Modify: `scripts/tests/test_build_native_helper.py`
- Modify: `scripts/tests/test_signed_helper_bundle.py`
- Modify: `package.json:9-21`

**Interfaces:**

Generated root:

```text
src-tauri/generated/stage1/
  stage1-owner-public-key.json
  stage1-owner-binding.json
  candidate-metadata.json
  candidate-metadata.json.sha256
  review-manifest.json
  review-manifest.json.sha256
  harness-review-manifest.json
  harness-review-manifest.json.sha256
  prebuilt_runtime_stage1.py
  stage1_owner_authorization.py
  review-root/<exact BASE_REVIEW_ROSTER paths>
```

Final bundle destination:

```text
Contents/Resources/jarvis/stage1/
```

- [ ] **Step 1: Write RED prepare tests**

Assert two exact modes:

- bootstrap mode: anchor `unprovisioned` and binding `unfrozen`; generated root contains only those two records plus a canonical `bootstrap-only.json` marker; issue is unavailable;
- final mode: anchor pinned and binding frozen; every generated file is a regular single-link copy with exact bytes/mode/hash; no symlink or extra file survives.

Reject stale base manifest, sidecar mismatch, revoked aggregate, changed source, wrong binding, unpinned anchor, generated destination symlink, dirty unexpected file, and copy-time source changes.

- [ ] **Step 2: Verify RED**

```bash
python3 -m unittest scripts.tests.test_prepare_stage1_owner_authorization_bundle -v
```

Expected: FAIL.

- [ ] **Step 3: Implement prepare as verification plus atomic copy**

The ordinary build command must never regenerate review identity. It only verifies tracked frozen inputs and copies them. Use a new private temporary sibling directory, fsync files/directories, then atomic rename. Bootstrap mode is explicit from tracked resource states, not an environment downgrade.

- [ ] **Step 4: Update Tauri resources and build ordering**

Add package scripts:

```json
"test:stage1-owner-authorization": "python3 -m unittest scripts.tests.test_stage1_owner_authorization scripts.tests.test_pin_stage1_owner_public_key scripts.tests.test_freeze_stage1_owner_authorization scripts.tests.test_prepare_stage1_owner_authorization_bundle -v",
"prepare:stage1-owner-authorization": "python3 scripts/prepare_stage1_owner_authorization_bundle.py",
"verify:stage1-owner-authorization": "python3 scripts/verify_stage1_owner_authorization_bundle.py"
```

Change Tauri hooks to:

```text
npm run prepare:stage1-owner-authorization && npm run build:native-helper && npm run dev
npm run prepare:stage1-owner-authorization && npm run build:native-helper && npm run build
```

Map `generated/stage1` to `jarvis/stage1` as a resource. Keep the existing helper identity resource.

- [ ] **Step 5: Close native source generation**

Extend `SOURCE_ALLOWLIST` with every new owner C/Rust/Python/script/test/resource file, `src-tauri/src/main.rs`, `src-tauri/tauri.conf.json`, and `package.json`. Include the tracked binding. Exclude generated copies and build outputs.

Update tests so changing any owner source/resource changes generation and stale generated bundle material fails before build.

- [ ] **Step 6: Write RED bundle verifier tests**

Test bootstrap and final modes with fake app bundles. Final verifier must reject:

- unprovisioned anchor or unfrozen binding;
- missing/duplicate/extra/symlinked/writable/hard-linked resource;
- source/bundle byte mismatch;
- stale helper source generation;
- invalid outer seal or nested helper signature;
- unexpected app/helper entitlements;
- unexpected Tauri command/capability/shell surface;
- ordinary-startup owner action source pattern;
- resource path outside exact `Contents/Resources/jarvis/stage1`.

- [ ] **Step 7: Implement finalizer/verifier composition**

The finalizer first requires prepared resources, then preserves the existing exact staged-helper signing flow, then signs the outer app last. `verify_signed_helper_bundle.py` calls `verify_stage1_owner_authorization_bundle.verify(..., final=True)` before running the signed-helper self-test.

The bootstrap verifier accepts only unprovisioned/unfrozen resources and proves issue mode is unavailable; it is never accepted as a final candidate.

- [ ] **Step 8: Run GREEN**

```bash
python3 -m unittest \
  scripts.tests.test_prepare_stage1_owner_authorization_bundle \
  scripts.tests.test_build_native_helper \
  scripts.tests.test_signed_helper_bundle -v
npm run test:stage1-owner-authorization
```

Expected: PASS.

- [ ] **Step 9: Run no-build static configuration checks**

```bash
python3 -m py_compile \
  scripts/stage1_owner_authorization.py \
  scripts/pin_stage1_owner_public_key.py \
  scripts/freeze_stage1_owner_authorization.py \
  scripts/prepare_stage1_owner_authorization_bundle.py \
  scripts/verify_stage1_owner_authorization_bundle.py
python3 -m json.tool src-tauri/tauri.conf.json >/dev/null
python3 -m json.tool package.json >/dev/null
```

Expected: PASS. Remove any generated `__pycache__` files before handoff if the environment writes them.

**Checkpoint:** source-generation and signing order are closed; no app build yet. Do not commit.

---

### Task 12: Add user-presence-gated key deletion and source rollback safety

**Files:**
- Modify: `src-tauri/src/owner_authorization/issuer.rs`
- Modify: `src-tauri/src/platform/owner_authorization.rs`
- Modify: `src-tauri/native/owner_authorization.c`
- Modify: `src-tauri/src/owner_authorization/paths.rs`
- Modify: Rust/Python tests as needed
- Document after implementation: `ARCHITECTURE.md`, `.rps/STATE.md`

**Interfaces:**

```rust
pub trait LedgerStateReader {
    fn summarize_for_deletion(&self, ledger_fd: BorrowedFd<'_>) -> Result<LedgerDeletionState, OwnerAuthorizationError>;
}

pub enum LedgerDeletionState {
    Empty { summary_sha256: [u8; 32] },
    Quiescent { summary_sha256: [u8; 32], completed_receipts: u64 },
    ActiveOrUnrecovered,
}
```

Deletion challenge is exact:

```text
JARVIS-STAGE1-OWNER-AUTHORITY-DELETE-V1\0
|| pinned-key-id-bytes
|| ledger-summary-sha256-bytes
```

- [ ] **Step 1: Write RED deletion orchestration tests**

Use fake backend/ledger:

- active child or registered unrecovered workspace returns `delete-refused` and does not call key backend;
- key public bytes must match pinned anchor before delete proof;
- user cancellation/authentication failure does not delete;
- successful proof calls exact delete once, then a subsequent public-key lookup must return key-not-found;
- completed receipts and ledger markers remain;
- only `bootstrap/owner-public-key.json` is removed after verified key absence;
- unknown files/symlinks under bootstrap/workspace cause refusal rather than broad deletion.

- [ ] **Step 2: Verify RED**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization::issuer::tests::delete -- --nocapture
```

Expected: FAIL.

- [ ] **Step 3: Implement minimal deletion flow**

Acquire the ledger lock, recover exact registered stale workspace or refuse active state, compute canonical ledger summary, compare pinned/current public key, call `delete_after_proof`, query absence, then remove only the canonical bootstrap public record. Do not delete receipts or durable ledger history.

- [ ] **Step 4: Add rollback documentation assertions**

Bundle/source verifier tests must fail if owner source/resources are removed while the source rollback checklist lacks the authenticated key-deletion step. The implementation handoff updates architecture/state only with actual results.

- [ ] **Step 5: Run GREEN**

```bash
cargo test --manifest-path src-tauri/Cargo.toml owner_authorization -- --nocapture
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

Expected: PASS without real key access because tests use fakes.

**Checkpoint:** do not execute real deletion during implementation. Do not commit.

---

### Task 13: Run full automated verification before any real key ceremony

**Files:**
- No new product files expected.
- Modify failing implementation/tests only through fresh RED → GREEN cycles.

**Interfaces:** all prior tasks.

- [ ] **Step 1: Run Python owner and stage-1 suites**

```bash
npm run test:stage1-owner-authorization
npm run test:prebuilt-stage1
python3 -m unittest scripts.tests.test_build_native_helper scripts.tests.test_signed_helper_bundle -v
```

Expected: all pass; no external network, Keychain, Secure Enclave, or user-presence prompt.

- [ ] **Step 2: Run Rust verification**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --all-features
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

Expected: all pass.

- [ ] **Step 3: Run frontend/browser gates unchanged**

```bash
npm run check
npx playwright test
```

Expected: existing frontend and browser tests pass with no UI changes.

- [ ] **Step 4: Prove no runtime/model/network action occurred**

Run repository residue checks using the existing verification patterns. Require:

- no new `.gguf`, bottle layer, runtime staging root, model directory, or transfer receipt;
- no running `jarvis-runtime-helper`, Python stage-1 child, or model process;
- no changed `src/` UI file;
- no Tauri invoke handler, shell plugin, capability entry, or owner JavaScript surface;
- tracked anchor still unprovisioned unless the owner separately approved the ceremony.

- [ ] **Step 5: Inspect exact diff**

```bash
git diff --check
git status --short
```

Expected: only intended implementation/docs/state files; no generated build output, credentials, key material, or unrelated edits. Do not commit.

**Checkpoint:** automated implementation is verified, but no platform continuity claim exists yet.

---

### Task 14: Execute the two-phase local Apple Development continuity proof

**Prerequisite authority:** This task requires fresh explicit approval to build signed candidates, invoke Security.framework/Keychain/Secure Enclave, and satisfy user-presence prompts. Stop before this task without that approval.

**Files:**
- External state: app-owned Secure Enclave/Keychain item.
- External state: app-owned Application Support bootstrap public record.
- Modify through approved pinning: `src-tauri/resources/stage1-owner-public-key.json`.
- Generated outputs: `src-tauri/target`, `src-tauri/binaries`, and `src-tauri/generated`.
- Preserve prohibition: no transfer payload or model file.

**Interfaces:** exact CLI modes from Task 4 and pinning/build scripts from Tasks 5/11.

- [ ] **Step 1: Confirm signing identity is supplied externally**

```bash
test -n "$APPLE_SIGNING_IDENTITY"
```

Expected: exit 0. Do not print the identity, certificate hash, team ID, password, or key material into evidence.

- [ ] **Step 2: Verify bootstrap source state**

```bash
python3 -m json.tool src-tauri/resources/stage1-owner-public-key.json
python3 -m json.tool src-tauri/resources/stage1-owner-binding.json
```

Expected exact states: `unprovisioned` and `unfrozen`.

- [ ] **Step 3: Build and finalize provisional signed app**

```bash
npm run prepare:stage1-owner-authorization
npm run build:native-helper
npm run tauri -- build --bundles app
APPLE_SIGNING_IDENTITY="$APPLE_SIGNING_IDENTITY" npm run finalize:signed-helper
python3 scripts/verify_stage1_owner_authorization_bundle.py --bootstrap
```

Expected: strict signed-helper verification and bootstrap owner-bundle verification pass; issue mode remains unavailable.

- [ ] **Step 4: Record provisional designated requirement without secrets**

```bash
/usr/bin/codesign --verify --deep --strict src-tauri/target/release/bundle/macos/J.A.R.V.I.S.app
rm -rf artifacts/stage1-owner-provisional
mkdir -p artifacts/stage1-owner-provisional
/usr/bin/ditto src-tauri/target/release/bundle/macos/J.A.R.V.I.S.app artifacts/stage1-owner-provisional/J.A.R.V.I.S.app
/usr/bin/codesign --verify --deep --strict artifacts/stage1-owner-provisional/J.A.R.V.I.S.app
/usr/bin/codesign -d -r- artifacts/stage1-owner-provisional/J.A.R.V.I.S.app 2>artifacts/stage1-owner-provisional.designated-requirement.txt
```

Expected: strict verification succeeds for the build output and preserved copy; requirement identifies bundle `dev.rohtolos.jarvis` and the externally supplied Apple Development identity. The evidence file contains no private key or credential. The scoped removal touches only the ignored provisional evidence directory created by this task.

- [ ] **Step 5: Provision once under user presence**

```bash
src-tauri/target/release/bundle/macos/J.A.R.V.I.S.app/Contents/MacOS/jarvis-dashboard --stage1-owner-key-provision
```

Expected after Touch ID/device-password approval:

```text
JARVIS_STAGE1_OWNER_KEY_PROVISIONED
```

On cancellation/unavailability: fixed error, no fallback, STOP.

- [ ] **Step 6: Pin only the public key**

```bash
npm run pin:stage1-owner-public-key
python3 -m json.tool src-tauri/resources/stage1-owner-public-key.json
```

Expected: state `pinned`, exact algorithm, 64-hex key ID, canonical 65-byte X9.63 public key. No private or signature field.

- [ ] **Step 7: Freeze base manifest and derived binding**

Only after all roster source is final:

```bash
npm run freeze:stage1-owner-authorization
npm run prepare:stage1-owner-authorization
```

Expected: fresh non-revoked manifest/sidecar and frozen binding; prepare passes with no source mismatch.

- [ ] **Step 8: Re-run all automated gates after pin/freeze**

```bash
npm run test:stage1-owner-authorization
npm run test:prebuilt-stage1
python3 -m unittest scripts.tests.test_build_native_helper scripts.tests.test_signed_helper_bundle -v
cargo test --manifest-path src-tauri/Cargo.toml --all-features
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm run check
npx playwright test
```

Expected: all pass. Any roster-source fix invalidates the freeze; return to Step 7.

- [ ] **Step 9: Build/finalize final signed candidate from pinned/frozen source**

```bash
rm -rf src-tauri/target/release/bundle/macos/J.A.R.V.I.S.app
npm run prepare:stage1-owner-authorization
npm run build:native-helper
npm run tauri -- build --bundles app
APPLE_SIGNING_IDENTITY="$APPLE_SIGNING_IDENTITY" npm run finalize:signed-helper
npm run verify:signed-helper
npm run verify:stage1-owner-authorization
```

Expected: final bundle verification passes. This deletes only a build output, not user data; do not run without the build approval that gates this task.

- [ ] **Step 10: Prove designated-requirement compatibility both ways**

Save the final requirement, compare exact text, and require each app to satisfy the other's requirement using the bundle verification script:

```bash
python3 scripts/verify_stage1_owner_authorization_bundle.py \
  --compare-designated-requirement artifacts/stage1-owner-provisional.designated-requirement.txt \
  --provisional-app artifacts/stage1-owner-provisional/J.A.R.V.I.S.app
```

The command must:

- extract final designated requirement;
- require byte-equal normalized requirement text;
- run `codesign --verify --strict -R=<provisional requirement> final-app`;
- run the reciprocal check against a preserved provisional app copy;
- emit only `JARVIS_STAGE1_DESIGNATED_REQUIREMENT_OK`.

If any check fails: **STOP AND REDESIGN**. Do not continue.

- [ ] **Step 11: Prove final app retrieves the same key**

```bash
src-tauri/target/release/bundle/macos/J.A.R.V.I.S.app/Contents/MacOS/jarvis-dashboard --stage1-owner-key-prove
```

Expected after user presence:

```text
JARVIS_STAGE1_OWNER_KEY_CONTINUITY_OK
```

A new key, key mismatch, missing key, canceled prompt, or fallback attempt is a STOP.

- [ ] **Step 12: Clean rebuild exact final source and prove again**

Remove only generated build outputs, rebuild/finalize with the same identity, repeat designated-requirement verification, then run `--stage1-owner-key-prove` again. The same pinned key ID must be observed internally and only the fixed success token may be public.

- [ ] **Step 13: Record continuity evidence**

Record only:

- provisional/final/clean-final app bundle SHA-256 values;
- normalized designated-requirement SHA-256;
- pinned public key ID;
- fixed proof success tokens;
- exact command exit statuses;
- statement that no transfer was attempted.

Do not retain prompt screenshots, user identity, certificate common name, raw public key unless already present in reviewed source, signature bytes, OS error text, or local paths.

**STOP condition:** If retrieval does not survive both rebuild transitions with the same designated requirement, stop the entire plan. Delete the key only through the authenticated provisional/final deletion path if safe, preserve evidence of failure, and design a different keychain ownership model before further work.

**Checkpoint:** continuity is proven; transfer remains blocked. Do not commit.

---

### Task 15: Independent exact-candidate review and final no-transfer certification

**Prerequisite:** Task 14 passed. This task still does not authorize stage-1 transfer.

**Files:**
- Update after real results: `ARCHITECTURE.md`
- Update after real results: `.rps/STATE.md`
- Create review evidence under the repository's existing spike/evidence convention only if separately authorized by the parent workflow.
- Do not modify candidate metadata unless a review finding requires a new candidate, in which case restart freeze/review.

**Interfaces:** exact frozen source, manifest, binding, generated resources, signed app, continuity evidence.

- [ ] **Step 1: Freeze exact final hashes**

Programmatically record hashes/modes/sizes for:

- base harness manifest and sidecar;
- derived binding;
- pinned trust anchor;
- generated stage-1 resource tree aggregate;
- main app executable;
- nested helper;
- outer app bundle verification result;
- provisional/final/clean-final designated-requirement digest;
- exact source-generation identifier.

Do not add the derived binding to the base manifest retroactively; that would reintroduce circularity.

- [ ] **Step 2: Obtain two independent reviews**

Each reviewer must receive the same exact candidate hashes and answer at minimum:

- Can an untrusted CLI caller mint, replace, replay, extend, or redirect authority?
- Is every private-key operation Secure Enclave-only and user-presence-gated?
- Is there any software, OpenSSL, shell, `security` CLI, or alternate-key fallback?
- Does Python verify the pinned key signature before mutation/network?
- Are candidate/review/binding/five artifacts/limits/directories/receipt/nonce/expiry/generation all signed and replayed exactly?
- Can crash recovery delete only registered app-owned residue?
- Can receipt recovery use network or change destination?
- Can ordinary startup query the key, create directories, launch Python/helper, or use network?
- Does deletion require proof and preserve completed ledger/receipts?
- Does any path enable model transfer?
- Did the same-key retrieval survive provisional → final → clean-final rebuilds with the same designated requirement?

A reviewer must identify source/app hashes in the report. Review of a prior or rebuilt candidate does not transfer.

- [ ] **Step 3: Resolve findings with TDD**

For each verified P0-P2 finding:

1. add a focused RED regression;
2. observe expected failure;
3. implement minimal fix;
4. run focused GREEN;
5. rerun all automated gates;
6. refreeze manifest/binding/resources;
7. rebuild/refinalize;
8. repeat continuity proof when source/resource/signing identity relevance changes;
9. restart both independent reviews on the new exact candidate.

- [ ] **Step 4: Run final complete verification**

```bash
npm run test:stage1-owner-authorization
npm run test:prebuilt-stage1
python3 -m unittest scripts.tests.test_build_native_helper scripts.tests.test_signed_helper_bundle -v
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --all-features
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm run check
npx playwright test
npm run verify:signed-helper
npm run verify:stage1-owner-authorization
```

Also run the signed ordinary app and prove:

- no user-presence prompt;
- no Python stage-1 process;
- no helper process;
- no new Application Support file;
- no network request;
- no model/runtime payload.

- [ ] **Step 5: Update architecture and state factually**

Record implementation, automated verification, signed-bundle verification, continuity proof, review status, key-provisioned state, and remaining distribution limitations separately. Keep next safe action as owner approval for one exact stage-1 bottle transfer only. State explicitly that model transfer remains blocked.

- [ ] **Step 6: Inspect final workspace and do not commit**

```bash
git diff --check
git status --short
```

Expected: reviewed source/docs plus intentionally ignored generated/build outputs; no secret material, payloads, receipts, model files, or unrelated edits. No commit or push.

**Final transfer gate:** only after both independent reviews pass with no unresolved P0-P2 may the owner separately approve running:

```text
J.A.R.V.I.S.app/Contents/MacOS/jarvis-dashboard --stage1-owner-authorization-issue
```

That future command is not part of this implementation plan's execution. It would require user presence, mint a fresh 300-second generation, run only the five-layer stage-1 flow, publish the canonical receipt, and leave `MODEL_TRANSFER_BLOCKED`.

---

## Plan self-review checklist

- [ ] Every spec requirement maps to at least one task.
- [ ] No task adds a webview, UI, ordinary-startup action, software key, shell credential path, or model transfer.
- [ ] The public-key bootstrap and manifest/binding circularities are both resolved explicitly.
- [ ] Same-key retrieval after two same-designated-requirement rebuild transitions is a hard STOP gate.
- [ ] Provisioning, issuance, validation, one-time consumption, crash recovery, receipt recovery, deletion, and rollback each have RED → GREEN coverage.
- [ ] Canonical app-owned ledger/workspace/receipt identities are signed and checked by inherited descriptors.
- [ ] The exact five artifacts and every current limit are present in independent literal tests.
- [ ] Source-generation and outer-signing effects include all owner-authority inputs.
- [ ] Bundle preparation verifies frozen inputs and never silently refreezes them.
- [ ] No commit step exists.
- [ ] Independent exact-candidate review precedes any transfer.
- [ ] Model transfer remains blocked.

## Execution handoff

Plan saved to `docs/superpowers/plans/2026-09-20-stage1-native-owner-authorization.md`.

Implementation must start only after separate authorization. Use `superpowers:subagent-driven-development` or `superpowers:executing-plans`, follow each RED → GREEN task in order, stop before Task 14 without explicit key/signing approval, and never execute the future transfer command as part of this plan.
