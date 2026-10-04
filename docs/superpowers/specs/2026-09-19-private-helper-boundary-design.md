# Private Helper Boundary Design

Date: 2026-09-19

## Authority

The owner approved the foundation-first sequence and instructed the agent to continue without repeated stops inside that scope. This authorizes design, implementation, local builds, and tests for this boundary. It does not authorize a model download, UI redesign, credentials, commit, push, deployment, notarization, system installation, or expanded user-data access.

## Outcome

Build a reusable experimental native boundary for a future local inference helper and prove it with a deterministic fixture. The slice must demonstrate strict framed IPC, manifest-bound candidate identity, private process ownership, cancellation, kernel network denial, bounded redacted evidence, and verified cleanup without exposing any new Tauri command or shipping a model/helper.

## Non-goals

- No AI inference or model artifact.
- No product provider switch or Settings control.
- No microphone, filesystem, credential, web, or automation capability.
- No arbitrary executable path from the webview.
- No runtime plugin loading.
- No production claim that `sandbox-exec` is the final signed-helper sandbox.

## Approaches considered

### Selected: reusable Rust boundary plus test-only fixture

Keep the protocol, manifest validation, launcher, lifecycle ownership, and evidence schema in a focused Rust library. Compile a deterministic fixture only behind an explicit Cargo feature for integration tests. The Tauri application imports no command and cannot invoke it from the webview.

This gives reusable boundary code and adversarial tests without retaining a fake assistant feature or widening authority. Production use remains blocked until a shipped signed helper removes the documented pathname-launch gap.

### Rejected: Python or shell-only harness

A disposable external harness is quicker but repeats the trust failures found in spike 001: caller-controlled thresholds, weak process accounting, path aliasing, and evidence that is not tied to the exact executable.

### Rejected: immediate model helper

A real model would add a multi-gigabyte download and complex parser/runtime attack surface before the IPC and lifecycle boundary is proven. The previous model remains rejected for integration.

## Component boundaries

### `helper_protocol`

Defines protocol version `1`, strict request/response envelopes, frame encoding, and frame decoding.

- Four-byte unsigned big-endian length prefix.
- Maximum payload: 65,536 bytes.
- UTF-8 JSON only.
- Exact-key validation on both envelopes and every nested variant, plus `serde(deny_unknown_fields)`.
- Unknown protocol versions, zero-length frames, oversized frames, malformed JSON, invalid UTF-8, and truncated bodies are rejected before dispatch.
- Request IDs are non-zero `u64` values allocated by the owner.
- The fixture supports only `hello`, `probe`, `cancel`, and `shutdown` requests.
- Responses are `ready`, `started`, `chunk`, `cancelled`, `complete`, and structured `error` messages.
- Generated text is always untrusted data; it cannot encode an action.

### `helper_manifest`

Freezes the executable identity before launch.

Required fields:

- schema and protocol version;
- run ID;
- helper build ID;
- canonical executable path relative to an owner-selected root;
- executable byte size and SHA-256;
- platform and architecture;
- fixed resource and timeout budgets;
- launcher policy ID;
- evidence schema version.

Validation rejects absolute paths, traversal, symlinks, non-regular files, size/hash mismatch, unsupported versions, non-finite metrics, caller-defined acceptance thresholds, and path aliasing.

The fixture root is owner-only and changed from mode `0700` to `0500` after freezing. The executable must be owner-controlled, executable, non-group/world-writable, and single-linked; its held descriptor, inode, size, and hash are revalidated immediately before launch. macOS `sandbox-exec` still accepts only a pathname, so this does not eliminate a malicious same-UID race between final validation and `exec`. That residual blocks production helper approval; the production design requires a signed app-owned helper and signature/designated-requirement verification.

### `helper_process`

Owns one helper generation.

- Uses structured `Command` arguments, never a shell string.
- Uses a canonical fixed executable validated against the manifest.
- Clears ambient environment variables and supplies only a minimal fixed environment.
- Pipes stdin/stdout through bounded nonblocking delivery sized for the full manifest-valid 256-chunk burst plus terminal frames, and drains stderr into a bounded byte count plus SHA-256, never retained text; excess responses close the pipe and trigger termination instead of blocking a worker join.
- Starts a dedicated Unix process group.
- Requires the helper `ready` frame before accepting work.
- Challenges and fences every helper generation with unpredictable run/generation IDs, then fences every response by request ID.
- Cancellation sends a protocol cancel request, waits at most 250 ms for `cancelled`, rejects late frames throughout the full window and shutdown, then terminates the process group on timeout.
- Shutdown allows two seconds for clean exit, then sends TERM and KILL to the whole group.
- Drop performs best-effort group termination and never detaches a child.
- An independent 10 ms monitor preserves observed peak RSS/process count and terminates the group on a ceiling or accounting failure. The default policy also applies a one-process kernel limit; process-group-only descendant handling is test-only and is not claimed as an unescapable production supervisor.

### `helper_sandbox`

The experiment launcher on macOS invokes fixed `/usr/bin/sandbox-exec` with a static default profile that denies all network operations and process forks, plus a child `RLIMIT_NPROC` of one, while allowing pipes and threads. Descendant/process-budget adversarial modes are feature-gated and use a separate test-only no-network profile that permits forks solely to probe accounting and cleanup.

This is an experiment boundary only. A future shipped helper requires its own signed bundle, entitlements, hardened runtime, and notarization review. Passing these tests does not approve `sandbox-exec` as the production sandbox.

### `helper_evidence`

Writes and verifies only allowlisted counters and hashes.

- A candidate manifest is frozen before launch and hashed canonically.
- Every evidence document includes the same schema, run ID, per-scenario generation IDs, manifest SHA-256, executable SHA-256, protocol version, and harness version.
- Callers cannot construct accepted counters: one sealed lifecycle harness derives the receipt from real helper generations and verified cleanup.
- The verifier uses hard-coded gate semantics; evidence cannot supply its own thresholds.
- JSON is parsed from one bounded immutable byte buffer per descriptor; the same bytes are hashed and decoded.
- Only exact expected filenames are opened.
- Files are opened relative to one held `O_DIRECTORY|O_NOFOLLOW` descriptor and must be owner-owned, mode `0600`, regular, bounded, and single-linked.
- Writes use descriptor-relative `O_CREAT|O_EXCL|O_NOFOLLOW`, mode `0600`, file and directory `fsync`; `inventory.json` is written last as the commit marker, so partial publication is never accepted.
- Destination aliases and non-empty publication directories are rejected before any evidence file is created.
- No prompt, completion, stderr text, environment value, local executable path, credential-shaped field, or arbitrary event payload is retained.

## Fixture behavior

The fixture binary is compiled only with Cargo feature `fixture-helper` and is never added to the Tauri bundle.

Modes are selected by a fixed enum in its test manifest, not arbitrary command text:

- normal handshake/probe/complete;
- delayed cancellation acknowledgement;
- deliberately late post-cancel output;
- ignored cancellation;
- malformed frame;
- unknown field/version;
- crash after start;
- bounded stderr flood;
- response-channel, continuous-stderr, aggregate-output/chunk, transient RSS, and persistent/transient process-count budget overflow;
- maximum valid 256-chunk burst succeeds regardless of producer/consumer timing;
- attempted process fork under the default policy;
- attempted outbound loopback connection with an unsandboxed success control and exact policy-denial errno;
- descendant process that must die with the group.

## Data flow

1. Test creates a private temporary root.
2. Test copies the fixture binary into the root and freezes its manifest.
3. Manifest validation recomputes executable size/hash and canonical manifest hash.
4. Launcher starts the fixture in a dedicated process group under the deny-network/no-fork profile and independent resource monitor.
5. Owner completes `hello`/`ready` negotiation.
6. Owner submits one bounded synthetic probe.
7. Responses are decoded, fenced, and reduced to counters/hashes.
8. Cancellation or shutdown terminates and reaps the process group.
9. A sealed lifecycle receipt is published with `inventory.json` last and replayed against the same in-memory run receipt; a new generation receipt rejects an old package.
10. RAII guards delete private temporary roots on success, error, and unwind; removal is read back.

## Failure handling

- Any framing, schema, identity, accounting, sandbox, timeout, or cleanup failure rejects the run.
- Monitor absence or parse failure is a failure, never “no network” or zero memory.
- Unexpected child exit produces a structured local error and cannot trigger an action.
- Late frames after cancellation are discarded by generation/request ownership and fail the adversarial test.
- Evidence publication failure leaves no accepted run; a partial temporary file is never treated as evidence.

## Test strategy

### Protocol unit tests

- round-trip every frame type;
- reject zero, oversized, truncated, invalid UTF-8, invalid JSON, unknown field, unknown variant, and wrong version;
- enforce non-zero request IDs and monotonic chunk sequences.

### Manifest/evidence unit tests

- reject traversal, symlink, path alias, size/hash mismatch, unsupported architecture/version, NaN/infinity, negative counters, self-declared thresholds, missing identity, unexpected files, oversized files, and credential-shaped keys;
- prove exclusive descriptor-relative publication does not follow a pre-created symlink and rejects mode/link/path swaps;
- recompute verdicts from sealed bounded observations rather than caller-supplied `passed` flags.

### Process integration tests

- successful handshake and probe;
- unsandboxed outbound-loopback control succeeds and the identical sandboxed connection fails with policy errno;
- process fork is denied by the default kernel profile;
- cancellation acknowledgement within 250 ms;
- post-cancel late output is detected and rejected;
- ignored cancellation terminates the entire process group;
- descendant cannot survive shutdown;
- malformed output, crash, response flood, continuous stderr, aggregate output/chunk overflow, transient RSS/process spikes, and timeout remain bounded;
- default-feature tests skip the fixture cleanly, while all-feature tests run it;
- temporary root deletion is verified after success and injected unwind.

### Product regression gates

- no Tauri application command is registered;
- fixture binary is absent from the release bundle;
- frontend checks, full Chromium/WebKit suite, Rust fmt/clippy/tests, native build, signature verification, and launch remain green.

## Acceptance

This slice passes only when all adversarial Rust tests pass, an independent reviewer finds no P0/P1/P2 boundary defect, the release app contains no fixture/helper, the Tauri command surface remains empty, and temporary roots/processes are absent after tests.

Passing authorizes only the experimental boundary foundation. The documented macOS pathname-launch limitation must be removed by the separately reviewed signed-helper design before any production helper or model integration. This slice does not authorize model download, model integration, user-data access, or release.

## Rollback

Remove the new Rust boundary modules, fixture feature/bin, tests, and documentation. Because no Tauri command, preference, database, model, or user data is introduced, rollback requires no migration.
