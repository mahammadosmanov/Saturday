# Signed app-owned helper implementation plan

Status: active

> Execute with TDD. Do not expose a webview command, launch the helper during ordinary startup, download a model, change the UI, commit, push, deploy, or publish.

**Goal:** bundle a dormant native helper, mutually authenticate the signed parent and child at runtime, and verify a bounded framed health lifecycle from a native-only self-test mode.

**Canonical design:** `docs/superpowers/specs/2026-09-19-signed-app-helper-design.md`

## Task 1 — macOS running-process identity bridge

Files:
- Create `src-tauri/native/code_identity.c`
- Create `src-tauri/src/platform/code_identity.rs`
- Create `src-tauri/src/platform/mod.rs`
- Modify `src-tauri/build.rs`
- Modify `src-tauri/Cargo.toml`

Steps:

- [x] Add RED Rust tests proving ad-hoc self identity is rejected and invalid/dead PIDs fail closed.
- [x] Add a C bridge using Security.framework and CoreFoundation with complete release paths.
- [x] Expose only `process_matches_current_team(pid) -> Result<bool, IdentityError>`.
- [x] Compile the bridge only on macOS; use a fail-closed unsupported-platform stub elsewhere.
- [x] Run focused tests, formatting, and warnings-denied Clippy.

## Task 2 — production helper core

Files:
- Create `src-tauri/src/signed_helper/identity.rs`
- Create `src-tauri/src/signed_helper/helper.rs`
- Create `src-tauri/src/signed_helper/mod.rs`
- Create `src-tauri/src/bin/jarvis-runtime-helper.rs`
- Modify `src-tauri/Cargo.toml`

Steps:

- [x] Add RED unit tests with an injected peer verifier for parent rejection, strict handshake, health, cancellation, shutdown, malformed frames, and EOF.
- [x] Implement a helper state machine on the existing exact framed protocol.
- [x] Embed the build-generation identifier and report it only in `Ready`.
- [x] Reject `Probe::NetworkAttempt` and every unsupported transition.
- [x] Make the binary validate its immediate parent before reading stdin.
- [x] Verify focused unit tests and direct ad-hoc execution fails closed.

## Task 3 — deterministic helper build and identity resource

Files:
- Create `scripts/build_native_helper.py`
- Create `scripts/tests/test_build_native_helper.py`
- Modify `.gitignore`
- Modify `package.json`
- Modify `src-tauri/tauri.conf.json`

Steps:

- [x] Add RED Python tests for the source allowlist, target-triple validation, symlink rejection, atomic copy, identity schema, hash mismatch, and unsupported architecture.
- [x] Build the helper with an explicit source-generation environment value.
- [x] Verify Mach-O architecture and compute binary SHA-256.
- [x] Atomically publish the target-suffixed external binary and generated identity JSON.
- [x] Ignore generated artifacts in Git.
- [x] Configure `externalBin` and map identity JSON to `Contents/Resources/jarvis/helper-identity.json`.
- [x] Prebuild the helper before Tauri dev/build without adding shell-plugin permissions.

## Task 4 — native-only supervisor and self-test

Files:
- Create `src-tauri/src/signed_helper/supervisor.rs`
- Modify `src-tauri/src/signed_helper/mod.rs`
- Modify `src-tauri/src/main.rs`
- Create `src-tauri/tests/signed_helper.rs`

Steps:

- [x] Add RED tests for strict identity JSON, hash mismatch, symlink/path escape, writable nested code, wrong generation, peer mismatch, timeouts, late output, and cleanup.
- [x] Resolve the helper and identity resource only relative to the current `.app` executable.
- [x] Verify binary hash before spawn and live child team immediately after spawn and before `Hello`.
- [x] Launch under the fixed no-network/no-fork profile with private temporary HOME/TMPDIR.
- [x] Complete handshake, health, cancellation, shutdown, bounded stderr, and process-group cleanup.
- [x] Add only `--verify-signed-helper`; ordinary startup remains unchanged.
- [x] Emit one fixed success token or redacted error code.

## Task 5 — signed bundle certification

Files:
- Create `scripts/verify_signed_helper_bundle.py`
- Create `scripts/tests/test_verify_signed_helper_bundle.py`
- Update `ARCHITECTURE.md`
- Update `.rps/STATE.md`
- Update `.rps/MEMORY.md`
- Create `docs/spikes/003-signed-app-helper/README.md`

Steps:

- [x] Add RED verifier tests for missing helper, duplicate helper, ad-hoc identity, mismatched teams, invalid outer seal, invalid nested signature, hash mismatch, and unexpected command surface.
- [x] Build an ad-hoc app and prove self-test fails closed.
- [x] Build a local Apple Development signed candidate using an environment-only identity.
- [x] Verify outer and nested signatures, exact non-empty matching team identity, generated identity hash, and strict outer seal.
- [x] Run the signed self-test and ordinary launch; prove ordinary launch creates no helper.
- [x] Tamper only with a disposable copy and prove outer verification and self-test fail; delete the copy.
- [x] Run default/all-feature Rust tests, frontend checks, all browser tests, release build, signature checks, residue checks, and launch.
- [x] Obtain independent review and fix every verified P0-P2.
- [x] Record exact results and remaining Developer ID/notarization/XPC limitations.

## Rollback

Delete the production helper, signed-helper modules/tests, C bridge, build/finalizer/verifier scripts and their tests, all three helper-related package commands, helper entitlement file, generated artifact configuration, `externalBin`/resource mappings, self-test argument, and generated ignores. Restore `beforeDevCommand` to `npm run dev` and `beforeBuildCommand` to `npm run build`. Re-run the existing non-shipping helper tests, frontend checks, browser suite, native build, signature verification, and launch.