# RPS Project Protocol — Hermes starter adaptation 0.1.0

Baseline: RPS v1.0. Added here: lightweight adoption, checkpoint memory, and
portable agent entry points. This is a workflow specification, not an RPS CLI.
A repository copy is pinned until the owner approves an update.

## Operating loop

Problem -> evidence -> feasibility -> GO/PIVOT/STOP/DEFER -> product/MVP/UX
-> requirements and quality scenarios -> architecture -> bounded changes
-> verification -> controlled release -> outcome learning -> evolution/retirement.

Return to earlier stages when evidence warrants it. Do not invent a complete
plan before learning what matters. Prefer reversible decisions. No universal
architecture, test count, or documentation volume is mandatory.

## Start every substantive task

Read `AGENTS.md`, this protocol, `.rps/STATE.md`, the canonical configuration,
and relevant requirements, decisions, active change, and code. Inspect the
current workspace. Load only relevant context; no full-history prompt dump.
When instructions conflict, surface the conflict and preserve stronger safety
and authorization requirements. The user can request a read-only task.

## Evidence discipline

Distinguish OBSERVED, SUPPORTED, INFERRED, ASSUMED, DECIDED, and UNKNOWN.
Record sources, dates/versions, context, and uncertainty where material.
Seek contradictory evidence on consequential assumptions. Check current
primary documentation before relying on volatile platform/API claims.
Do not present earlier summaries or AI outputs as verified facts.

## Project profiles

P0: disposable, no production data/users. Hypothesis, time box, result,
explicit disposal or promotion decision.
P1: genuinely low-impact work. Basic outcome, requirements, verification,
and data recovery as needed.
P2: users or work rely on it. Requirements, risk-driven tests, security,
privacy, accessibility, release/recovery, observability, and ownership.
P3: high-impact or regulated. Add domain-specific professional assurance,
independent review, and relevant legal/safety controls.

Record a provisional profile when evidence is incomplete. Do not silently
promote a prototype into production or reduce rigor without justification.

## Plan before significant commitment

For ideas, research users, actual behavior, current alternatives, value,
feasibility, distribution, costs, operations, and relevant harms/constraints.
Test the most consequential unknown first. Set success and stop conditions.

For the MVP, select a narrow complete journey that tests value, not a large
feature checklist. Cover important error/offline/permission/recovery states.
Turn vague qualities into measurable scenarios with justified thresholds.

For architecture, identify ownership, invariants, contracts, dependencies,
data lifecycle, and failure modes. Modular does not mean maximum file or
service count. Record consequential decisions and their trade-offs.

## Implement proportionately

Cosmetic or truly low-risk work: a small patch and relevant check; document
only when behavior/meaning changes. Watch for deceptively high-risk text or
configuration changes.

Ordinary change: one concise change record with goal, scope, non-goals,
acceptance, tests, and outcome.

Significant/high-risk change: evidence, explicit plan, contracts/migrations,
risk/failure checks, independent review where appropriate, rollout/recovery,
and approval at the relevant authority boundary. Use the six-file RPS packet
when useful, not as ceremony for every edit.

Prefer independently verifiable vertical slices. Keep side effects explicit,
protect invariants, follow repository conventions, avoid speculative layers,
and measure before optimizing. Preserve others' edits and task boundaries.

## Verification and permission

Use actual repository/toolchain commands discovered and documented in the
canonical configuration. Missing commands are a setup gap, not an invitation
to claim verification. Disclose partial, failed, unavailable, skipped, and
historical checks. Do not disable controls merely to make checks pass.

Review the relevant diff and requirements. Security-sensitive logic, payments,
authorization, destructive changes, migrations, and releases require stronger
review proportionate to risk. Tests and a second AI are evidence, not proof.
Do not automatically commit, push, deploy, spend, change permissions, or delete
user data unless authorized for that action.

## Memory and handoff

STATE is a short current index: goal, stage, profile, constraints, canonical
paths, active change, actual verification, blockers, and next safe action.
MEMORY is durable learning: rejected options, important failed experiments,
traps, and reasons, each linked to evidence/decisions where possible.
Decision records contain why, alternatives, consequences, owner, and status.
Change outcomes contain intended vs actual behavior and verification.
Git contains implementation history; never duplicate raw diffs into memory.

Checkpoint after meaningful milestones and before handing off or ending an
unfinished task when possible. Include failed checks and unverified changes.
Do not invent past dates, approvals, completed tests, or released status.
Preserve history through explicit superseding decisions rather than pretending
an old decision never happened. Keep only decision-relevant knowledge; do not
store secrets, unnecessary personal data, or raw internal reasoning.

## Completion is more than code

Report behavior, verification, limitations, risks, and next action. Mark
implementation/verification/review/release separately. Synchronize relevant
requirements, decisions, operations, and memory when their meaning changes.
Do not call work done merely because files exist or compilation succeeded.

## Continuation check

A fresh agent with repository access should identify purpose, users, stage,
constraints, architecture decisions, active work, failed/unrun checks, and
next action from file evidence, without guessing. Check the current working
tree too: memory describes a checkpoint, not guaranteed live state.

## Evolution

Propose protocol changes with rationale and version. Apply them to a project
only with owner approval; existing projects retain their pinned version.
Documents are instructions, not enforcement. Real controls remain permissions,
review, tests, and release policy. No guarantee of flawless memory is made.
