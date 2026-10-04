# J.A.R.V.İ.S Dashboard Implementation Plan

Historical initial plan. Do not execute obsolete no-microphone, clock or dense-layout requirements. [Current RPS contract and next work](../../rps-alignment.md) governs continuation; unchecked historical steps are not a new task queue.

> For agentic workers: use executing-plans, inline execution, with test-first vertical slices. User requested immediate implementation; do not re-ask execution-method questions within the approved local stage. No commits.

**Goal:** Build, exercise and deliver the approved cinematic dashboard as browser preview and macOS app.
**Architecture:** React presentation composes typed local UI state and deterministic simulation functions. Tauri contains the same built assets without a backend service. CSS/SVG/Canvas loops are centrally motion-gated.
**Tech Stack:** Tauri 2, React, TypeScript, Vite, Motion, Radix Dialog, Vitest, Playwright.
**Spec:** `docs/superpowers/specs/2026-09-07-dashboard-design.md`

## Global constraints
- Standalone `jarvis-dashboard`; existing Vector untouched.
- No real AI, microphone, host telemetry, external integration, paid call or machine control.
- All simulation labeled at the point of use; local clock/session elapsed are actual runtime values.
- Local build/run/test approved; commits, pushes, deployments, installation/replacement and expanded privileges not authorized.
- Bundled fonts; strict types; motion pause/reduced motion; keyboard and mobile usability.

## Task 1 — App shell and first working navigation
Files: package.json, tsconfig.json, vite.config.ts, index.html, src/main.tsx, src/App.tsx, src/styles.css, src/components/Chrome.tsx, tests/dashboard.spec.ts, playwright.config.ts.
- [ ] Resolve package versions from npm, write configs and install with lockfile.
- [ ] Write a browser test that expects `Command center`, visible `Demo environment`, and clicking `Systems` changes main heading to `Systems diagnostics`.
- [ ] Run `npx playwright test -g 'navigation'` against a minimal mount; require the expected missing-heading RED.
- [ ] Implement shell/nav and initial visual composition; rerun to GREEN.
- [ ] Build with `npm run build`; browser check errors and primary layout.

## Task 2 — Persistence and configurable workspace
Files: src/domain/preferences.ts, src/domain/preferences.test.ts, src/hooks/useWorkspace.ts, src/components/SettingsDialog.tsx, src/components/Objectives.tsx.
- [ ] Test `parseWorkspace(raw: string | null): Workspace` returns literal default accent `cyan`, motion `true`, density `comfortable`, and visible telemetry/network/activity/objectives on invalid JSON.
- [ ] Run `npm test -- preferences` to RED, implement parsing, rerun GREEN.
- [ ] Add a validation regression for foreign values, excessive entries, duplicate IDs and oversized text; watch RED, clamp/filter safely and rerun GREEN.
- [ ] Add browser test for checking an objective, changing accent/motion/widgets, reload and persistence; watch RED, implement useWorkspace and dialogs, rerun GREEN.
- [ ] Test reset confirmation and storage-denied warning without crashing; implement then rerun.

## Task 3 — Instrument animation and diagnostic interaction
Files: src/components/Core.tsx, src/components/Instruments.tsx, src/components/Diagnostics.tsx, src/domain/simulation.ts, src/domain/simulation.test.ts, src/hooks/useSimulation.ts, src/styles.css.
- [ ] Test `telemetryAt(tick: number)` stays bounded at explicit probe times and reports source `simulation`; watch RED, implement and rerun.
- [ ] Add browser scanner test: start reveals running status, cancel reveals cancelled status; repeated starts cannot overlap; completed run gets success. Watch RED before implementing the scanner hook.
- [ ] Implement HUD core rings, Canvas particle field, telemetry sparklines, radar sweep and topology tied to a central motion-enabled value. Motion pause never blocks deterministic diagnostic progress.
- [ ] Test reduced-motion disables CSS loops and Canvas scheduling; test paused/hidden states, adjust before proceeding.

## Task 4 — Command palette and demo conversation
Files: src/components/CommandPalette.tsx, src/components/Conversation.tsx, src/domain/commands.ts, src/domain/commands.test.ts, src/hooks/useConversation.ts.
- [ ] Test `demoReply(prompt: string)` explicitly says the assistant is simulated for unknown text and status explains no host measurements. RED, implement, GREEN.
- [ ] Browser-test Cmd/Ctrl+K, filter, arrows/Enter, Escape focus return; RED then palette implementation then GREEN.
- [ ] Browser-test chat send, empty and long input, scripted reply, voice preview with no getUserMedia call and cancellation. RED then implement owned timers, caps and explicit labels then GREEN.
- [ ] Connect action log and dedicated Systems/Network/Activity views; verify every visible action works or has a clear disabled reason.

## Task 5 — Native shell and full acceptance
Files: src-tauri/Cargo.toml, src-tauri/tauri.conf.json, src-tauri/build.rs, src-tauri/src/main.rs, src-tauri/icons/*, README.md, docs/verification.md, .rps/STATE.md.
- [ ] Write minimal native shell with no command plugins and no external IPC capabilities. Bundle only local assets and use restrictive CSP.
- [ ] Run full `npm run check`, Playwright Chromium/WebKit at desktop/mobile, and axe audits in primary/dialog states. Fix defects without weakening assertions.
- [ ] Audit design composition, text contrast, clipping and all advertised controls; capture actual screenshots to artifacts/.
- [ ] Run `npm run tauri -- build --bundles app`, inspect native bundle and launch process/window on this Mac. State code-sign/notarization limits explicitly.
- [ ] Start localhost preview, verify HTTP/DOM, open actual app for user, checkpoint results and report exact artifact paths.

## Execution outcome

The vertical UI slices, native packaging and local launch were implemented. The checklist above is preserved as the prospective plan, not a claim of one-to-one file or test organization. The final acceptance record is `docs/verification.md`: 3 unit tests and 40 cross-engine browser tests passed, with exact tested behaviors, resolved findings and omissions recorded. In particular, scripted responses were verified through the real conversation interface rather than the planned separate commands unit module. Native launch/window evidence is distinct from headless WebKit input coverage. No commits or release were made.

## Self-review
All spec flows map to Tasks 1–4; responsive, accessibility, offline assets and native checks map to Task 5. No production integration or release is implied. Main unknown is final user aesthetic acceptance, which automated tests cannot certify.
