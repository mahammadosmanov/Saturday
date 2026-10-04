#!/usr/bin/env python3
"""Verify and atomically prepare exact Stage 1 bundle resources."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import stat
import sys
import uuid
from pathlib import Path
from typing import Any

_SCRIPT_ROOT = Path(__file__).resolve().parents[1]
if str(_SCRIPT_ROOT) not in sys.path:
    sys.path.insert(0, str(_SCRIPT_ROOT))

try:
    from scripts import freeze_stage1_owner_authorization as freezer
    from scripts.stage1_owner_roster import (
        BASE_REVIEW_ROSTER,
        BINDING_PATH,
        BINDING_SCHEMA,
        CANDIDATE_PATH,
        CANDIDATE_REVIEW_MANIFEST_PATH,
        CANDIDATE_REVIEW_SIDECAR_PATH,
        CANDIDATE_SIDECAR_PATH,
        HARNESS_REVIEW_MANIFEST_PATH,
        HARNESS_REVIEW_SIDECAR_PATH,
        TRUST_ANCHOR_PATH,
        UNFROZEN_BINDING_BYTES,
        canonical_json_bytes,
    )
except ModuleNotFoundError:
    import freeze_stage1_owner_authorization as freezer
    from stage1_owner_roster import (
        BASE_REVIEW_ROSTER,
        BINDING_PATH,
        BINDING_SCHEMA,
        CANDIDATE_PATH,
        CANDIDATE_REVIEW_MANIFEST_PATH,
        CANDIDATE_REVIEW_SIDECAR_PATH,
        CANDIDATE_SIDECAR_PATH,
        HARNESS_REVIEW_MANIFEST_PATH,
        HARNESS_REVIEW_SIDECAR_PATH,
        TRUST_ANCHOR_PATH,
        UNFROZEN_BINDING_BYTES,
        canonical_json_bytes,
    )

GENERATED_RELATIVE = Path("src-tauri/generated/stage1")
PUBLIC_KEY_SCHEMA = "saturday.local-provider.stage1-owner-public-key.v1"
UNPROVISIONED_ANCHOR_BYTES = canonical_json_bytes(
    {"schema": PUBLIC_KEY_SCHEMA, "state": "unprovisioned"}
)
BOOTSTRAP_MARKER_BYTES = canonical_json_bytes(
    {
        "issue_available": False,
        "schema": "saturday.local-provider.stage1-bundle.v1",
        "state": "bootstrap-only",
    }
)
BOOTSTRAP_FILE_MAP: dict[str, str | None] = {
    "bootstrap-only.json": None,
    "stage1-owner-binding.json": BINDING_PATH,
    "stage1-owner-public-key.json": TRUST_ANCHOR_PATH,
}
FINAL_FILE_MAP: dict[str, str] = {
    "candidate-metadata.json": CANDIDATE_PATH,
    "candidate-metadata.json.sha256": CANDIDATE_SIDECAR_PATH,
    "harness-review-manifest.json": HARNESS_REVIEW_MANIFEST_PATH,
    "harness-review-manifest.json.sha256": HARNESS_REVIEW_SIDECAR_PATH,
    "prebuilt_runtime_stage1.py": "scripts/prebuilt_runtime_stage1.py",
    "review-manifest.json": CANDIDATE_REVIEW_MANIFEST_PATH,
    "review-manifest.json.sha256": CANDIDATE_REVIEW_SIDECAR_PATH,
    "stage1-owner-binding.json": BINDING_PATH,
    "stage1-owner-public-key.json": TRUST_ANCHOR_PATH,
    "stage1_owner_authorization.py": "scripts/stage1_owner_authorization.py",
    **{f"review-root/{relative}": relative for relative in BASE_REVIEW_ROSTER},
}


def _regular_identity(path: Path) -> tuple[int, int, int, int, int, int]:
    metadata = path.lstat()
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
        raise ValueError("source is not one regular single-link file")
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_nlink,
        stat.S_IMODE(metadata.st_mode),
    )


def _safe_source(root: Path, relative: str, maximum: int = 16 * 1024 * 1024) -> Path:
    relative_path = Path(relative)
    if (
        relative_path.is_absolute()
        or relative_path.as_posix() != relative
        or any(part in ("", ".", "..") for part in relative_path.parts)
    ):
        raise ValueError("source path is invalid")
    root_resolved = root.resolve(strict=True)
    path = root / relative_path
    identity = _regular_identity(path)
    resolved = path.resolve(strict=True)
    if root_resolved not in resolved.parents or identity[2] <= 0 or identity[2] > maximum:
        raise ValueError("source escaped root or has invalid size")
    return path


def _read_regular(path: Path, maximum: int = 16 * 1024 * 1024) -> bytes:
    before = _regular_identity(path)
    if before[2] <= 0 or before[2] > maximum:
        raise ValueError("source size is invalid")
    payload = path.read_bytes()
    if len(payload) != before[2] or _regular_identity(path) != before:
        raise ValueError("source changed while reading")
    return payload


def _copy_regular(source: Path, destination: Path) -> None:
    payload = _read_regular(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.is_symlink() or destination.exists():
        raise ValueError("temporary destination already exists")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(destination, flags, 0o600)
    try:
        view = memoryview(payload)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise OSError("short write")
            view = view[written:]
        os.fchmod(descriptor, 0o644)
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    copied = _read_regular(destination)
    if copied != payload:
        raise ValueError("copied resource identity differs")


def _decode_canonical_object(payload: bytes) -> dict[str, Any]:
    def unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        value: dict[str, Any] = {}
        for key, item in pairs:
            if key in value:
                raise ValueError("duplicate JSON key")
            value[key] = item
        return value

    try:
        value = json.loads(
            payload.decode("ascii"),
            object_pairs_hook=unique,
            parse_constant=lambda _: (_ for _ in ()).throw(ValueError("non-finite JSON")),
        )
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("invalid canonical JSON") from error
    if not isinstance(value, dict) or canonical_json_bytes(value) != payload:
        raise ValueError("JSON is not one canonical object")
    return value


def _mode(root: Path) -> str:
    anchor = _read_regular(_safe_source(root, TRUST_ANCHOR_PATH), 4096)
    binding = _read_regular(_safe_source(root, BINDING_PATH), 1 << 20)
    if anchor == UNPROVISIONED_ANCHOR_BYTES and binding == UNFROZEN_BINDING_BYTES:
        return "bootstrap"
    anchor_value = _decode_canonical_object(anchor)
    binding_value = _decode_canonical_object(binding)
    if (
        anchor_value.get("schema") != PUBLIC_KEY_SCHEMA
        or anchor_value.get("state") != "pinned"
        or binding_value.get("schema") != BINDING_SCHEMA
        or binding_value.get("state") != "frozen"
        or binding_value.get("model_transfer") != "blocked"
    ):
        raise ValueError("Stage 1 source state is neither bootstrap nor final")
    return "final"


def _validate_final_state(root: Path) -> None:
    freezer._validate_pinned_anchor(root / TRUST_ANCHOR_PATH)
    existing = freezer._load_existing_binding(root / BINDING_PATH)
    if existing is None or existing.get("state") != "frozen":
        raise ValueError("Stage 1 binding is not frozen")
    replayed = freezer.freeze(root)
    if replayed != existing:
        raise ValueError("Stage 1 binding replay differs")


def _expected_payloads(root: Path, mode: str) -> dict[str, bytes]:
    if mode == "bootstrap":
        return {
            "bootstrap-only.json": BOOTSTRAP_MARKER_BYTES,
            "stage1-owner-binding.json": _read_regular(
                _safe_source(root, BINDING_PATH), 1 << 20
            ),
            "stage1-owner-public-key.json": _read_regular(
                _safe_source(root, TRUST_ANCHOR_PATH), 4096
            ),
        }
    _validate_final_state(root)
    return {
        relative: _read_regular(_safe_source(root, source))
        for relative, source in FINAL_FILE_MAP.items()
    }


def _tree_files(root: Path) -> set[str]:
    if root.is_symlink():
        raise ValueError("prepared destination is a symlink")
    metadata = root.lstat()
    if not stat.S_ISDIR(metadata.st_mode):
        raise ValueError("prepared destination is not a directory")
    files: set[str] = set()
    for path in root.rglob("*"):
        relative = path.relative_to(root).as_posix()
        info = path.lstat()
        if path.is_symlink():
            raise ValueError("prepared destination contains a symlink")
        if stat.S_ISDIR(info.st_mode):
            continue
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
            raise ValueError("prepared destination contains an unsafe entry")
        files.add(relative)
    return files


def _tree_matches(root: Path, payloads: dict[str, bytes]) -> bool:
    try:
        if _tree_files(root) != set(payloads):
            return False
        for relative, payload in payloads.items():
            path = root / relative
            if stat.S_IMODE(path.lstat().st_mode) != 0o644 or _read_regular(path) != payload:
                return False
        return True
    except (OSError, ValueError):
        return False


def _is_exact_bootstrap(root: Path) -> bool:
    try:
        expected = {
            "bootstrap-only.json": BOOTSTRAP_MARKER_BYTES,
            "stage1-owner-binding.json": UNFROZEN_BINDING_BYTES,
            "stage1-owner-public-key.json": UNPROVISIONED_ANCHOR_BYTES,
        }
        return _tree_matches(root, expected)
    except (OSError, ValueError):
        return False


def _fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _seal_directories(root: Path) -> None:
    directories = [path for path in root.rglob("*") if path.is_dir()]
    for path in sorted(directories, key=lambda item: len(item.parts), reverse=True):
        os.chmod(path, 0o755)
        _fsync_directory(path)
    os.chmod(root, 0o755)
    _fsync_directory(root)


def _remove_owned_tree(path: Path) -> None:
    if path.is_symlink():
        raise ValueError("refusing to remove a symlinked generated tree")
    if path.exists():
        for directory in [path, *[p for p in path.rglob("*") if p.is_dir()]]:
            os.chmod(directory, 0o700)
        shutil.rmtree(path)


def prepare(repo_root: Path) -> dict[str, object]:
    root = Path(repo_root)
    root_info = root.lstat()
    if root.is_symlink() or not stat.S_ISDIR(root_info.st_mode):
        raise ValueError("repository root is unsafe")
    root = root.resolve(strict=True)
    mode = _mode(root)
    payloads = _expected_payloads(root, mode)

    generated_parent = root / GENERATED_RELATIVE.parent
    if generated_parent.is_symlink():
        raise ValueError("generated parent is a symlink")
    generated_parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if generated_parent.is_symlink() or not generated_parent.is_dir():
        raise ValueError("generated parent is unsafe")
    destination = root / GENERATED_RELATIVE
    if destination.is_symlink():
        raise ValueError("prepared destination is a symlink")
    if destination.exists():
        if _tree_matches(destination, payloads):
            return {"file_count": len(payloads), "mode": mode}
        if mode != "final" or not _is_exact_bootstrap(destination):
            raise ValueError("prepared destination differs from exact expected state")

    temporary = generated_parent / f".stage1.prepare-{os.getpid()}-{uuid.uuid4().hex}"
    temporary.mkdir(mode=0o700)
    source_identities: dict[str, tuple[int, int, int, int, int, int]] = {}
    try:
        for relative in sorted(payloads):
            source_relative = (
                BOOTSTRAP_FILE_MAP.get(relative)
                if mode == "bootstrap"
                else FINAL_FILE_MAP[relative]
            )
            if source_relative is None:
                target = temporary / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                descriptor = os.open(
                    target,
                    os.O_WRONLY
                    | os.O_CREAT
                    | os.O_EXCL
                    | getattr(os, "O_CLOEXEC", 0)
                    | getattr(os, "O_NOFOLLOW", 0),
                    0o600,
                )
                try:
                    os.write(descriptor, payloads[relative])
                    os.fchmod(descriptor, 0o644)
                    os.fsync(descriptor)
                finally:
                    os.close(descriptor)
                continue
            source = _safe_source(root, source_relative)
            before = _regular_identity(source)
            source_identities[source_relative] = before
            _copy_regular(source, temporary / relative)
            if _regular_identity(source) != before:
                raise ValueError("source changed while copying")
        if {
            relative: _read_regular(temporary / relative)
            for relative in sorted(payloads)
        } != {relative: payloads[relative] for relative in sorted(payloads)}:
            raise ValueError("prepared tree identity differs")
        for source_relative, identity in source_identities.items():
            if _regular_identity(_safe_source(root, source_relative)) != identity:
                raise ValueError("source changed while copying")
        _seal_directories(temporary)
        _fsync_directory(generated_parent)

        if destination.exists():
            backup = generated_parent / f".stage1.previous-{os.getpid()}-{uuid.uuid4().hex}"
            os.replace(destination, backup)
            try:
                os.replace(temporary, destination)
            except BaseException:
                os.replace(backup, destination)
                raise
            _fsync_directory(generated_parent)
            _remove_owned_tree(backup)
        else:
            os.replace(temporary, destination)
            _fsync_directory(generated_parent)
        if not _tree_matches(destination, payloads):
            raise ValueError("published prepared tree differs")
        return {"file_count": len(payloads), "mode": mode}
    finally:
        if temporary.exists():
            _remove_owned_tree(temporary)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--repo-root", type=Path, default=Path(__file__).resolve().parents[1]
    )
    arguments = parser.parse_args(argv)
    result = prepare(arguments.repo_root)
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
