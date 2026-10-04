# Larger, locally voice-reactive core — v0.3.1

Current consolidated evidence and open gates: [RPS alignment](rps-alignment.md). Sections below preserve revision-specific history; historical owner gates and archive aliases are not current instructions.

## Header cleanup (current)
Owner requested removal of UI PREVIEW, pause button, clock and logo. Removed those header elements and the Clock/Emblem import; remaining Workspace/Settings controls stay right-aligned. The central J.A.R.V.İ.S identity, voice renderer, microphone Stop button and point-of-use simulation disclosures remain unchanged. Motion settings and command-palette pause/resume retain accessibility without a header button. Scripted status no longer claims the removed clock exists.

Verification: header absence regression failed before implementation in both engines. `npm run check` passes clean lint/typecheck/build and 8 unit tests; `npm run test:e2e` passes 100 tests across Chromium/WebKit, retaining motion/pixel/phase assertions through the command palette. Native app build and strict signature verification pass. Deliverable: usual bundle path plus `artifacts/jarvis-dashboard-v0.3.1-clean-header-macos-arm64.zip`; prior cinematic archive preserved for rollback. No settings migration or microphone changes.

## Cinematic voice response follow-up (historical)
The owner confirms input capture works and reports that the animation is too passive. This refinement changes only `src/components/Core.tsx`, `src/minimal.css` and the browser regression: stronger low-level visual gain, 24ms attack/140ms release, deforming/expanding particle mesh, three frequency-shaped contours, radial voice detail, and bounded onset pulses (three reusable slots, 420ms cooldown, 650ms lifetime). The canvas covers the full core while preserving idle geometry scale. No microphone, storage, permissions, network or dependency changes.

Verification: `npm run check` passes clean lint/typecheck/build and 8 unit tests; `npm run test:e2e` passes all 98 Chromium/WebKit tests. A new ordinary-speech outer-core pixel regression failed in both engines before implementation and passes afterward, including settling during silence. Existing pause/reduced-motion, theme continuity, layout, offline and microphone lifecycle tests pass. `npm run tauri -- build --bundles app`, strict ad-hoc signature verification, arm64 architecture, audio-input-only entitlement and archive CRC pass. Native performance and the owner's aesthetic assessment of this stronger response remain unverified; synthetic input is not physical-device evidence.

Delivery: rebuilt v0.3.1 app at the usual bundle path; distinct `artifacts/jarvis-dashboard-v0.3.1-cinematic-macos-arm64.zip` preserves the previous v0.3.1 archive as rollback. No migration, system input changes, commit or push performed by this task.

## Settings input selection follow-up (historical)
The owner granted microphone permission, but the shipped native view reported disconnection immediately. App-scoped WebKit logs show permission granted for **Aggregate Device**, analyser/source creation, then `RealtimeMediaSource::captureFailed`. System audio inventory confirmed Aggregate Device was the default input and MacBook Pro Microphone was also available. This is a successful permission grant followed by failed device capture, not evidence that the owner denied permission.

The owner approved app-only input selection and required it to remain in **Settings** ("ekle ama settingsde olsun"). The selector enumerates audio inputs without starting capture, refreshes on device change, fences late list results, remembers the exact selected ID in the existing version-1 preference envelope, and passes an exact `deviceId` constraint only for an explicit selection. Legacy preferences default to System default. Missing inputs remain visible and produce a recovery message; there is no silent fallback. Audio itself is never persisted and no system-wide input setting changes.

Current verification: **96/96 Chromium/WebKit tests and 8/8 unit tests**, lint/typecheck/build, Rust fmt/clippy, native build and ad-hoc signature/audio-only entitlement/archive CRC checks pass. See `artifacts/input-check-verification.txt`, `artifacts/input-browser-verification.txt`, `artifacts/input-build-verification.txt`, and `artifacts/input-native-verification.json`. New regressions cover Settings-only placement, no capture on discovery, selection persistence, exact constraints, missing-device handling and enumeration failure. Current archive: `artifacts/jarvis-dashboard-v0.3.1-macos-arm64.zip`; the unversioned archive matches. The previous versioned archives remain intact.

**Remaining native gate:** select MacBook Pro Microphone in Settings, then verify physical voice response and Stop. This has not yet been observed on v0.3.1; do not promote browser fixtures into native proof. The rebuilt app is running. The original v0.3.0 evidence below is historical.


## Decision and scope
The owner requested a larger J.A.R.V.İ.S animation area that reacts to their voice while listening, then approved both changes ("Evet, ikisini de uygula") after disclosure of local-only analysis, explicit microphone start/stop, and owner-granted macOS permission. This is a bounded extension of the existing listening UI, not an AI/voice assistant, transcription service or native command bridge.

## Behavior and data boundary
- `useMicrophone` acquires audio only on an explicit user action. No automatic start or background restart.
- Web Audio measures short-lived RMS energy and frequency bands. A mutable signal feeds the existing Canvas loop without per-frame React renders or a second visual loop.
- The particle field expands/brightens with energy; a polar ring responds to low/mid/high bands. Fast attack and slower release soften changes without remounting or seeking existing ring timelines. Reduced motion and Pause freeze visuals; the explicit Stop control still releases capture.
- Stop, Escape, another panel, hidden document, pagehide and unmount dispose tracks, nodes, buffers and context. Cancelled permission promises cannot revive an old capture. End handlers are installed before awaits and track liveness is checked before listening.
- No recording, transcription, audio persistence, speaker playback, upload, AI provider, new dependency or CSP relaxation. The only added native entitlement is audio input, with a microphone usage explanation. No camera permission.
- The old fabricated voice interaction is removed. Text conversation and diagnostics remain clearly scripted.
- Default desktop core is 470px rather than 340px; large screens allow 520px. Responsive space is reserved for error text without scrolling at native minimum/default dimensions.

## Verified on the implemented candidate
- `npm run check`: lint/typecheck/production build and 7 unit tests pass. Log: `artifacts/voice-check-verification.txt`.
- `npm run test:e2e`: 90/90 Chromium/WebKit tests pass with retries disabled. Log: `artifacts/voice-browser-verification.txt`.
- Real Web Audio processing of an oscillator-derived MediaStream drives and quiets rendered ring pixels. The physical-device boundary is substituted: these are not recordings or a physical Mac microphone test.
- Capture checks include no auto-start, Stop, Escape, panel opening, hidden/pagehide cleanup, denied permission and retry, cancelled/late permission, ended input before startup, post-start disconnection, conversation entry, and reduced-motion stop behavior.
- Test-first failures reproduced missing voice response, the old 340px core, false listening with ended input, and permission-error overflow; targeted regressions and the full suite pass after fixes.
- Independent read-only review identified the startup-ended-track race. It was reproduced in both engines and fixed by early listeners, readiness checks and cleanup.
- Geometry/evidence from `node scripts/capture-voice.mjs`: 960×720, 600×600, 390×844 and 1440×900, both idle and denied permission, fit without horizontal/vertical overflow in both engines. No page errors. See `artifacts/voice/geometry.json` and PNGs.
- Rust fmt/clippy and native arm64 build pass. Log: `artifacts/voice-build-verification.txt`. Bundle version, microphone usage text, audio-input-only entitlement, ad-hoc signature, architecture and archive CRC checked in `artifacts/voice-native-verification.json`.

## Original v0.3.0 owner gate / limits (superseded above)
**Physical microphone capture in the packaged macOS WKWebView is not yet verified.** The agent has not opened the physical microphone or granted any system permission. The owner must start listening in the rebuilt app and approve the macOS/webview microphone prompt, then check voice response and Stop. Browser-engine tests do not certify TCC prompts or the shipped native origin. Native runtime accessibility/performance/aesthetic acceptance and public notarization are not certified.

## Local delivery and recovery
- App: `src-tauri/target/release/bundle/macos/J.A.R.V.İ.S.app`.
- Archive: `artifacts/jarvis-dashboard-v0.3.0-macos-arm64.zip`; unversioned archive points to the same build.
- Previous v0.2.0 archive preserved and CRC checked. Restore it into a separate directory, not over active work.
- No storage migration/deletion; objectives/settings remain intact. No commits, pushes, deployment, system install, paid service or Vector changes.
