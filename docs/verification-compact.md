# Compact v0.2.0 verification

## Outcome and scope
The approved compact core + Workspace drawer is implemented. The default native window is 960×720. The core remains mounted, timelines preserve phase, rate changes ease instead of seeking, and secondary tools no longer clutter the primary screen. Saved objectives, accent, motion and legacy preference fields survive reload. This is a local UI preview, not a public release or connected assistant.

## Executed checks
| Check | Observed result |
|---|---|
| `npm run check` | Exit 0: Biome lint has no errors/warnings; 3 unit tests pass; strict TypeScript and Vite production build pass |
| `npm run test:e2e` | 62/62 pass in Chromium/WebKit, no retries |
| Keyboard/exit-race stress | `npx playwright test tests/commands.spec.ts --grep 'exiting command|keyboard search' --repeat-each 5`: 40/40 pass across both engines |
| Motion continuity | Same ring players and phase across pause/resume, state and theme changes; stable canvas/core lifetime through tools/conversation; actual pixels freeze under pause/reduced motion/hidden visibility and resume |
| Layout | No desktop scrolling at 960×720 and 1440×1000; eight responsive sizes from 320×1000 through 1920×1080; composer, conversation and diagnostic drawer fit |
| Accessibility and flows | Automated WCAG A/AA checks on all current surfaces; focus trapping/return, direct composer, command search, cancellation, safe text, reset confirmation, storage corruption/denial and offline runtime pass |
| Rust | `cargo fmt --manifest-path src-tauri/Cargo.toml --check` and `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`: exit 0 |
| Native build | `npm run tauri -- build --bundles app`: exit 0; arm64 confirmed; final app and archive Info.plist report 0.2.0 |
| Signature/archive | `codesign --verify --deep --strict` passes; ZIP CRC has no bad entry |
| Launch | Final rebuilt app opened and its native executable process observed |

Raw current logs: `artifacts/compact-browser-verification.txt`, `artifacts/compact-native-verification.txt`, `artifacts/compact-artifact-verification.txt`. Older `final-*` logs belong to v0.1 and are not revision evidence.

## Animation samples
`node scripts/capture-refinement.mjs` instruments actual canvas paints, not simulated telemetry. The final capture observed 181 paints in each 3-second idle sample and 180 in each 3-second diagnostic sample. Maximum observed paint gap: Chromium 17.3 ms; WebKit 21 ms. Both persistent ring players remained running; inner playback rate settled from 1 to 1.45 during scanning. No browser page errors were captured. These short headless samples are not native display-refresh certification or a guarantee under every system load.

Data: `artifacts/compact/animation-evidence.json`. Screenshots: `home-`, `workspace-`, `amber-`, and `mobile-` with Chromium/WebKit suffixes under `artifacts/compact/`.

## Defects reproduced and resolved
- Pause removed ring animations; resume created new players at zero. Replaced removal with persistent pause/play timelines.
- State changed duration from 28000 to 4000 ms, changing phase at a fixed timestamp. Persistent-duration players now change speed with progress-preserving playback APIs.
- Navigation remounted the core and recreated the canvas. Secondary tools now use drawers while the core remains mounted.
- Implicit Enter submission stole composer focus. Pointer focus policy now excludes keyboard-generated synthetic clicks.
- A full verification rerun exposed an intermittent Escape race: the exiting palette consumed Escape while Workspace stayed open. A timing-controlled DOM/keyboard regression reproduced the same class for Workspace, Settings and Conversation. App-owned Escape handling closes the active panel, while existing guards keep retired dialogs from closing replacements.
- The previous CSS specificity warning was removed with the retired focus-mode styling; no lint rule was suppressed.

## Research and knowledge
Used the Codebase Memory MCP graph plus exact source reads and live browser probes. Primary Motion/MDN references and fetch limitations are recorded in [the refinement](compact-refinement.md). No paid animation service or additional animation framework was installed.

## Artifact and recovery
- Final archive: `/Users/rohtolos/Documents/GitHub/jarvis-dashboard/artifacts/jarvis-dashboard-v0.2.0-macos-arm64.zip`
- SHA-256: `635df38d8c76f76746336eb1bcc9238e998b1fdf6b911d6f175d020cda750542`
- App: `src-tauri/target/release/bundle/macos/J.A.R.V.İ.S.app`
- Prior app: `artifacts/jarvis-dashboard-v0.1-rollback.zip`; prior source: `artifacts/pre-compact-source.tgz`. Restore separately, not over ongoing work.
- Signature is local ad-hoc, not Developer ID notarization. Do not disable Gatekeeper for public distribution.

## Limits
Native launch/packaging was verified; native input flows and display performance were not independently automated. Headless WebKit is not a claim of full native certification. Intel macOS, Windows/Linux, screen-reader certification and owner aesthetic acceptance remain unverified. Hidden/occluded and reduced-motion pausing is intentional. No real AI, microphone, speech engine, host diagnostics, system control or cloud sync was added. No commits, pushes or deployment were performed. The prior dependency audit is historical and was not rerun for this UI-only revision; dependency versions did not change.
