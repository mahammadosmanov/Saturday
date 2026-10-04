# Signed app-owned helper design

Date: 2026-09-19
Status: approved implementation slice under the owner's foundation-first roadmap and repeated instruction to continue without intermediate approval stops

## Scope

Build a dormant, app-owned native helper candidate and prove mutual code-identity checks plus strict framed IPC without exposing any helper command to the webview.

This slice may:

- add a production helper binary with health, cancellation, and shutdown messages only;
- bundle that helper as Tauri nested code;
- add a native-only signed-bundle self-test mode;
- compile a small macOS Security.framework bridge;
- generate reproducible helper identity metadata during the build;
- use the installed Apple code-signing identity for a local signed-candidate verification run.

This slice may not:

- add a Tauri invoke handler or JavaScript shell permission;
- launch the helper during ordinary app startup;
- add a model, model download, inference, microphone capture, file access, network access, secrets, persistence, plugins, automation, or UI changes;
- describe an Apple Development signature as Developer ID distribution or notarization;
- commit, push, deploy, or publish an archive.

## Decision

Use a Tauri `externalBin` nested executable in `Contents/MacOS`, but do not use `tauri-plugin-shell`. The native Rust parent owns process creation and stdio. The helper and parent mutually require a valid non-ad-hoc Apple team identity before the parent sends any request payload.

The other approaches are deferred:

1. **In-process runtime:** rejected because crashes and memory corruption would share the trusted UI process.
2. **Unauthenticated sidecar path launch:** rejected because pathname and same-UID replacement races remain.
3. **XPC service now:** strongest platform direction, but introduces a Swift/Objective-C service bundle and a larger bridge before the minimal identity contract is proven. XPC remains the upgrade path for privileged automation and untrusted plugins.

## Trust model

The signed candidate consists of:

- the outer `J.A.R.V.I.S.app` signature;
- the nested `jarvis-runtime-helper` signature;
- the outer resource seal over the nested executable;
- a generated helper identity document sealed into the app;
- a source-generation identifier embedded in the helper;
- a parent-held `Child` handle and child PID;
- mutual running-process team checks before protocol traffic;
- the strict versioned framed protocol already proven by the non-shipping harness.

A PID is accepted only for the directly spawned, still-live child that the parent owns. The helper checks its immediate parent at process start. Neither side accepts a PID supplied through IPC. This avoids the arbitrary-PID trust pattern while keeping the first production candidate smaller than XPC.

## macOS identity bridge

A small C bridge links Security.framework and CoreFoundation. It exposes one narrow function:

```text
jarvis_process_matches_current_team(pid) -> match | mismatch | system error
```

The bridge:

1. obtains `SecCodeRef` for the current process;
2. obtains dynamic `SecCodeRef` for the live peer PID;
3. performs strict validity checks on both;
4. reads `kSecCodeInfoTeamIdentifier` from each;
5. rejects missing/empty team identifiers, including ad-hoc signatures;
6. compares exact team strings;
7. releases every Core Foundation object on every path.

The API never returns the team identifier to JavaScript, logs, evidence, or persisted files.

## Helper behavior

The helper:

- validates its parent identity before reading stdin;
- clears ambient environment except the fixed locale and temporary home supplied by the native supervisor;
- uses only length-framed stdin/stdout;
- accepts `Hello`, `Probe::Health`, `Cancel`, and `Shutdown`;
- rejects network probes and all unknown messages;
- emits no raw environment, path, credential, prompt, or file content;
- never opens a listener, filesystem path, model, microphone, network socket, shell, or child process;
- exits on parent EOF, protocol violation, identity mismatch, deadline, or shutdown.

The build embeds a source-generation identifier derived from the helper source, shared protocol source, Cargo lockfile, and build-script version. The generated identity file records that generation and the built helper SHA-256. Binary hashes remain evidence; the Apple team signature is the authorization anchor.

## Parent behavior

Ordinary app startup remains unchanged and does not launch the helper.

A native-only command-line self-test mode performs the bounded candidate test:

1. locate the nested helper under the current `.app` bundle;
2. reject symlinks, non-Mach-O files, aliases outside the bundle, and writable nested code;
3. verify the helper SHA-256 against the sealed generated identity document;
4. spawn it in a new session with piped stdio and the no-network/no-fork sandbox policy;
5. validate the live child against the current app team before writing `Hello`;
6. complete handshake, health probe, cancellation, and shutdown;
7. verify no descendants, no late frames, bounded stderr, and zero surviving process-group members;
8. print one fixed success token or a redacted error code, then exit.

The mode is not a Tauri command, not reachable from JavaScript, and performs no privileged assistant action.

## Build pipeline

A deterministic Python build helper:

1. requires the target triple from Tauri's hook or `rustc --print host-tuple`;
2. computes the source-generation identifier from an explicit allowlist;
3. builds only `jarvis-runtime-helper` with that identifier supplied at compile time;
4. verifies the result is Mach-O for the requested architecture;
5. computes its SHA-256;
6. copies it to `src-tauri/binaries/jarvis-runtime-helper-<target-triple>` through a temporary file and atomic rename;
7. writes generated identity JSON through the same bounded atomic flow;
8. refuses symlinks, unsupported target triples, extra source inputs, and missing lockfiles.

Generated binaries and identity files are ignored by Git. Tauri bundles the external binary and identity resource. A signed-candidate build supplies `APPLE_SIGNING_IDENTITY` only in the local verification environment; no certificate name, hash, key, password, or team identifier is committed.

## Release checks

The candidate passes only when all are true:

- default and all-feature Rust tests pass;
- frontend checks and 102 browser tests pass;
- no Tauri invoke handler or JavaScript sidecar permission exists;
- the helper is absent from ordinary source-control artifacts and present exactly once in the app bundle;
- `codesign --verify --deep --strict` passes for the app;
- strict verification passes for the nested helper;
- app and helper carry the same non-empty TeamIdentifier in the explicitly signed candidate;
- the outer seal fails after a copied candidate is tampered, then passes again only after rebuilding;
- ad-hoc candidate self-test fails closed;
- signed candidate self-test passes and leaves no helper process or temporary root;
- ordinary app launch still succeeds and does not spawn the helper;
- independent review reports no unresolved P0, P1, or P2 in this bounded slice.

## Known remaining limits

Apple Development signing proves the local architecture but is not Developer ID distribution and is not notarization. The self-test uses a directly owned child PID rather than an audit token; this is acceptable only for the parent-created, live, blocked-before-input child. Privileged automation and untrusted plugin services still require XPC/audit-token authentication or an equivalent OS broker before release.

A real model remains separately gated by candidate quality, lifecycle evidence, a new large-download approval, and successful completion of this signed-helper slice.

## Rollback

Rollback deletes the production helper binary, Security.framework bridge, generated-build script, finalizer, verifier, all associated Python/Rust tests, all three helper-related package commands, generated artifact ignores, `externalBin` and resource entries, helper entitlement file, native self-test mode, and signed-helper tests. It also restores `beforeDevCommand` to `npm run dev` and `beforeBuildCommand` to `npm run build` so Tauri no longer calls the removed native-helper command. The existing non-shipping fixture harness and minimal Tauri shell remain intact.