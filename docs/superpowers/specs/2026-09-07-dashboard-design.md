# J.A.R.V.İ.S Dashboard — stage 1 design

Historical initial design. [Current RPS contract](../../rps-alignment.md) indexes explicit superseding requirements. The implementation status below describes the original stage, not current open work.

Status: implementing the standalone direction approved in chat by the user's "do it" instruction. No claim of separate written-artifact approval. Date: 2026-09-07.

## Outcome and scope
A locally runnable, visually rich desktop app with an equivalent browser preview. The primary surface is Monitor with secondary Command/Inspect. Personal, single-user, no accounts. The existing Vector project remains untouched. This is an original cinematic interpretation of the requested assistant aesthetic, with no film assets or claim of affiliation.

## Stack decision
Tauri 2 with React, strict TypeScript and Vite; Motion for state transitions; CSS/SVG for instrument loops; Canvas for a bounded particle core; accessible Radix dialogs; bundled Rajdhani and IBM Plex Mono fonts. npm lockfile and Cargo.lock pin the actual resolved dependencies. Vitest for domain behavior; Playwright Chromium/WebKit for flows and a11y checks.

Tauri is chosen over Electron because stage 1 needs no Node backend and should not bundle Chromium. Native SwiftUI would provide deeper macOS conventions but constrains reuse and requires a different UI implementation. Vite SPA is appropriate for a local dashboard: no SSR, SEO, server components, hosting, database or framework server needed. Rust native layer remains minimal until a real integration is authorized.

Sources checked: https://v2.tauri.app/start/ , https://v2.tauri.app/start/frontend/vite , https://react.dev/learn/build-a-react-app-from-scratch , https://motion.dev/docs/react-motion-config . These sources support compatibility, not a universal best-stack claim.

## Composition and visuals
A narrow navigation rail, top status strip, screen heading, three-column command center, large central assistant instrument and asymmetrical surrounding panels. Deep charcoal/blue-black canvas with precise cyan engineering linework and restrained amber for simulation notices. No purple SaaS gradient, oversized rounded cards, film imagery or illegible filler. Typography: Rajdhani display/interface, IBM Plex Mono instrument captions. Chamfered corners, thin grid, radial labels and crosshairs provide HUD character.

The core has concentric counter-rotating rings, segmented radial meters, orbital particles, a central wireframe field and state-aware brightness. Telemetry charts, signal-map sweeps and progressive scanner steps animate only while motion is enabled and the page is visible. Mount transitions are brief; no unskippable boot sequence. Idle motion must not block any control.

## Working flows
1. Navigate Overview, Systems, Network and Activity views; Settings opens a keyboard-accessible dialog.
2. Open a searchable command palette by button or Cmd/Ctrl+K; arrow-key selection, Enter, Escape, focus return. Commands navigate, start a simulated diagnostic, open chat, toggle focus and animation.
3. Start/cancel a deterministic simulated diagnostic; progress stages and terminal outcome are explicit; repeated start does not overlap. It measures no actual machine data.
4. Open a conversation panel, submit a bounded text prompt and receive a clearly scripted demo answer. Voice preview animates a listening/processing response without requesting the microphone. Unknown prompts explicitly explain the demo boundary. Close/reopen preserves local in-session messages.
5. Add/check/remove bounded local objectives and restore them on reload. Demonstration initial objectives are marked. Empty input is refused; user text rendered safely as text.
6. Customize visible widgets, cyan/amber accent, interface density and animation. Preferences persist in localStorage; corrupt or blocked storage falls back safely with a visible notice. Focus mode reduces side-panel distractions. Reset affects only this app's preferences/objectives after confirmation.
7. Real local clock and session elapsed time are distinguished from simulated CPU/memory/network instruments. Activity records user actions; initial simulation entries are marked.

## State and data boundaries
UI consumes a typed, deterministic demo model through small functions/hooks. Theme/preferences and objectives are persisted via validated, versioned localStorage data. No messages, credentials or real files stored. All timers are disposed on unmount; diagnostics and voice preview are owned/cancellable. Keep conversation callbacks and scanner state independent. Future adapters may replace the demo model without altering instrument presentation, but there is no speculative agent framework now.

## Accessibility and performance
Semantic landmarks, native buttons, named icon controls, accessible Radix dialogs, visible focus and 44px touch targets on compact screens. All animation stops when explicitly paused or OS reduced motion is active, including Canvas and CSS loops; requestAnimationFrame pauses on document hidden. Avoid per-frame React renders and large particle arrays. Essential text must meet WCAG AA contrast. No flashing sequences. All features must work without color alone.

Responsive acceptance: no horizontal overflow at 390x844, 768x1024, 1280x800, 1440x900, and 1920x1080. At wide sizes the central core dominates without swallowing controls. Smaller screens reflow and keep all navigation usable. Empty, invalid-storage, stopped scan and unknown command states are explicit.

## Acceptance and verification
- npm dependency install, strict typecheck/lint, domain tests and production build pass.
- Playwright exercises navigation, command palette including keyboard/focus, diagnostic completion/cancel, chat demo, voice simulation, objective persistence, settings persistence, reset, reduced motion and viewport overflow.
- Automated accessibility audit has no serious/critical findings in primary and modal states; manual screen-reader certification is not claimed.
- Native Tauri app builds; launch is checked on this Mac. Unsigned/ad-hoc development packaging is not notarized release certification.
- Browser output contains no uncaught runtime errors, real provider requests, microphone permission requests or remote font dependency.
- Capture actual browser screenshots for user review; automated geometry/DOM evidence is not substituted for user's aesthetic acceptance.

## Costs, risks and recovery
No paid service or hosting. Dependency download and native build consume local network/disk/CPU. WebKit rendering can differ from Chromium, so check both. Visual effects are intentionally richer than ordinary productivity apps; pause/density/focus controls mitigate fatigue. This is not an actual JARVIS intelligence. Native integration and AI need later design/permissions. No commit, push, deployment, app replacement or global runtime installation. Rollback: stop the app/dev server and archive the new project; do not delete unrelated data.
