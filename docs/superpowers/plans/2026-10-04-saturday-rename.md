# Saturday Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the active product and repository identity with Saturday and prepare a clean, reviewed source snapshot for a public GitHub repository.

**Architecture:** Rename the frontend/package, Tauri/Cargo, helper, storage, and security namespaces in dependency order; retain the existing Tauri permission and signing boundaries. Preserve historical proof intact on disk and exclude uneditable old-name evidence from the public initial snapshot rather than changing its hashes or signatures.

**Tech Stack:** React, TypeScript, Vite, Tauri 2, Rust/Cargo, Python scripts/tests, npm, GitHub CLI/Git.

**Spec:** `docs/superpowers/specs/2026-10-04-saturday-rename-design.md`

## Global Constraints

- The product name is `Saturday`.
- No old-name aliases or local-data migration; old application storage and authorization identity are not migrated.
- No fixture helper, Tauri invoke handler, plugin permission, network capability, or privilege is added.
- Keep historical manifests, signatures, verification output, and records byte-for-byte unchanged; do not include old-name historical records in the public initial commit.
- Exclude build outputs, archives, binaries, credentials, and private local data from the public repository.
- Do not publish until the exact GitHub account, repository name, public visibility, and staged file list have been confirmed.

## Review Focus

- Existing tests may depend on serialized protocol strings; update all producers, consumers, and independent fixtures together, then run the full Rust/Python suites.
- New bundle ID must isolate Saturday app storage and Keychain tags; verify exact new value and that no old identity is silently accepted.
- Helper source-generation hashes must include renamed source paths and produce deterministic current-source identities.
- Lockfiles must agree with the new package/crate names without dependency-version drift.
- Public staging must omit historical evidence that cannot be truthfully relabeled and must contain no secrets or private local data.

---

### Task 0: Preserve pre-rename recovery copy

**Files:**
- Create: `/tmp/jarvis-dashboard-before-saturday.tar.gz` (local recovery archive; never stage or publish)

- [x] **Step 1: Archive the current project before changing active identities**

Run: `tar -czf /tmp/jarvis-dashboard-before-saturday.tar.gz --exclude='jarvis-dashboard/.git' --exclude='jarvis-dashboard/node_modules' --exclude='jarvis-dashboard/src-tauri/target' -C ~/Documents/GitHub jarvis-dashboard`
Expected: archive exists and contains source/configuration/docs without Git internals or regenerable dependency/build caches.

- [x] **Step 2: Verify archive integrity and representative members**

Run: `tar -tzf /tmp/jarvis-dashboard-before-saturday.tar.gz`
Expected: listing includes `jarvis-dashboard/src-tauri/Cargo.toml`, `jarvis-dashboard/src/main.tsx`, and project docs.

### Task 1: Frontend and web package identity

**Files:**
- Modify: `package.json`, `package-lock.json`, `index.html`
- Modify: `src/components/Chrome.tsx`, `Conversation.tsx`, `Core.tsx`, `MicrophoneSelect.tsx`, `Modal.tsx`, `BuiltInModules.tsx`, `SocialStudio.tsx`
- Modify: `src/domain/preferences.ts`
- Modify: `tests/**/*.spec.ts`, relevant `src/**/*.test.ts`
- Modify: active README and current docs that describe the active UI (Task 3 owns documentation edits)

**Interfaces:**
- Produces package name `saturday`; app title, page metadata, UI text, and active localStorage key namespaces use Saturday.
- Old localStorage entries are not read, migrated, or retained as aliases.

- [x] **Step 1: Pin changed branding/storage behavior in tests**

Add/update consumer-visible checks for the Saturday document title, active UI identity, and newly namespaced workspace/social-studio persistence; assert that old keys are not read.

- [x] **Step 2: Run the focused frontend test**

Run: `npm run test:e2e -- tests/dashboard.spec.ts tests/workspace.spec.ts tests/social-studio.spec.ts --project=chromium`
Expected: fail on the former document title/heading and storage keys.

- [x] **Step 3: Rename frontend package and active UI identifiers**

Update exact active copy, page metadata, package name and lockfile root package name, and localStorage keys. Do not alter product behavior or dependencies.

- [x] **Step 4: Verify frontend**

Run: `npm run check`.
Expected: lint, tests, typecheck, and Vite build pass with package name `saturday`.

- [x] **Step 5: Commit the verified frontend slice**

Run: `git add package.json package-lock.json index.html src tests && git commit -m "feat: rename frontend identity to Saturday"`
Expected: one local commit on the feature branch; no push.
### Task 2: Native, helper, and authorization identity

**Files:**
- Rename: `src-tauri/src/bin/jarvis-runtime-helper.rs`, `src-tauri/src/bin/jarvis-helper-fixture.rs` to Saturday-derived filenames.
- Modify: `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, Rust source under `src-tauri/src/`, Rust tests and fixtures.
- Modify: `scripts/build_native_helper.py`, `scripts/finalize_signed_helper_bundle.py`, `scripts/freeze_stage1_owner_authorization.py`, `scripts/pin_stage1_owner_public_key.py`, `scripts/prepare_stage1_owner_authorization_bundle.py`, `scripts/stage1_owner_authorization.py`, `scripts/stage1_owner_roster.py`, `scripts/prebuilt_runtime_stage1.py`, verification scripts, and Python tests/fixtures.

**Interfaces:**
- Cargo package/crate and executable/helper/resource names use `saturday` / `saturday-runtime-helper` consistently.
- Tauri `productName` is `Saturday`; bundle identifier is `dev.rohtolos.saturday`.
- Application-support path, Secure Enclave tag, protocol schemas, temp paths, environment variables, and signature domain separators use Saturday-derived values; old values are rejected.
- Preserve all existing security boundaries and protocol field structure except the identity/schema/domain values being renamed.

- [x] **Step 1: Update security and packaging regression expectations**

Update focused Rust/Python tests to require exact Saturday names and identifiers, reject old names, and verify helper source-generation includes renamed paths. Preserve unrelated historical evidence; rebaseline active candidate fixtures/manifests only under the owner's explicit approval, with a fresh review required before publication.

- [x] **Step 2: Run focused Rust and Python tests; observe expected failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` and `npm run test:stage1-owner-authorization`; also run the relevant helper/prebuilt-runtime Python test commands from `package.json`.
Expected: failures at stale identity expectations and stale active protocol fixtures only.

- [x] **Step 3: Rename native identities and all dependent callers**

Rename Cargo package, crate import paths, bin targets/files, Tauri product/bundle/resource settings, support/tag/schema/domain values, helper environment names, and all active scripts/tests. Keep signatures and permission surface unchanged. Regenerate only derived assets through existing scripts; never synthesize a signing identity.

- [x] **Step 4: Verify native security and packaging contracts**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`, all documented feature-specific Rust tests, all owner-authorization/helper Python suites, `cargo fmt --manifest-path src-tauri/Cargo.toml --check`, and `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`.
Expected: all pass; the old identity is rejected; the fixture helper remains excluded from production configuration.
- [x] **Step 5: Commit the verified native slice**

Run: `git add` only native code/configuration, active fixtures, generated-source metadata, and plan progress; then create one local native-rename commit.
Expected: native identity slice is locally committed; no push.

### Task 3: Active documentation, repository name, and public snapshot

**Files:**
- Modify: `README.md`, `AGENTS.md`, `.rps/PROJECT.yaml`, current `docs/` pages and package references.
- Modify: `.gitignore` or explicit staging exclusions for immutable historical docs/evidence and local-only records.
- Rename local project directory from `jarvis-dashboard` to `Saturday` after all processes using it are stopped.

**Interfaces:**
- Active public project documentation and app instructions refer only to Saturday.
- Historical records remain unchanged locally and excluded from the public initial commit when their old identity is integral to their evidence.

- [x] **Step 1: Update active documentation and project metadata**

Document the new identity and explicit absence of old-data migration; keep historical facts marked historical and unmodified. Ensure local instructions use the new directory/package/bundle paths.

- [x] **Step 2: Verify repository-wide active identity and privacy exclusions**

Run targeted case-insensitive searches over active source/config/docs; inspect all search hits and justify retained generic/technical tokens. Confirm `.gitignore` excludes build outputs and artifacts and historical evidence selected for exclusion.

- [x] **Step 3: Review staged public file list and credentials**

Run the project credential scan and inspect the complete proposed staged file list. Expected: no secret/private-data findings, no generated output or packaged binaries, no historical evidence containing the previous brand.

- [x] **Step 4: Commit active documentation and local project metadata**

Run: create a local documentation/project-metadata commit after reviewing the exact changed files; no push.
Expected: current local docs name Saturday; historical records selected for exclusion remain unchanged.

- [x] **Step 5: Rename the local repository directory**

Run: stop app/server processes, remove the committed nested worktree, rename the repository directory to `Saturday`, check out the rename branch, and verify the repository path.
Expected: active checkout is `~/Documents/GitHub/Saturday`; the old local checkpoint remains recoverable and is never pushed.
### Task 4: Full verification and release preparation

**Files:**
- Modify: current verification record and README verification commands/results.

- [x] **Step 1: Run full project checks**

Run the package's complete frontend, browser, Python, Rust, format, Clippy, and native build checks as documented in `README.md`; perform a smoke launch of the rebuilt Saturday app and inspect its displayed title.

- [x] **Step 2: Verify exact names and output metadata**

Inspect built package metadata and `Info.plist` for `Saturday` / `dev.rohtolos.saturday`; confirm generated artifacts are ignored and build output size is measured but not staged.

- [x] **Step 3: Run independent review and resolve findings**

Request a fresh review of the rename/security-identity diff and the exact staged tree. Fix confirmed issues, rerun impacted tests, and keep historical evidence unaltered.

- [x] **Step 4: Confirm GitHub destination and publication authorization**

Discover authenticated owner; ask the user to confirm the exact `owner/Saturday` destination, public visibility, and staged file list immediately before repository creation/push.

- [x] **Step 5: Create initial commit and publish**

After confirmation, create the initial commit, create the public GitHub repository with the exact approved name, push the requested branch, and verify the repository URL and pushed commit through GitHub.
