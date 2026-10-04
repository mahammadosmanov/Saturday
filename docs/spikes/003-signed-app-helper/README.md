# Spike 003 — Signed app-owned helper

Date: 2026-09-20
Status: **GO for the dormant local Apple Development signed-helper boundary; NO-GO for model integration or distribution release**

## Question

Can J.A.R.V.I.S bundle a dormant helper, bind it to current source and the signed app, mutually verify the live parent/child signing team before IPC, and exercise a narrow lifecycle without exposing the helper to the webview?

## Implemented

- Tauri bundles exactly one `jarvis-runtime-helper` nested executable.
- Ordinary app startup does not launch it.
- The webview has no invoke handler, shell plugin, helper permission, or capability entry.
- A C Security.framework bridge compares valid non-ad-hoc team identities for the current process and a directly owned live peer PID.
- The helper checks its immediate parent before reading stdin.
- The native parent checks the spawned live child before writing `Hello`.
- IPC uses the existing strict framed protocol and supports only handshake, health, cancellation, and shutdown.
- A deterministic build script hashes an explicit source allowlist, uses Cargo `--locked`, recomputes the generation after build, validates arm64 Mach-O, and stages ignored generated artifacts atomically.
- Finalization signs the helper with hardened runtime and an explicit empty entitlement set, seals its post-signing hash into the resource, then signs the app with hardened runtime and the audio-input entitlement.
- Verification recomputes current source generation and enforces exact bundle placement, one helper, matching non-empty team identity, hardened runtime, exact entitlements, helper hash/size, empty webview capabilities, self-test success, and stable source inputs.

## Verified results

- Default Rust suite: 36 unit tests passed; fixture integration skipped.
- All-feature Rust suite: 36 unit tests plus 13 helper-boundary integration tests passed.
- Warnings-denied Clippy passed.
- Python build/release suite: 13 tests passed.
- Frontend check: 20 tests, lint, TypeScript, and production frontend build passed.
- Browser suite: 102/102 passed.
- Ad-hoc bundle self-test failed closed.
- Apple Development signed candidate finalized and verifier passed.
- Signed self-test completed health, cancellation, shutdown, stderr, and cleanup checks.
- Ordinary launch produced the app process and no helper process.
- Disposable helper tampering broke the outer signature and self-test; the copy was deleted.
- A stale source generation was rejected after the verifier correction.
- A deliberately wrong sealed generation produced the fixed `protocol` failure.

## Independent review outcome

Review-driven corrections added current-source generation binding, locked Cargo builds, exact pre-sign staged-byte validation, untouched outer-seal validation before trust, hardened runtime on app and helper, empty helper entitlements, strict EOF and nonzero-exit handling, nonblocking flood cleanup, an absolute helper lifetime cap, verified temporary-root deletion, and complete rollback hooks. Final lifecycle review reported no unresolved P0-P2 findings. The final provenance regression asserts the exact first outer-verification command and unchanged helper/identity bytes, mode, and mtime when that verification rejects the candidate.

## Boundaries

This is not real AI and the helper is dormant outside the native self-test. Apple Development signing is not Developer ID distribution and the app is not notarized. Privileged automation and untrusted plugins still require XPC/audit-token authentication or an independently reviewed equivalent. No model was downloaded, installed, or integrated.

## Rollback

Follow the complete rollback inventories in the signed-helper design and implementation plan. Generated helper binaries and identity files are ignored build artifacts and can be deleted without affecting the non-shipping helper harness or scripted UI provider.
