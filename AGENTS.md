# Saturday
Read `ARCHITECTURE.md` and any directly relevant active design or contract docs before substantial changes.

- Local-first personal assistant in phased development. Current behavior remains a compact UI with user-triggered microphone input solely for on-device loudness/spectrum animation. New assistant capabilities require bounded feature contracts, least-privilege capability design, executable verification, and explicit authority for consequential actions. No recording, transcription, AI, upload, real telemetry, filesystem mutation, automation, or external integration is active merely because it appears on the roadmap.
- Standalone project; never modify Vector or other repositories as part of this app.
- React/TypeScript/Vite/Motion frontend, Tauri 2 desktop shell. Custom CSS and SVG/Canvas visuals, bundled fonts, no runtime CDN.
- Test behavior before implementation. Respect reduced motion, keyboard navigation, focus, responsive layouts and local-storage failure.
- Keep visual components separate from domain data and persistence. The approved modular direction starts with statically bundled trusted modules; runtime third-party plugins stay disabled until the security gate in `ARCHITECTURE.md` is satisfied.
- Commit, publish, deploy, install system-wide, or invoke paid/external services only with explicit maintainer approval.
- Keep this file and the README aligned with actual project behavior, repository contents, and verification commands.
