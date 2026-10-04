"""Closed non-circular source roster for Stage 1 owner authorization."""

from __future__ import annotations

import hashlib
import json
import os
import stat
from pathlib import Path
from typing import Any, Iterable

BINDING_SCHEMA = "jarvis.local-provider.stage1-owner-binding.v1"
REVIEW_SCHEMA = "jarvis.local-provider.review-manifest.v1"
TRUST_ANCHOR_PATH = "src-tauri/resources/stage1-owner-public-key.json"
BINDING_PATH = "src-tauri/resources/stage1-owner-binding.json"
HARNESS_REVIEW_MANIFEST_PATH = (
    "docs/spikes/005-prebuilt-runtime-candidate/harness-review-manifest.json"
)
HARNESS_REVIEW_SIDECAR_PATH = HARNESS_REVIEW_MANIFEST_PATH + ".sha256"
CANDIDATE_PATH = "docs/spikes/005-prebuilt-runtime-candidate/candidate-metadata.json"
CANDIDATE_SIDECAR_PATH = CANDIDATE_PATH + ".sha256"
CANDIDATE_REVIEW_MANIFEST_PATH = (
    "docs/spikes/005-prebuilt-runtime-candidate/review-manifest.json"
)
CANDIDATE_REVIEW_SIDECAR_PATH = CANDIDATE_REVIEW_MANIFEST_PATH + ".sha256"
UNFROZEN_BINDING_BYTES = (
    b'{"schema":"jarvis.local-provider.stage1-owner-binding.v1","state":"unfrozen"}'
)
REVOKED_REVIEW_AGGREGATES = frozenset(
    {
        "f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0",
        "380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0",
        "1c49497d67bc21f3a1a3edf49bca82be180ca517fce3380ead4b99389c9e4bc0",
    }
)

# Outputs derived from this roster are deliberately excluded to avoid circular hashes.
BASE_REVIEW_ROSTER = tuple(
    sorted(
        {
            "AGENTS.md",
            "package.json",
            "docs/spikes/005-prebuilt-runtime-candidate/README.md",
            CANDIDATE_PATH,
            CANDIDATE_SIDECAR_PATH,
            CANDIDATE_REVIEW_MANIFEST_PATH,
            CANDIDATE_REVIEW_SIDECAR_PATH,
            "docs/superpowers/plans/2026-09-20-prebuilt-runtime-stage1.md",
            "docs/superpowers/plans/2026-09-20-stage1-native-owner-authorization.md",
            "docs/superpowers/specs/2026-09-20-prebuilt-runtime-stage1-design.md",
            "docs/superpowers/specs/2026-09-20-stage1-native-owner-authorization-design.md",
            "scripts/build_native_helper.py",
            "scripts/finalize_signed_helper_bundle.py",
            "scripts/freeze_stage1_owner_authorization.py",
            "scripts/pin_stage1_owner_public_key.py",
            "scripts/prebuilt_runtime_stage1.py",
            "scripts/prepare_stage1_owner_authorization_bundle.py",
            "scripts/stage1_owner_authorization.py",
            "scripts/stage1_owner_roster.py",
            "scripts/verify_signed_helper_bundle.py",
            "scripts/verify_stage1_owner_authorization_bundle.py",
            "scripts/tests/test_freeze_stage1_owner_authorization.py",
            "scripts/tests/test_prepare_stage1_owner_authorization_bundle.py",
            "scripts/tests/stage1_owner_test_support.py",
            "scripts/tests/fixtures/stage1-owner-binding-frozen.json",
            "scripts/tests/fixtures/stage1-owner-payload.json",
            "scripts/tests/fixtures/stage1-owner-envelope.json",
            "scripts/tests/test_build_native_helper.py",
            "scripts/tests/test_pin_stage1_owner_public_key.py",
            "scripts/tests/test_prebuilt_runtime_stage1_candidate.py",
            "scripts/tests/test_prebuilt_runtime_stage1_extract.py",
            "scripts/tests/test_prebuilt_runtime_stage1_runner.py",
            "scripts/tests/test_prebuilt_runtime_stage1_transport.py",
            "scripts/tests/test_stage1_owner_authorization.py",
            "scripts/tests/test_signed_helper_bundle.py",
            "src-tauri/Cargo.toml",
            "src-tauri/Entitlements.plist",
            "src-tauri/HelperEntitlements.plist",
            "src-tauri/Info.plist",
            "src-tauri/build.rs",
            "src-tauri/native/code_identity.c",
            "src-tauri/native/owner_authorization.c",
            TRUST_ANCHOR_PATH,
            "src-tauri/src/lib.rs",
            "src-tauri/src/main.rs",
            "src-tauri/src/owner_authorization/binding.rs",
            "src-tauri/src/owner_authorization/canonical.rs",
            "src-tauri/src/owner_authorization/cli.rs",
            "src-tauri/src/owner_authorization/deletion.rs",
            "src-tauri/src/owner_authorization/issuer.rs",
            "src-tauri/src/owner_authorization/mod.rs",
            "src-tauri/src/owner_authorization/paths.rs",
            "src-tauri/src/owner_authorization/process.rs",
            "src-tauri/src/platform/mod.rs",
            "src-tauri/src/platform/owner_authorization.rs",
            "src-tauri/tests/stage1_owner_authorization.rs",
            "src-tauri/tauri.conf.json",
        }
    )
)


def canonical_json_bytes(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=True,
        allow_nan=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("ascii")


def collect_records(repo_root: Path, paths: Iterable[str]) -> list[dict[str, Any]]:
    root = Path(repo_root)
    root_info = root.lstat()
    if not stat.S_ISDIR(root_info.st_mode) or root.is_symlink():
        raise ValueError("repository root is unsafe")
    resolved_root = root.resolve(strict=True)
    records: list[dict[str, Any]] = []
    seen: set[str] = set()
    for relative in sorted(paths):
        if relative in seen or not _safe_relative(relative):
            raise ValueError("resource roster path is invalid")
        seen.add(relative)
        path = root / relative
        try:
            metadata = path.lstat()
            resolved = path.resolve(strict=True)
        except OSError as error:
            raise ValueError("resource roster member is missing") from error
        if (
            not stat.S_ISREG(metadata.st_mode)
            or path.is_symlink()
            or metadata.st_nlink != 1
            or resolved_root not in resolved.parents
        ):
            raise ValueError("resource roster member is unsafe")
        payload = path.read_bytes()
        after = path.lstat()
        before_identity = (
            metadata.st_dev,
            metadata.st_ino,
            metadata.st_size,
            metadata.st_mtime_ns,
            metadata.st_nlink,
        )
        after_identity = (
            after.st_dev,
            after.st_ino,
            after.st_size,
            after.st_mtime_ns,
            after.st_nlink,
        )
        if before_identity != after_identity:
            raise ValueError("resource roster member changed while hashing")
        records.append(
            {
                "path": relative,
                "bytes": len(payload),
                "mode": stat.S_IMODE(metadata.st_mode),
                "sha256": hashlib.sha256(payload).hexdigest(),
            }
        )
    return records


def collect_base_records(repo_root: Path) -> list[dict[str, Any]]:
    return collect_records(repo_root, BASE_REVIEW_ROSTER)


def review_aggregate(records: list[dict[str, Any]]) -> str:
    if records != sorted(records, key=lambda item: item["path"]):
        raise ValueError("review records are not sorted")
    return hashlib.sha256(canonical_json_bytes(records)).hexdigest()


def _safe_relative(value: str) -> bool:
    if not isinstance(value, str) or not value or "\\" in value or "\0" in value:
        return False
    path = Path(value)
    return (
        not path.is_absolute()
        and value == path.as_posix()
        and all(component not in ("", ".", "..") for component in path.parts)
    )
