# Prebuilt runtime stage-1 implementation plan

1. Add strict candidate/review replay and purpose-bound preflight; test unknown fields, stale hashes, wrong capability state, wrong volume, and insufficient disk.
2. Add stdlib OCI challenge/token/download transport; test realm/service/scope, exact origins/ports, one redirect, Authorization stripping, Content-Length, byte ceilings, and digest mismatch.
3. Add descriptor-relative bottle extraction; test traversal, aliases, duplicate/case/Unicode names, special files, PAX/sparse metadata, link escape/cycles, mode policy, entry and expanded-byte limits.
4. Add relocation and Mach-O/license inventory without running package-manager or bottle code; test malformed/fat Mach-O, unknown load paths, missing dependencies, symlink graph, and license closure.
5. Add strict stage-1 receipt publication/replay and handle-based cleanup; test contradictory verdicts, raw-data leakage, dirty destinations, symlink swaps, interruption, and residue.
6. Freeze exact harness/tests/spec/candidate bytes, run all local gates, obtain two independent reviews, then request owner approval for bottle transfer only.
7. After approved acquisition, freeze relocated bytes and require a new review plus separate model-transfer approval.

Every behavior follows RED → minimal GREEN → focused regression. No runtime/model payload is downloaded while executing this plan.

## Remediation outcome

Items 1–5 are implemented with focused RED/GREEN regressions plus an injected-transport end-to-end test. Aggregates `f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0` and `380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0` are rejected historical candidates and cannot authorize transfer. The second remediation adds app-owned ledger/workspace identity binding, cross-process locking, registered crash recovery, closed-roster/executing-source replay, externally digested ledger-bound receipts, per-layer extraction identities, one-read-per-inode inspection, canonical dylib paths, and verified CodeDirectory hashes. Item 6 remains gated on a new exact freeze, two independent reviews, and explicit owner approval; item 7 remains blocked because no bottle transfer has occurred.
