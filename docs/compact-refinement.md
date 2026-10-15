# Compact interface and continuous animation refinement

> **Superseded UI scope:** A later user-approved cutover removes Social Studio, Workspace (Tasks/Diagnostics/Activity), and Settings. The retained surface is the core/composer, command palette, Conversation, explicit microphone visualization, and motion preference. Existing local-storage records are not deliberately deleted or schema-migrated. This document records the earlier Workspace-drawer revision; see the current [README](../README.md).

## Authority and scope
User reported clutter and animation glitches, requested a compact/minimal UI and improved animation quality, and approved **compact core + Workspace drawer**. This is a bounded frontend revision, not a new architecture, connected assistant feature, release, or data deletion. Saved objectives/settings must survive. No repeated design-document approval cycle.

## Decision
Keep the core continuously mounted. Quiet header, compact core, actual command input. Put Tasks, Diagnostics and Activity in a Workspace drawer; retain settings and keyboard command palette. Remove the main-screen telemetry/map/environment panels, coordinates, repeated labels and sidebar. Preserve useful data, not decorative fake dashboards.

## Reproduced defects, before changes
Chromium and WebKit returned the same result: at a frozen currentTime=6000 ms, changing state changed inner-ring duration 28000→4000 ms and progress 0.21428571428571427→0.5. Pausing removed all ring animations and made the old animation idle; resuming created a different animation at time/progress zero. Navigating Systems→Overview replaced the canvas node.

## Implementation and verification
1. Add failing regressions for continuity, compact primary surface, stable core lifetime and retained drawer flows.
2. Use persistent Web Animations API transform timelines on HTML carriers for rings; pause/play preserves time; updatePlaybackRate changes speed without seeking. Ease rate changes with a short timestamp-based ramp. Remove duration-swapping CSS and decorative loops.
3. Precompute static particle geometry, keep phase across theme/pause changes, avoid per-frame DOM writes and repeated per-vertex trigonometric setup. Honor reduced motion and page visibility without a restart on return.
4. Rework only presentation/controls; keep existing simulation/conversation/persistence boundaries and safeguards.
5. Verify idle/active/pause/resume/theme/dialog/visibility behavior in Chromium and WebKit; reduced motion; keyboard focus; responsive sizing; offline behavior; build native candidate and preserve rollback archive.

## Sources and scope
- https://motion.dev/docs/performance — prefer transform/opacity and composited motion; do not claim hardware acceleration is guaranteed.
- https://developer.mozilla.org/en-US/docs/Web/API/Window/requestAnimationFrame — timestamp-based progress and hidden-document suspension.
- https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/animation-play-state — pause/resume rather than recreation.
- https://developer.mozilla.org/en-US/docs/Web/API/Animation/updatePlaybackRate — change rate while preserving playback position.
The search backend returned primary documentation. The separate Firecrawl extractor failed authorization and a browser documentation fetch timed out; no paid/key-gated service was installed or represented as used.
Codebase Memory MCP initially indexed 244 nodes/460 edges, with Core.tsx line 294 marked parse-partial and read directly. A post-implementation refresh indexed 257 nodes/488 edges with no parse-partial files. Tests/generated assets remain excluded by design. Exact source and live probes, not graph completeness, ground the diagnosis.

## Outcome
Implemented and verified as local v0.2.0. See [current verification](verification-compact.md). A final full rerun exposed an exiting-dialog Escape race; a timing-controlled public-DOM regression reproduced it for Workspace, Settings and Conversation. Escape now follows the active app panel rather than the retiring dialog layer. The full 62-test browser suite and an additional 40-run timing/keyboard stress check pass. No saved workspace schema migration or data deletion was introduced.
