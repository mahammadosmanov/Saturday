# Stage 1 verification — 2026-09-07

**Historical v0.1 evidence. Superseded by [compact v0.2 verification](verification-compact.md).** The old test count, layouts, screenshots, warning and checksum below do not describe the current candidate.

## Result and scope

UI-first implementation verified locally; not a released or connected AI assistant. The final source builds into a macOS arm64 application. Visual preference/fidelity remains the owner's judgment; automated checks do not certify aesthetics or full accessibility.

| Check | Actual result |
|---|---|
| `npm run check` | Exit 0: strict TypeScript check and production build passed; 3 unit tests passed; Biome has 0 errors and 1 nonblocking CSS specificity warning |
| `npm run test:e2e` | 40/40 passed across Chromium and WebKit, no retries |
| Responsive matrix | 320×1000, 390×844, 768×1024, 1024×1000, 1280×800, 1440×900, 1440×1000, 1920×1080; overview/conversation/systems usable without horizontal overflow |
| Desktop fit | Entire overview, including footer, fits 1440×1000 |
| Automated accessibility | Zero axe WCAG 2 A/AA and 2.1 AA violations in overview, Systems, Network, Activity, Settings, amber settings/overview, command palette and conversation |
| Motion | Pause and OS reduced-motion stop ambient Canvas/CSS motion; actual painted Canvas and advancing frames recorded |
| Runtime isolation | Browser observed no external HTTP requests and no page errors in the instrumented offline flow; current render capture has no page errors |
| `npm audit --omit=dev --json` | 0 reported production dependency vulnerabilities at check time; not a security certification |
| Rust checks | `cargo fmt --check`, `cargo check`, and `cargo clippy -- -D warnings` passed |
| Native packaging | `npm run tauri -- build --bundles app` passed; Mach-O arm64 confirmed |
| Signature/archive | `codesign --verify --deep --strict` passed; archive CRC check returned no bad entry |
| Native launch | Final bundle reopened, process and native titled window observed. The preceding bundle had a user-authorized foreground capture; a fresh final-bundle foreground capture was unavailable after the desktop driver session ended. Native launch evidence is separate from the automated headless WebKit input suite |

Raw logs: `artifacts/final-browser-verification.txt`, `artifacts/final-native-verification.txt`. Real browser screenshots: `dashboard-desktop.png`, `dashboard-diagnostic.png`, `command-palette.png`, `conversation.png`, `dashboard-amber.png`, `dashboard-mobile.png`. `artifacts/render-evidence.json` includes measured geometry, loaded fonts, actual Canvas painting and frame advancement. A one-second frame sample is not a hardware performance certification.

## Behavioral coverage

- Working navigation and explicit demo boundaries.
- Command search/execution, no-result handling, Cmd/Ctrl+K, Escape and focus return.
- Cancellable, non-overlapping diagnostics and completion events.
- Scripted conversation/status, safe text rendering, pending cancellation, labeled voice sample, and no delayed diagnostic action after closing a pending conversation.
- Saved accent, density, motion, widget visibility and objective state after reload.
- Focus trapping, confirmed/cancelled reset, malformed preference recovery notice, and session-only fallback when storage is blocked.
- Focus mode and its Escape exit; offline diagnostic/activity path.

## Findings and resolution

1. **Keyboard-inaccessible activity scroller:** detected by axe; made the scroll region keyboard reachable. Full-page/dialog audits pass in both engines.
2. **WebKit modal return focus:** Chrome-focused clicked buttons, while WebKit's native mousedown returned focus to BODY. Pointerdown focus was insufficient; event tracing established the cause. Scoped click-capture focus now preserves openers; the unchanged focus assertion passes in both engines.
3. **Dashboard exceeded its intended desktop viewport:** added a failing fit test, then height-aware desktop spacing and core sizing. 1440×1000 now fits.
4. **Unreadable preferences silently recovered:** a failing visible-notice assertion exposed the missing state. Recovery now explains the safe defaults; storage denial remains a separate session-only notice.
5. **Inconclusive native background capture:** a fully covered WKWebView was suspended/occluded. Native load logs and a user-authorized foreground capture distinguished this from startup failure. CSP/permissions were not loosened. A rapid quit/reopen briefly returned LaunchServices -600; reopening after the process exited succeeded.

One Biome `noDescendingSpecificity` warning remains for unrelated selectors `.focused .core-identity strong` and `.activity-panel.expanded .event-title strong`. It is disclosed, not suppressed; lint exits 0. No test or security rule was disabled to pass verification.

## Native artifact

- App: `src-tauri/target/release/bundle/macos/J.A.R.V.İ.S.app`
- ZIP: `artifacts/jarvis-dashboard-macos-arm64.zip`
- ZIP SHA-256: `a92f8408eb615e29df65e7c28f8aa2f581daf92175b8c9287124c6034a70d384`
- Signature: local ad-hoc, not Developer ID notarization.
- Native launch was exercised on this Mac. Native input flows were not independently automated with a native driver; those flows were exercised in real headless Chromium and WebKit. Intel macOS, Windows and Linux are unverified.

## Deliberate omissions

No AI provider, speech recognition/synthesis, microphone permission, real system telemetry, computer control, sync, secrets store, auto-update, public hosting, Apple notarization, production release, or human aesthetic acceptance. Nothing in the diagnostics establishes real machine health. No budget was spent on services; dependency retrieval/builds require initial internet/toolchain setup.

## Implementation-plan reconciliation

The planned pure `domain/commands.ts` seam was kept in the owned conversation hook instead of adding a speculative module. Scripted behavior is covered through the actual browser conversation interface; there is no separate commands unit-test file. Diagnostic timers live in `useSimulation.ts` and conversation timers in `useConversation.ts`; visuals are separated into component-specific styles plus `viewport.css`. These are implementation organization changes, not added assistant capabilities.
