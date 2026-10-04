# J.A.R.V.I.S — Personal command interface

An animated, local-first command center. UI/UX first, real assistant capabilities later. Independent of Vector.

## Open it

The macOS Apple Silicon build is at `src-tauri/target/release/bundle/macos/J.A.R.V.I.S.app`. Double-click it, or run:

```sh
open 'src-tauri/target/release/bundle/macos/J.A.R.V.I.S.app'
```

Portable archive: `artifacts/jarvis-dashboard-v0.3.1-ascii-name-macos-arm64.zip` (CRC verified). Older and unversioned archives are historical, not aliases of this candidate. This is a local, ad-hoc-signed development build, not an Apple-notarized public release. No Node.js, server, CLI, or AI account is required to run the packaged app. Do not disable Gatekeeper to distribute it; public distribution needs a separate signing/notarization step.

For browser development, use Node 22.12+:

```sh
npm ci
npm run dev
```

Open `http://127.0.0.1:1420/`. For native development, install the Tauri prerequisites (Rust and Xcode command-line tools), then `npm run tauri -- dev`.

## What works

- Compact 960×720 native interface: one animated core, a direct command composer, quiet header, and no scrolling at the default desktop size.
- Tasks, Diagnostics and Activity live in a Workspace drawer. Decorative telemetry/map panels, sidebar, coordinates and repeated labels are no longer on the main screen.
- Persistent ring timelines preserve position across pause/resume, state, theme and drawer changes. Short playback-rate ramps replace abrupt duration changes; static particle geometry is cached.
- Cmd/Ctrl+K command palette with search, arrow selection, Enter execution, and Escape dismissal.
- Cancellable diagnostic simulation with progress and an activity trail.
- Conversation drawer with explicitly scripted text responses. The microphone button now opens real local audio visualization, not a fabricated voice conversation.
- Larger core: 470px at 960×720, up to 520px on larger screens. Speech loudness expands and brightens the particle field; frequency bands shape three contours and radial detail, with bounded onset pulses, fast attack and gentle release.
- Settings-only microphone selector with a remembered, app-local input preference. It never changes macOS defaults or starts capture just by opening Settings. Missing selected devices are reported instead of silently switching inputs.
- Header contains only Workspace and Settings. Cyan/amber accents and animation pause remain in Settings and the command palette. The UI is compact by default; old density/widget preferences remain stored for compatibility, but do not clutter the settings UI.
- Locally saved objectives and preferences; reset requires confirmation. Malformed/unsupported stored data is sanitized; unreadable envelopes and unavailable storage are disclosed.
- Keyboard focus management, scrollable keyboard regions, responsive layouts, and OS reduced-motion support. Background visibility suspends ambient animation.

**Real:** local settings/checklist, records of UI actions, and user-triggered microphone loudness/spectrum analysis.

**Simulated:** diagnostic checks and assistant text replies. Audio is processed only on-device, never recorded, transcribed, persisted, played through speakers, or uploaded. Start listening explicitly; Stop, Escape, opening another panel, hiding the page, or closing the app releases capture. The OS permission prompt belongs to the owner. No host measurements, AI inference, cloud sync, external runtime requests, or machine control. Conversation and activity are session-only. Do not put secrets in demo conversation or objectives; localStorage is not encrypted storage.

## Stack and architecture

Tauri 2 + React 19 + TypeScript 7 + Vite 8. Motion handles message transitions; the Web Animations API controls persistent transform timelines, and custom CSS/SVG/Canvas renders the compact core. Radix Dialog supplies focus-managed modal primitives. Fonts are bundled, not fetched from a CDN. Dependency versions and native dependencies are locked by `package-lock.json` and `src-tauri/Cargo.lock`.

- `src/components/`: presentation and interactive surfaces.
- `src/hooks/`: owned/cancellable microphone capture, conversation/diagnostic lifecycles, motion and persistence.
- `src/domain/`: validated workspace data and deterministic telemetry.
- `src-tauri/`: small native shell, local assets, unchanged restrictive CSP, no command plugins, and microphone-only usage text/audio-input entitlement.
- `tests/`: real Chromium/WebKit flows and accessibility checks.
- `.rps/`: canonical project scope, decisions, state and durable lessons.

This stack keeps the UI reusable while leaving a native integration boundary available. It does not prematurely add an agent runtime or privileged bridge. Before real features, design typed adapters, permissions, data retention and cancellation per capability; never turn the demo hooks directly into unconstrained computer control.

## Verification

```sh
npx playwright install chromium webkit
npm run check
npm run test:e2e
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
npm run tauri -- build --bundles app
```

Current requirements, RPS alignment, verification, known gaps and recovery: [RPS acceptance contract](docs/rps-alignment.md). Feature history: [voice-reactive core](docs/voice-reactive-core.md). Screenshots/geometry in `artifacts/voice/` are historical; the current automated logs use the `rps-` prefix. Run `node scripts/capture-voice.mjs` with the dev server running to recreate browser evidence without using the physical microphone. The [compact verification](docs/verification-compact.md) and [initial stage verification](docs/verification.md) are historical.

## Recovery and boundaries

The previous compact app is preserved at `artifacts/jarvis-dashboard-v0.2.0-macos-arm64.zip`. Older recovery artifacts remain `artifacts/jarvis-dashboard-v0.1-rollback.zip` and `artifacts/pre-compact-source.tgz`. Restore into a separate directory rather than extracting over active work.

Reset workspace through Settings only after confirming you want to discard this app's local objectives/preferences. Browser and native storage are separate. Closing the app loses session-only activity and conversations, not saved workspace preferences. Nothing is written to Vector. No commits, pushes, deployment, system-wide installation, external integrations, or paid services were performed.

The artwork is original and inspired by cinematic assistant interfaces; this is not affiliated with Marvel or an official J.A.R.V.I.S. product.
