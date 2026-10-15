# Saturday
Read `ARCHITECTURE.md` and any directly relevant active design or contract docs before substantial changes.

- Native bilingual voice is implemented in source but remains an unverified runtime until integrated native builds, reviewed worker/model artifacts, owner-personal trust setup, physical audio/inference/speech checks, signing and installed-app proof pass. Browser preview has no assistant, microphone or speech fallback; the test-only protocol driver is not native readiness evidence.
- Standalone project; never modify Vector or other repositories as part of this app.
- React/TypeScript/Vite/Motion frontend, Tauri 2 desktop shell. Custom CSS and SVG/Canvas visuals, bundled fonts, no runtime CDN.
- Test behavior before implementation. Respect reduced motion, keyboard navigation, focus, responsive layouts and local-storage failure. Voice protocol tests must exercise consumer behavior through the explicit browser-only native protocol driver, never browser media APIs or fake product grants.
- Keep visual components separate from domain data and persistence. The approved modular direction starts with statically bundled trusted modules; runtime third-party plugins stay disabled until the security gate in `ARCHITECTURE.md` is satisfied.
- Commit, publish, deploy, install system-wide, or invoke paid/external services only with explicit maintainer approval.
- Keep this file and the README aligned with actual project behavior, repository contents, and verification commands.
