# Saturday — Local bilingual assistant

A compact local-first interface with a native-only Turkish/English voice-assistant implementation in progress. The browser preview does not provide assistant or microphone fallback behavior.

## Open it

Build the macOS owner-setup bootstrap app from source. Install Node.js 22.12 or later, Python 3.12, Rust, Xcode Command Line Tools, and a local Apple Development signing identity. Voice-owner keys and authorization state use the app's default app-scoped Keychain group; the app no longer requests a custom shared Keychain entitlement. Then run:

```sh
npm ci
npm run build:native-bootstrap
security find-identity -v -p codesigning
export APPLE_SIGNING_IDENTITY="<installed Apple Development identity>"
npm run finalize:signed-helper
npm run verify:signed-helper
open -n -a "src-tauri/target/release/bundle/macos/S.A.T.U.R.D.A.Y.app" --args --voice-owner-bootstrap
```

The bootstrap intentionally contains no voice workers and has no owner-key binding. It exists so the app owner can establish the personal Secure Enclave trust anchor; manually approve any macOS Keychain/Touch ID/TCC prompts. The owner key and ledger remain app-scoped; workers never receive Keychain access. Existing data created under the former shared voice-owner group is not migrated automatically. This is **not** a voice-ready app. A separate owner-bound build requires reviewed local worker artifacts, the owner's private bootstrap records, approved model assets, local signing, and native verification. Once those prerequisites are present, build the owner-bound product with `npm run build:native-full`; it layers `src-tauri/tauri.full.conf.json` over the bootstrap-safe base config.

The bootstrap command waits until its main window is focused, then displays a Cancel-first native consent prompt. Continue only if you intend to create this app's local Secure Enclave trust key.

Tauri initially assembles the bundle with local ad-hoc signing; `finalize:signed-helper` applies the local Apple Development identity and derives the macOS `com.apple.application-identifier` entitlement from the signing team and bundle identifier. It adds no shared Keychain access group. `npm run verify:signed-helper` verifies the signed helper, app entitlements, and bundled resources; it does not exercise Keychain APIs. On this workstation, native consent succeeded but Secure Enclave Keychain lookup returned `errSecMissingEntitlement` (-34018), so owner setup is blocked until macOS accepts the app's Keychain access-group entitlement. `codesign --verify` alone does not prove Gatekeeper will launch it. The result is not notarized or suitable for public distribution. Developer ID release certification and notarization remain release gates; CI does not use signing credentials. After the one-time owner setup, running the app requires no Node.js, local server, or AI account.

For browser development, use Node.js 22.12 or later:

```sh
npm ci
npm run dev
```

Open `http://127.0.0.1:1420/`. For native owner-setup development, install the Tauri prerequisites (Rust and Xcode Command Line Tools), then run `npm run tauri -- dev --config src-tauri/tauri.bootstrap.conf.json`.



## Current behavior and implementation status

- The compact UI, command palette, saved appearance preferences, keyboard focus handling, reduced motion and responsive layouts are implemented.
- The React frontend contains a strict versioned native voice-protocol client, bounded snapshot/event handling, Turkish/English copy, typed-text and transcript review flows, native capture controls, setup/readiness and asset disclosures.
- Tauri source contains the corresponding native voice, capture, speech, worker, authorization and asset-management implementation. The bootstrap app package was built and signature-checked, but native owner setup currently fails at Secure Enclave Keychain lookup with `errSecMissingEntitlement` (-34018) on this local signing identity. The owner-bound voice product has not completed its integrated build or runtime verification.
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

Architecture and security boundaries are described in [ARCHITECTURE.md](ARCHITECTURE.md). The implementation is not yet a verified usable voice runtime: source/runtime receipts, reviewed worker artifacts, frozen model assets, owner-personal trust setup, and physical audio/inference/speech verification remain required. Bootstrap owner-key setup currently stops on `errSecMissingEntitlement`; the package contains no voice workers or model assets.

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
