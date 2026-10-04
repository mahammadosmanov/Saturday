# Saturday — Personal command interface

An animated, local-first command center. UI/UX first, real assistant capabilities later. Independent of Vector.

## Open it

The packaged application is built locally; generated binaries and signatures are not checked into this repository. To build the macOS app after installing the Tauri prerequisites (Rust and Xcode command-line tools), run:

```sh
npm ci
npm run tauri -- build --bundles app
```

This development build uses local ad-hoc signing; it is not notarized and is not suitable for public distribution. Public distribution needs a separate signing/notarization review. No Node.js, server, CLI, or AI account is required to run a built app.

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
- Active project architecture and security contracts are documented in `ARCHITECTURE.md`. Private project ledgers, historical verification records, and local build evidence are excluded from the public source snapshot.

This stack keeps the UI reusable while leaving a native integration boundary available. It does not prematurely add an agent runtime or privileged bridge. Before real features, design typed adapters, permissions, data retention and cancellation per capability; never turn the demo hooks directly into unconstrained computer control.

## Verification

```sh
npx playwright install chromium webkit
npm run check
npm run test:e2e
python3 -m unittest discover -s scripts/tests -v
cargo test --manifest-path src-tauri/Cargo.toml --features fixture-helper
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm run tauri -- build --bundles app
```

Current product direction and security boundaries: [architecture](ARCHITECTURE.md). Historical verification records, project ledgers, and local build evidence are excluded from the public source snapshot.

## Recovery and boundaries

Prior app archives and recovery artifacts are local historical evidence and are not part of the public source repository. Restore any recovery copy into a separate directory rather than extracting over active work.

Reset workspace through Settings only after confirming you want to discard this app's local objectives/preferences. Browser and native storage are separate. Closing the app loses session-only activity and conversations, not saved workspace preferences. Nothing is written to Vector. No deployment, system-wide installation, external integrations, or paid services.

The artwork is original and inspired by cinematic assistant interfaces; this project is not affiliated with Marvel or an official Saturday product.
