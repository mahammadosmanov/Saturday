# Spike 002 — Private helper boundary foundation

Date: 2026-09-19
Status: **GO for the non-shipping experiment harness; NO-GO for a production helper or model integration**

## Question

Can J.A.R.V.I.S own a deterministic local helper through a strict, offline, resource-bounded, replayable boundary without exposing native authority to the webview or shipping the test fixture?

## Implemented boundary

- strict 32-bit length-prefixed JSON with exact outer and nested schemas;
- unpredictable run/generation challenges and request ownership;
- manifest-bound executable size/hash, fixed policy/budget IDs, private sealed candidate roots, and immediate inode/hash revalidation;
- default macOS sandbox denial of network and process forks plus a one-process kernel limit;
- bounded nonblocking response delivery sized for the full valid 256-chunk protocol burst;
- independent 10 ms process-group resource monitor, fixed output/chunk/stderr/RSS/process/time budgets, cancellation acknowledgement, late-frame rejection, and bounded shutdown/drop;
- sealed lifecycle receipts that callers cannot fabricate;
- descriptor-relative, bounded, strict evidence parsing/publication with owner/mode/link checks, empty-directory requirement, inventory commit marker, and cross-run replay rejection;
- feature-gated deterministic fixture and RAII cleanup on success, error, and unwind.

## Adversarial coverage

The integration suite covers normal operation, maximum valid burst, response flood, exact outbound-denial errno with an unsandboxed success control, exact no-fork denial with a control, cancellation and late output, ignored cancellation, descendant cleanup in a test-only fork profile, malformed/crashed helpers, bounded/continuous stderr, aggregate output/chunk overflow, persistent/transient process spikes, transient RSS, manifest-bound evidence replay, dirty evidence directories, and cleanup.

## Independent review

Three review rounds were used. Earlier rounds found executable mutation gaps, fabricable/replayable evidence, permissive nested JSON, evidence TOCTOU, missing budgets, response-queue deadlock/rejection, process-group escape, post-hoc resource checks, default-test leakage, and weak denial evidence. Verified P1/P2 findings were corrected and regression-tested.

The final independent acceptance review returned **PASS** with:

- default Rust tests: 24 passed; fixture suite skipped;
- all-feature Rust tests: 24 unit + 13 helper-boundary tests passed;
- Rust formatting and Clippy with warnings denied: passed;
- no new temporary roots or helper processes;
- candidate review hashes stable.

## Product containment

- The fixture is absent from the release app.
- The Tauri command surface remains empty.
- No model/runtime, user-data access, credential, network capability, or UI route was added.
- The scripted offline conversation provider remains active.

## Remaining production blocker

macOS `sandbox-exec` launches by pathname. Even after held-descriptor/inode/hash revalidation and a sealed private root, this cannot eliminate a malicious same-UID race before `exec`. Therefore this harness does **not** certify production executable identity.

The next slice must use an app-owned signed helper and verify its designated requirement, hardened runtime, entitlements, installation/removal lifecycle, and rollback before any model is integrated.

## Rollback

Remove `src-tauri/src/helper/`, the feature-gated fixture binary and integration test, helper-specific Cargo dependencies/configuration, and this spike/spec/plan. No user data or migration is involved.
