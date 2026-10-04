#!/usr/bin/env python3
"""Verify exact prepared and bundled Stage 1 owner resources."""

from __future__ import annotations

import argparse
import hashlib
import json
import stat
import sys
from pathlib import Path

_SCRIPT_ROOT = Path(__file__).resolve().parents[1]
if str(_SCRIPT_ROOT) not in sys.path:
    sys.path.insert(0, str(_SCRIPT_ROOT))

try:
    from scripts import prepare_stage1_owner_authorization_bundle as preparer
except ModuleNotFoundError:
    import prepare_stage1_owner_authorization_bundle as preparer


def _files(root: Path) -> set[str]:
    if root.is_symlink():
        raise ValueError("Stage 1 resource root is a symlink")
    metadata = root.lstat()
    if not stat.S_ISDIR(metadata.st_mode):
        raise ValueError("Stage 1 resource root is not a directory")
    files: set[str] = set()
    for path in root.rglob("*"):
        relative = path.relative_to(root).as_posix()
        info = path.lstat()
        if path.is_symlink():
            raise ValueError("Stage 1 resource tree contains a symlink")
        if stat.S_ISDIR(info.st_mode):
            continue
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
            raise ValueError("Stage 1 resource is not one sealed regular file")
        files.add(relative)
    return files


def _snapshot(root: Path, payloads: dict[str, bytes]) -> dict[str, str]:
    if _files(root) != set(payloads):
        raise ValueError("Stage 1 resource composition differs")
    snapshot: dict[str, str] = {}
    for relative, expected in sorted(payloads.items()):
        path = root / relative
        if stat.S_IMODE(path.lstat().st_mode) != 0o644:
            raise ValueError("Stage 1 resource mode differs from reviewed source")
        payload = preparer._read_regular(path)
        if payload != expected:
            raise ValueError("Stage 1 resource differs from reviewed source")
        snapshot[relative] = (
            f"{hashlib.sha256(payload).hexdigest()}:{len(payload)}:"
            f"{stat.S_IMODE(path.lstat().st_mode):04o}"
        )
    return snapshot


def verify_prepared(repo_root: Path, *, final: bool) -> dict[str, object]:
    root = Path(repo_root)
    if root.is_symlink():
        raise ValueError("repository root is a symlink")
    root = root.resolve(strict=True)
    mode = preparer._mode(root)
    if final and mode != "final":
        raise ValueError("final Stage 1 resources are unavailable")
    if not final and mode != "bootstrap":
        raise ValueError("bootstrap verification requires bootstrap source state")
    payloads = preparer._expected_payloads(root, mode)
    generated = root / preparer.GENERATED_RELATIVE
    snapshot = _snapshot(generated, payloads)
    return {
        "file_count": len(snapshot),
        "mode": mode,
        "snapshot": snapshot,
    }


def verify(app: Path, repo_root: Path, *, final: bool = True) -> dict[str, object]:
    prepared = verify_prepared(repo_root, final=final)
    app_input = Path(app)
    if app_input.is_symlink():
        raise ValueError("app bundle is a symlink")
    app_root = app_input.resolve(strict=True)
    if app_root.suffix != ".app":
        raise ValueError("app bundle path is invalid")
    prepared_root = Path(repo_root).resolve(strict=True) / preparer.GENERATED_RELATIVE
    payloads = {
        relative: preparer._read_regular(prepared_root / relative)
        for relative in prepared["snapshot"]
    }
    bundled_root = app_root / "Contents/Resources/saturday-resources/stage1"
    bundled_snapshot = _snapshot(bundled_root, payloads)
    if bundled_snapshot != prepared["snapshot"]:
        raise ValueError("bundled Stage 1 snapshot differs from prepared resources")
    protected_names = {Path(relative).name for relative in payloads}
    for candidate in app_root.rglob("*"):
        if candidate.name not in protected_names or not (
            candidate.is_file() or candidate.is_symlink()
        ):
            continue
        try:
            candidate.relative_to(bundled_root)
        except ValueError:
            raise ValueError("duplicate Stage 1 resource exists")
    return {
        "mode": prepared["mode"],
        "snapshot": bundled_snapshot,
        "stage1_resource_count": len(bundled_snapshot),
        "stage1_snapshot_sha256": hashlib.sha256(
            json.dumps(
                bundled_snapshot, sort_keys=True, separators=(",", ":")
            ).encode("ascii")
        ).hexdigest(),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bootstrap", action="store_true")
    parser.add_argument(
        "--repo-root", type=Path, default=Path(__file__).resolve().parents[1]
    )
    parser.add_argument(
        "--app",
        type=Path,
        default=Path("src-tauri/target/release/bundle/macos/Saturday.app"),
    )
    arguments = parser.parse_args(argv)
    repository = arguments.repo_root.resolve(strict=True)
    app = arguments.app
    if not app.is_absolute():
        app = repository / app
    result = verify(app, repository, final=not arguments.bootstrap)
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
