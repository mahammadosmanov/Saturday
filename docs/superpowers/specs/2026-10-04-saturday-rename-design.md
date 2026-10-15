# Saturday project rename design

> **Later storage/UI decision:** A subsequent user-approved cutover removes Social Studio, Workspace, and Settings, while preserving existing Saturday local-storage records without deliberate deletion or schema migration. That supersedes this design's local-data replacement assumptions only; the identity-renaming history below remains historical. See the current [README](../../../README.md).

## Goal

Rename the project identity from J.A.R.V.I.S / `jarvis-dashboard` to Saturday throughout the active project and prepare a clean initial commit for a public GitHub repository.

## Scope

- Change user-visible product copy, app display name, package/crate/binary names, repository metadata, local workspace directory, bundle identifier, helper/resource paths, test names/fixtures, scripts, and current maintainer documentation to Saturday.
- Replace old persistent-storage, Keychain/application-tag, protocol-schema, and cryptographic domain identifiers rather than retaining compatibility aliases. Existing local data and owner-authorization material under old identifiers will not be migrated; the renamed app starts under new identifiers. This follows the owner's explicit request for a literal name replacement.
- Update all dependent references and tests. Regenerate any current-source-derived identity assets only through existing documented project scripts; do not invent signing keys or claim notarization.
- Keep historical evidence trustworthy: do not edit bytes inside frozen manifests, signatures, or prior verification output to make them appear to describe Saturday. Exclude historical records that contain the old brand from the public initial snapshot if they cannot be safely relabeled without invalidating their evidence; leave the local originals unchanged.
- Keep generated build output and packaged binaries excluded from publication. Scan the exact publishable tree for credentials and private data before staging.

## GitHub publication

The current directory is a Git repository on `feat/cinematic-dashboard` with no commits and no remote. Confirm the authenticated GitHub owner and intended `Saturday` repository name before publication. Create the repository with public visibility only after reviewing the exact staged file list and confirming no credentials or private material are included. Create an initial commit and push the branch; do not publish application binaries as part of this source-repository rename.

## Acceptance

- Active source/configuration/documentation and the local repository name use Saturday, with no old branded identifiers in files included in the public initial commit.
- Existing format/build/test contracts are updated and pass; new bundle/package metadata reflects Saturday.
- Old identifiers are not silently accepted as aliases. Existing old app data and authorization identity are explicitly documented as not migrated.
- The exact staged tree passes the credential/privacy review; public repository URL and commit are reported only after GitHub confirms creation and push.

## Risks and recovery

Changing app/bundle/keychain namespaces and signed protocol domain values can make existing local state, provisioned authorization keys, and old signatures inaccessible or invalid. Preserve the current local tree as a backup before replacing names. Recovery is by restoring the pre-rename project and old identifier set; automatic migration is out of scope because it would reintroduce old-name compatibility strings.
