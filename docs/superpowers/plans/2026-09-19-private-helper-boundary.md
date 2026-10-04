# Private Helper Boundary Implementation Plan

Status: completed and independently accepted for the non-shipping harness. Review-driven implementation details supersede the original sketches below; the canonical outcome is [`../../spikes/002-private-helper-boundary/README.md`](../../spikes/002-private-helper-boundary/README.md) and the final contract is the sibling design document.

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Completed steps use checked boxes. Do not commit, push, deploy, install software, download a model, or change the UI.

**Goal:** Build and adversarially verify a reusable private native-helper boundary without exposing it to the webview or shipping a helper/model.

**Architecture:** A focused Rust library owns strict framed JSON, immutable executable manifests, process-group lifecycle, macOS deny-network launch, and redacted manifest-bound evidence. A deterministic feature-gated fixture exercises the boundary in integration tests; it is absent from release bundles and no Tauri command is registered.

**Tech Stack:** Rust 2021, Tauri 2, `serde`, `serde_json`, `sha2`, `thiserror`, `libc`, standard-library process/file APIs, Cargo unit/integration tests.

**Spec:** `docs/superpowers/specs/2026-09-19-private-helper-boundary-design.md`

## Global Constraints

- Protocol version: `1`.
- Frame payload limit: `65_536` bytes.
- Request IDs: non-zero `u64`.
- Cancel acknowledgement budget: `250 ms`.
- Graceful shutdown budget: `2 s`.
- No shell strings, arbitrary executable path, inherited ambient environment, raw output evidence, Tauri command, model, UI change, or bundled fixture.
- The macOS experiment profile denies `network*`; it is not represented as the final production sandbox.
- Evidence thresholds live in verifier code, never evidence input.
- No commits are authorized for this repository.

---

### Task 1: Strict framed protocol

**Files:**
- Create: `src-tauri/src/lib.rs`
- Create: `src-tauri/src/helper/mod.rs`
- Create: `src-tauri/src/helper/protocol.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Produces: `PROTOCOL_VERSION`, `MAX_FRAME_BYTES`, `RequestEnvelope`, `Request`, `ResponseEnvelope`, `Response`, `ProtocolError`, `read_frame<R: Read>()`, and `write_frame<W: Write>()`.
- Consumers: fixture, process owner, and evidence tests.

- [x] **Step 1: Add protocol dependencies and library entry point**

Add exact dependencies:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
thiserror = "2"
libc = "0.2"
```

`src/lib.rs` exports only `pub mod helper;`; `main.rs` remains command-free.

- [x] **Step 2: Write RED protocol tests**

Tests must cover:

```rust
#[test]
fn round_trips_hello_request() { /* exact equality */ }
#[test]
fn rejects_zero_length_frame() { /* ProtocolError::InvalidLength(0) */ }
#[test]
fn rejects_oversized_frame_before_allocation() { /* 65_537 */ }
#[test]
fn rejects_truncated_body() { /* UnexpectedEof */ }
#[test]
fn rejects_invalid_utf8_and_json() { /* bounded parse errors */ }
#[test]
fn rejects_unknown_fields_and_protocol_version() { /* deny_unknown_fields */ }
#[test]
fn rejects_zero_request_id() { /* validation error */ }
#[test]
fn rejects_non_monotonic_chunks() { /* sequence ownership */ }
```

- [x] **Step 3: Run RED**

Run: `cargo test --lib helper::protocol`
Expected: compilation/test failure because protocol types are not implemented.

- [x] **Step 4: Implement minimal strict protocol**

Use four-byte big-endian length and `serde_json::from_slice`. Validate the envelope after deserialization. Never deserialize before checking the advertised length.

Required request variants:

```rust
pub enum Request {
    Hello { manifest_sha256: String },
    Probe { probe: ProbeKind },
    Cancel { target_request_id: u64 },
    Shutdown,
}
```

Required response variants:

```rust
pub enum Response {
    Ready { helper_build_id: String },
    Started,
    Chunk { sequence: u32, text: String },
    Cancelled,
    Complete { output_bytes: u64, output_sha256: String },
    Error { code: HelperErrorCode },
}
```

All objects use `#[serde(deny_unknown_fields)]` and snake-case tagged variants. `Chunk.text` is bounded to `4096` bytes during validation and never interpreted as an action.

- [x] **Step 5: Run GREEN and formatting**

Run: `cargo fmt --check && cargo test --lib helper::protocol`
Expected: all protocol tests pass.

---

### Task 2: Immutable manifest and redacted evidence

**Files:**
- Create: `src-tauri/src/helper/manifest.rs`
- Create: `src-tauri/src/helper/evidence.rs`
- Modify: `src-tauri/src/helper/mod.rs`

**Interfaces:**
- Produces: `CandidateManifestV1`, `FrozenCandidate`, `freeze_candidate()`, `EvidenceContext`, `RawProbeCounters`, `EvidenceVerdict`, `publish_evidence()`, and `verify_evidence_dir()`.
- Consumes: protocol constants and a caller-owned private root.

- [x] **Step 1: Write RED manifest tests**

Cover exact regular-file size/hash verification and rejection of absolute paths, `..`, symlinks, non-files, wrong hash/size, unsupported protocol/schema/architecture, zero budgets, and executable/evidence path aliases.

- [x] **Step 2: Write RED evidence tests**

Cover:

```rust
#[test]
fn verifier_recomputes_verdict_and_ignores_no_passed_field() {}
#[test]
fn rejects_mixed_run_manifest_or_executable_hashes() {}
#[test]
fn rejects_nan_infinity_negative_or_missing_counters() {}
#[test]
fn rejects_unexpected_oversized_or_symlinked_files() {}
#[test]
fn atomic_writer_does_not_follow_predictable_symlink() {}
#[test]
fn evidence_contains_no_output_or_local_path_fields() {}
```

Use an allowlisted schema; do not recursively trust arbitrary JSON keys.

- [x] **Step 3: Run RED**

Run: `cargo test --lib helper::`
Expected: compilation/test failure because modules are absent.

- [x] **Step 4: Implement canonical identity and strict validation**

`freeze_candidate()` canonicalizes the private root and relative executable, checks `symlink_metadata`, streams SHA-256, validates fixed schema/protocol/platform/architecture, then hashes canonical JSON with recursively sorted keys. It returns paths only to the launcher; evidence receives hashes and IDs only.

- [x] **Step 5: Implement safe evidence publication**

Use `OpenOptions::create_new(true).mode(0o600)` with a cryptographically unpredictable filename generated from OS randomness already available through a unique create loop. Write canonical JSON with `allow_nan` impossible by Rust types, flush, `sync_all`, atomically rename in the same directory, then `fsync` the parent. Reject existing symlink destinations and any alias with candidate artifacts.

Expected evidence files are exactly:

```text
manifest.json
probe-counters.json
verdict.json
inventory.json
```

The verifier has hard-coded fixture gates: correct handshake, one completed probe, network attempt denied, cancellation within 250 ms, no late frames, no surviving process, and verified temporary-root removal receipt. It opens only exact regular files, each at most `65_536` bytes, and recomputes inventory hashes and verdict.

- [x] **Step 6: Run GREEN**

Run: `cargo fmt --check && cargo test --lib helper::`
Expected: all manifest/evidence tests pass.

---

### Task 3: Feature-gated fixture and owned process generation

**Files:**
- Create: `src-tauri/src/helper/process.rs`
- Create: `src-tauri/src/helper/sandbox.rs`
- Create: `src-tauri/src/bin/jarvis-helper-fixture.rs`
- Modify: `src-tauri/src/helper/mod.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Produces: `HelperGeneration`, `LaunchPolicy`, `MacOsDenyNetwork`, `ProcessError`, and fixture modes.
- Consumes: `FrozenCandidate` and framed protocol.

- [x] **Step 1: Feature-gate the fixture**

Add:

```toml
[features]
default = []
fixture-helper = []

[[bin]]
name = "jarvis-helper-fixture"
path = "src/bin/jarvis-helper-fixture.rs"
required-features = ["fixture-helper"]
```

Do not add it to Tauri bundle resources or sidecars.

- [x] **Step 2: Write RED lifecycle unit tests with an injectable runner**

Cover fixed argv, empty environment, piped stdio, dedicated process group, bounded stderr digest/count, ready timeout, response ownership, cancel acknowledgement, TERM/KILL escalation, and Drop cleanup.

- [x] **Step 3: Run RED**

Run: `cargo test --lib helper::`
Expected: compilation/test failure because lifecycle modules are absent.

- [x] **Step 4: Implement the macOS experiment launcher**

Execute only fixed `/usr/bin/sandbox-exec` with static profile:

```scheme
(version 1)
(allow default)
(deny network*)
```

Pass the canonical validated fixture path as a structured argument. Fail if `sandbox-exec` is missing, not a regular root-owned executable, or cannot start. Clear the environment and set only `PATH=/usr/bin:/bin`, `LANG=C`, and owner-created `HOME`/`TMPDIR`.

- [x] **Step 5: Implement process-group ownership**

On Unix call `CommandExt::process_group(0)`. Store the generation ID, child PID/process-group ID, stdin/stdout workers, and bounded stderr counter/digest worker. Use `libc::killpg` only inside one documented wrapper. Never expose a spawn method to Tauri.

A fixed `/bin/ps -axo pid=,ppid=,pgid=,rss=` sampler validates and aggregates the complete process group. Missing command, malformed output, or ownership ambiguity returns an error.

- [x] **Step 6: Implement fixture modes**

Fixture mode is fixed at compile/test invocation and supports `normal`, `cancel-delay`, `cancel-ignore`, `malformed`, `crash`, `stderr-flood`, `network-attempt`, and `descendant`. It never reads files other than protocol stdin and never accepts a path or command from a frame.

- [x] **Step 7: Run GREEN**

Run: `cargo fmt --check && cargo test --lib helper::`
Expected: all lifecycle unit tests pass.

---

### Task 4: Adversarial end-to-end boundary tests

**Files:**
- Create: `src-tauri/tests/helper_boundary.rs`
- Modify: `src-tauri/Cargo.toml` only if a test-only dependency is proven necessary.

**Interfaces:**
- Consumes all Task 1–3 interfaces and the feature-gated fixture.
- Produces executable evidence that the boundary fails closed.

- [x] **Step 1: Write RED integration tests**

Tests must launch the real feature-gated fixture and cover:

```rust
#[test]
fn normal_probe_is_manifest_bound_and_redacted() {}
#[test]
fn kernel_denies_fixture_network_attempt() {}
#[test]
fn cancel_is_acknowledged_within_budget_without_late_frame() {}
#[test]
fn ignored_cancel_kills_and_reaps_entire_process_group() {}
#[test]
fn descendant_cannot_survive_owner_shutdown() {}
#[test]
fn malformed_crash_timeout_and_stderr_flood_are_bounded() {}
#[test]
fn published_evidence_replays_and_temp_root_is_removed() {}
```

- [x] **Step 2: Run RED against the fixture target**

Run: `cargo test --features fixture-helper --test helper_boundary`
Expected: failures expose missing integration behavior, not skipped tests.

- [x] **Step 3: Implement only the missing integration behavior**

Keep test orchestration outside product startup. Do not register a Tauri command or add a frontend import.

- [x] **Step 4: Run GREEN repeatedly**

Run twice:

```bash
cargo test --features fixture-helper --test helper_boundary
cargo test --features fixture-helper --test helper_boundary
```

Expected: both runs pass; no fixture process or temporary root remains after either run.

- [x] **Step 5: Run full Rust gates**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Expected: all pass with no warnings.

---

### Task 5: Prove release containment and product non-regression

**Files:**
- Modify: `.rps/STATE.md`
- Modify: `.rps/MEMORY.md` only for durable lessons discovered during implementation.
- Modify: `docs/superpowers/specs/2026-09-19-private-helper-boundary-design.md` only if verified implementation constraints differ.

**Interfaces:**
- Consumes the built/tested boundary.
- Produces a fresh release artifact and containment receipts.

- [x] **Step 1: Prove no webview/native authority was added**

Inspect `src-tauri/src/main.rs` and assert no `invoke_handler`, sidecar, resource, or fixture reference exists. Search frontend source for helper imports and expect zero.

- [x] **Step 2: Run frontend and browser gates**

Run:

```bash
npm run check
npx playwright test
```

Expected: existing unit/type/build checks and all Chromium/WebKit tests pass.

- [x] **Step 3: Build the release app without fixture feature**

Run: `npm run tauri -- build --bundles app`
Expected: release app builds and contains no `jarvis-helper-fixture` filename or fixture hash.

- [x] **Step 4: Verify signature and launch**

Run `codesign --verify --deep --strict --verbose=2` on the app, launch a fresh instance, and read back its PID. Do not claim notarization.

- [x] **Step 5: Verify residue**

Assert no test fixture process, private temporary root, model file, or evidence payload remains outside the project’s bounded test receipts.

---

### Task 6: Independent review and checkpoint

**Files:**
- Modify only files required by verified review findings.
- Update: `.rps/STATE.md`

**Interfaces:**
- Consumes the exact candidate implementation and test receipts.
- Produces an accept/reject decision for the boundary foundation only.

- [x] **Step 1: Dispatch two independent reviews**

One reviewer audits protocol/schema/evidence trust boundaries. One audits process/sandbox/lifecycle ownership and release containment. They must report P0–P3 findings with reproducible evidence.

- [x] **Step 2: Verify every reported finding locally**

Do not trust reviewer summaries. Reproduce valid defects with a failing test, repair by RED/GREEN, and rerun all Task 4–5 gates.

- [x] **Step 3: Record exact outcome**

Update `.rps/STATE.md` with implemented files, exact commands/results, open risks, whether any P0/P1/P2 remains, and rollback instructions. Mark the boundary accepted only if all spec acceptance criteria pass.

- [x] **Step 4: Checkpoint at the next architecture boundary**

Do not download a model or expose the helper to the app. The next slice is a separately designed model candidate/evidence run using this accepted foundation.
