# Local Provider Spike Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce an independently reviewable, fail-closed candidate bundle and disposable signed-helper inference spike for Qwen3.5-4B without activating it in the product.

**Architecture:** A frozen review manifest binds the candidate metadata, corpus, scorer, acquisition code, evidence contract, and tests. One lifecycle owner performs exact-origin bounded acquisition, source-tree verification, offline build, sandboxed inference, strict scoring, evidence publication, and mandatory cleanup. Product integration remains absent until a later reviewed GO.

**Tech Stack:** Python 3 stdlib, Rust/Tauri signed helper boundary, llama.cpp v0.4.1 source, GGUF, macOS sandbox-exec, unittest.

**Spec:** `docs/superpowers/specs/2026-09-20-local-provider-spike-design.md`

## Global Constraints

- No runtime source, model, or other large artifact transfer before explicit owner approval bound to the exact reviewed aggregate.
- No model retention, product activation, webview command, UI change, commit, push, deployment, or release.
- Every production behavior follows observed RED → GREEN.
- The scripted provider remains the rollback and ordinary product provider.
- Raw prompts, completions, paths, environment, stdout, and stderr never enter retained evidence.
- Any unresolved independent P0-P2 yields NO-GO and cleanup.

---

### Task 1: Strict corpus and scorer

**Files:**
- Create: `docs/spikes/004-local-provider-candidate/behavior-corpus.json`
- Create: `scripts/local_provider_spike.py`
- Create: `scripts/tests/test_local_provider_behavior.py`

**Interfaces:**
- Produces: `load_behavior_corpus`, `evaluate_response`, `render_prompt`, and `corpus_digest`.

- [x] Add RED tests for exact, JSON, term, refusal, word-bound, prompt rendering, and corpus identity.
- [x] Run `python3 -m unittest scripts.tests.test_local_provider_behavior` and observe missing-symbol RED.
- [x] Implement strict corpus parsing, deterministic rendering, and validators.
- [x] Rerun the focused suite to GREEN.

### Task 2: GGUF metadata and source identity

**Files:**
- Modify: `scripts/local_provider_spike.py`
- Create: `scripts/tests/test_local_provider_gguf.py`
- Create: `scripts/tests/test_local_provider_git_tree.py`

**Interfaces:**
- Produces: `read_gguf_metadata` and `git_tree_sha1`.

- [x] Add and observe RED for GGUF version/type/duplicate/size checks.
- [x] Implement bounded metadata parsing and reach GREEN.
- [x] Add and observe RED against an independently calculated Git tree digest.
- [x] Implement regular-file-only Git tree replay and reach GREEN.

### Task 3: Acquisition primitives

**Files:**
- Modify: `scripts/local_provider_spike.py`
- Create: `scripts/tests/test_local_provider_acquisition.py`
- Create: `scripts/tests/test_local_provider_download.py`

**Interfaces:**
- Produces: `required_free_bytes`, `validate_transfer_url`, `copy_verified`, `safe_extract_tar`, and `download_https`.

- [x] Add and observe RED for traversal, links, limits, digest drift, redirects, and partial files.
- [x] Implement no-follow exclusive publication, safe extraction, exact HTTPS policy, and redirect revalidation.
- [x] Rerun focused tests to GREEN.

### Task 4: Immutable review identity and strict evidence

**Files:**
- Create: `docs/spikes/004-local-provider-candidate/evidence-contract.json`
- Modify: `scripts/local_provider_spike.py`
- Create: `scripts/tests/test_local_provider_review_manifest.py`
- Create: `scripts/tests/test_local_provider_evidence.py`

**Interfaces:**
- Produces: `build_review_manifest`, `verify_review_manifest`, `publish_evidence`, and `validate_evidence_directory`.

- [x] Add and observe RED for manifest drift, sidecars, unknown evidence, floats, secrets, tamper, aliases, and dirty destinations.
- [x] Implement canonical review aggregation and exact-set evidence replay.
- [x] Run all local-provider Python tests to GREEN.

### Task 5: Bounded workspace, build, and inference lifecycle

**Files:**
- Modify: `scripts/local_provider_spike.py`
- Create: `scripts/tests/test_local_provider_lifecycle.py`

**Interfaces:**
- Produces: `CandidateWorkspace`, `run_bounded_process`, `verify_build_outputs`, and `probe_network_policy`.

- [ ] Add RED tests proving exclusive private roots, same-filesystem checks, bounded process timeout/output-tree cleanup, and mandatory removal.
- [ ] Implement the minimum lifecycle owner and run focused GREEN.
- [ ] Add RED network positive/denied control and exact policy errno checks.
- [ ] Implement separate build and inference sandbox profiles and run GREEN.

### Task 6: Frozen manifest and executable preflight

**Files:**
- Replace: `docs/spikes/004-local-provider-candidate/candidate-manifest.json`
- Replace: `docs/spikes/004-local-provider-candidate/candidate-manifest.json.sha256`
- Modify: `scripts/local_provider_spike.py`
- Create: `scripts/tests/test_local_provider_manifest.py`

**Interfaces:**
- Produces: `load_candidate_manifest` and CLI `preflight` / `acquire` modes.

- [ ] Add RED tests for every exact manifest field, corpus/tool hash drift, stale external aggregate, free-space shortage, mount mismatch, and unapproved acquire.
- [ ] Implement strict manifest loading and preflight without network access.
- [ ] Implement `acquire` requiring the literal reviewed aggregate and explicit approval token.
- [ ] Run focused and complete Python suites to GREEN.

### Task 7: Dormant no-model signed-helper provider adapter

**Files:**
- Modify: `src-tauri/src/helper/protocol.rs`
- Create: `src-tauri/src/signed_helper/model_provider.rs`
- Modify: `src-tauri/src/signed_helper/mod.rs`
- Test: Rust unit tests colocated with these modules.

**Interfaces:**
- Produces: a model-session state machine over an injected backend; no webview command and no llama.cpp linkage.

- [ ] Add RED tests for exact initialize/generate/cancel/shutdown transitions, one active operation, late-token rejection, limits, and redacted errors.
- [ ] Implement the provider-neutral state machine and run focused GREEN.
- [ ] Prove ordinary app startup does not instantiate or launch it.

### Task 8: Freeze and independent pre-transfer review

**Files:**
- Create: `docs/spikes/004-local-provider-candidate/review-manifest.json`
- Modify: `docs/spikes/004-local-provider-candidate/README.md`
- Modify: `.rps/STATE.md`

**Interfaces:**
- Produces: one literal externally reviewed aggregate for owner approval.

- [ ] Run Python, Rust, frontend, browser, native build/signature, and no-residue gates.
- [ ] Freeze the closed roster read-only and replay every record.
- [ ] Obtain two independent before/after reviews; fix every P0-P2 by creating a new candidate.
- [ ] Record exact approved transfer bytes, disk headroom, cleanup, and rollback.

### Task 9: Owner transfer gate and disposable certification

**Files:**
- Publish only redacted evidence under `.rps/evidence/local-provider-004/<run-id>/` after approval.
- Update: `docs/spikes/004-local-provider-candidate/README.md`
- Update: `.rps/STATE.md` and `.rps/MEMORY.md`

**Interfaces:**
- Consumes: literal independently accepted review aggregate and explicit owner approval.
- Produces: strict replayable evidence and GO-for-integration-review or NO-GO.

- [ ] Obtain explicit approval for the exact runtime/model transfer and disk impact.
- [ ] Re-run preflight in the acquisition command, then acquire source before model and stop on any source failure.
- [ ] Build offline, verify source tree/build inventory/license/architecture, then acquire and verify model/template.
- [ ] Run 20/20 behavior, cold/performance, memory-growth, UI-stall, cancellation, network, no-listener, and cleanup gates.
- [ ] Publish and replay redacted evidence, independently review exact artifacts, and delete all runtime/model/staging bytes unless retention is separately approved.
- [ ] Keep the scripted provider active unless a later product-integration review is explicitly approved.
