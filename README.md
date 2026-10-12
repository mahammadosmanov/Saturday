# Saturday — Personal command interface

An animated, local-first command center. UI/UX first, real assistant capabilities later. Independent of Vector.

## Open it

The packaged macOS application is built from source. Install Node.js 22.12 or later, Python 3.12, Rust, and Xcode Command Line Tools, then run:

```sh
npm ci
npm run tauri -- build --bundles app
```

The build prepares the public bootstrap resources and native helper from source, and uses local ad-hoc signing. It is not notarized or suitable for public distribution. Developer ID signing, hardened-runtime release certification, and notarization are manual release gates; CI does not use signing credentials. No Node.js, server, CLI, or AI account is required to run a built app.

For browser development, use Node.js 22.12 or later:

```sh
npm ci
npm run dev
```

Open `http://127.0.0.1:1420/`. For native development, install the Tauri prerequisites (Rust and Xcode Command Line Tools), then `npm run tauri -- dev`.



## What works

- Compact 960×720 native interface: one animated core, direct command composer, and no scrolling at the default desktop size.
- Cmd/Ctrl+K command palette with search, arrow selection, Enter execution, and Escape dismissal.
- Conversation drawer with explicitly scripted text responses.
- Larger core: 470px at 960×720, up to 520px on larger screens. Speech loudness expands and brightens the particle field; frequency bands shape three contours and radial detail, with bounded onset pulses, fast attack and gentle release.
- User-triggered microphone visualization uses the saved app-specific input choice, or the system default when none is saved. Capture never changes macOS defaults and starts only after an explicit action. If the saved exact-device choice is rejected as overconstrained, an explicit “Use system default” control clears that app-specific choice and retries capture with the default device.
- Existing accent and motion preferences remain loaded from local storage. Motion can still be paused or resumed from the command palette; there is no Settings surface.
- Social Studio, Workspace, task, diagnostic, and activity surfaces are removed. Existing saved records remain under their current local-storage keys; the app does not delete or migrate them.
- Keyboard focus management, responsive layouts, OS reduced-motion support, and background-visibility animation suspension.

**Real:** saved local preferences and user-triggered microphone loudness/spectrum analysis.

**Simulated:** assistant text replies. Audio is processed only on-device, never recorded, transcribed, persisted, played through speakers, or uploaded. Stop, Escape, opening another panel, hiding the page, or closing the app releases capture. The OS permission prompt belongs to the owner. No host measurements, AI inference, cloud sync, external runtime requests, diagnostics, activity feed, or machine control. Conversation history is session-only. Do not put secrets in demo conversation; localStorage is not encrypted storage.

## Stack and architecture

Tauri 2 + React 19 + TypeScript 7 + Vite 8. Motion handles message transitions; the Web Animations API controls persistent transform timelines, and custom CSS/SVG/Canvas renders the compact core. Radix Dialog supplies focus-managed modal primitives. Fonts are bundled, not fetched from a CDN. Dependency versions and native dependencies are locked by `package-lock.json` and `src-tauri/Cargo.lock`.

- `src/components/`: presentation and interactive surfaces.
- `src/hooks/`: owned/cancellable microphone capture, scripted conversation, motion and app-local preference persistence.
- `src/domain/`: validated preferences, retained legacy workspace records, and audio processing.
- `src-tauri/`: small native shell, local assets, unchanged restrictive CSP, no command plugins, and microphone-only usage text/audio-input entitlement.
- `tests/`: real Chromium/WebKit flows and accessibility checks.
- Active project architecture and security contracts are documented in `ARCHITECTURE.md`. Private project ledgers, historical verification records, and local build evidence are excluded from the public source snapshot.

This stack keeps the UI reusable while leaving a native integration boundary available. It does not prematurely add an agent runtime or privileged bridge. Before real features, design typed adapters, permissions, data retention and cancellation per capability; never turn the demo hooks directly into unconstrained computer control.

The automated GitHub Actions workflow (`.github/workflows/ci.yml`) runs on pushes and pull requests with read-only repository access. Its gates are:

- Node.js 22 with `npm ci`, `npm run check` (lint, unit tests, TypeScript check, production frontend build).
- Playwright Chromium and WebKit end-to-end/accessibility tests (`npx playwright install --with-deps chromium webkit`, then `npm run test:e2e`).
- Python regression suite (`python3 -m unittest discover -s scripts/tests -v`).
- On macOS arm64: fetch Rust dependencies, build the clean-source ad-hoc `.app` first so Tauri's source hooks generate bootstrap resources and the runtime-helper binary, then run the Python suite and Rust fixture tests/formatting/Clippy. The Python suite contains offline Cargo invocations.

Run the same gates locally with:

```sh
npm ci
npm run check
npx playwright install chromium webkit
npm run test:e2e
cargo fetch --locked --manifest-path src-tauri/Cargo.toml
npm run tauri -- build --bundles app
python3 -m unittest discover -s scripts/tests -v
cargo test --locked --offline --manifest-path src-tauri/Cargo.toml --features fixture-helper
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --locked --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

The native build needs macOS arm64, Rust, and Xcode Command Line Tools. It creates only ignored generated inputs and a locally ad-hoc-signed app; it does not provision an owner, authenticate an owner, or use release credentials. Hosted CI has not been executed or claimed as passing; the first hosted result requires an authorized push. CI does not publish artifacts or perform Developer ID signing, notarization, or distribution certification.

Current product direction and security boundaries: [architecture](ARCHITECTURE.md). Historical verification records, project ledgers, and local build evidence are excluded from the public source snapshot.

## Recovery and boundaries

Prior app archives and recovery artifacts are local historical evidence and are not part of the public source repository. Restore any recovery copy into a separate directory rather than extracting over active work.

Previously saved workspace and Social Studio data remain in browser storage but no longer have an app UI. The saved microphone choice remains active; clearing that stored value is required to return to the system default. Browser and native storage are separate. Closing the app loses session-only conversations, not saved preferences. Nothing is written to Vector. No deployment, system-wide installation, external integrations, or paid services.

The artwork is original and inspired by cinematic assistant interfaces; this project is not affiliated with Marvel or an official Saturday product.
