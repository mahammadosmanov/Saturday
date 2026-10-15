# Saturday — Local bilingual assistant

A compact local-first interface with a native-only Turkish/English voice-assistant implementation in progress. The browser preview does not provide assistant or microphone fallback behavior.

## Open it

Build the macOS owner-setup bootstrap app from source. Install Node.js 22.12 or later, Python 3.12, Rust, Xcode Command Line Tools, a local Apple Development signing identity, and a matching provisioning profile that grants the app's voice-owner Keychain access group. Then run:

```sh
npm ci
npm run build:native-bootstrap
security find-identity -v -p codesigning
export APPLE_SIGNING_IDENTITY="<installed Apple Development identity>"
npm run finalize:signed-helper
npm run verify:signed-helper
open "src-tauri/target/release/bundle/macos/S.A.T.U.R.D.A.Y.app"
```

The bootstrap intentionally contains no voice workers and has no owner-key binding. It exists so the app owner can establish the personal Secure Enclave trust anchor; manually approve any macOS Keychain/Touch ID/TCC prompts. It is **not** a voice-ready app. A separate owner-bound build requires reviewed local worker artifacts, the owner's private bootstrap records, approved model assets, local signing, and native verification. Once those prerequisites are present, build the owner-bound product with `npm run build:native-full`; it layers `src-tauri/tauri.full.conf.json` over the bootstrap-safe base config.

Tauri initially assembles the bundle with local ad-hoc signing; `finalize:signed-helper` applies the local Apple Development identity and voice keychain entitlement. macOS also requires a matching provisioning profile for that entitlement; `codesign --verify` alone does not prove Gatekeeper will launch it. The result is not notarized or suitable for public distribution. Developer ID release certification and notarization remain release gates; CI does not use signing credentials. No Node.js, server, CLI, or AI account is required to run the built app.

For browser development, use Node.js 22.12 or later:

```sh
npm ci
npm run dev
```

Open `http://127.0.0.1:1420/`. For native owner-setup development, install the Tauri prerequisites (Rust and Xcode Command Line Tools), then run `npm run tauri -- dev --config src-tauri/tauri.bootstrap.conf.json`.



## Current behavior and implementation status

- The compact UI, command palette, saved appearance preferences, keyboard focus handling, reduced motion and responsive layouts are implemented.
- The React frontend contains a strict versioned native voice-protocol client, bounded snapshot/event handling, Turkish/English copy, typed-text and transcript review flows, native capture controls, setup/readiness and asset disclosures.
- Tauri source contains the corresponding native voice, capture, speech, worker, authorization and asset-management implementation. These source paths have not yet completed the final integrated build and runtime verification.
- The browser preview intentionally reports native voice as unavailable. Its test-only protocol driver exercises renderer contracts and is not a provider, native-capability substitute, or evidence of model readiness.

No browser microphone, Web Speech, scripted reply, cloud provider, or demo conversation fallback is shipped. Voice capture is explicit, bounded, native-only and intended to remain ephemeral. The UI does not claim that source code, runtimes, model assets, owner trust, signing, audio capture, local inference or spoken output are ready until their respective native gates are verified.

User preferences remain in their existing browser-local storage. Conversation/session content is not presented as durable history. No Vector files are modified.

## Stack and architecture

Tauri 2 + React 19 + TypeScript 7 + Vite 8. Motion handles message transitions; custom CSS/SVG/Canvas renders the compact core. Radix Dialog supplies focus-managed modal primitives. Fonts are bundled, not fetched from a CDN. Dependencies are locked by `package-lock.json` and `src-tauri/Cargo.lock`.

- `src/components/`: presentation and interactive surfaces.
- `src/hooks/`: native voice session ownership and app-local preference persistence.
- `src/domain/`: validated voice contracts, localization, preferences and retained legacy records.
- `src/platform/`: raw-byte native voice protocol transport and built-in command modules.
- `src-tauri/`: native voice implementation, workers, authorization, assets and restricted capability surface.
- `tests/`: browser behavior and accessibility checks; the native protocol driver is explicitly test-only.

Architecture and security boundaries are described in [ARCHITECTURE.md](ARCHITECTURE.md). The implementation is not yet a verified usable voice runtime: source/runtime receipts, reviewed worker artifacts, frozen model assets, owner-personal trust setup, native compilation, physical audio/inference/speech verification and installed-app proof remain required.

The automated GitHub Actions workflow (`.github/workflows/ci.yml`) runs on pushes and pull requests with read-only repository access. Its gates are:

- Node.js 22 with `npm ci`, `npm run check` (lint, unit tests, TypeScript check, production frontend build).
- Playwright Chromium and WebKit end-to-end/accessibility tests (`npx playwright install --with-deps chromium webkit`, then `npm run test:e2e`).
- On macOS arm64: fetch Rust dependencies, build the owner-setup bootstrap app (`npm run build:native-bootstrap`) so Tauri's source hooks generate bootstrap resources and the runtime-helper binary, then run the Python suite and Rust fixture tests/formatting/Clippy. The bootstrap contains no voice workers and does not verify the owner-bound voice product. The Python suite contains offline Cargo invocations.

Run the same gates locally with:

```sh
npm ci
npm run check
npx playwright install chromium webkit
npm run test:e2e
cargo fetch --locked --manifest-path src-tauri/Cargo.toml
npm run build:native-bootstrap
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
