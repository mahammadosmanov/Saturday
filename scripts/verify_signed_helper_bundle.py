#!/usr/bin/env python3
"""Fail-closed verifier for the signed nested-helper candidate."""

from __future__ import annotations

import hashlib
import json
import plistlib
import re
import stat
import subprocess
from pathlib import Path
from typing import Dict

try:
    from scripts.build_native_helper import (
        SOURCE_ALLOWLIST,
        IdentityDocument,
        source_generation,
        validate_stage1_bundle_resources,
        validate_tauri_resource_configuration,
    )
except ModuleNotFoundError:
    from build_native_helper import (
        SOURCE_ALLOWLIST,
        IdentityDocument,
        source_generation,
        validate_stage1_bundle_resources,
        validate_tauri_resource_configuration,
    )

TEAM_PATTERN = re.compile(r"^TeamIdentifier=([A-Z0-9]{10})$", re.MULTILINE)


def _team_identifier(path: Path) -> str:
    result = subprocess.run(
        ["/usr/bin/codesign", "-d", "--verbose=4", str(path)],
        check=True,
        capture_output=True,
        text=True,
    )
    details = result.stdout + result.stderr
    match = TEAM_PATTERN.search(details)
    if not match:
        raise ValueError("code has no non-ad-hoc team identity")
    return match.group(1)


def _requires_hardened_runtime(path: Path) -> str:
    result = subprocess.run(
        ["/usr/bin/codesign", "-d", "--verbose=4", str(path)],
        check=True,
        capture_output=True,
        text=True,
    )
    details = result.stdout + result.stderr
    if "flags=0x10000(runtime)" not in details:
        raise ValueError("code is missing hardened runtime")
    return _team_identifier(path)


def _entitlements(path: Path) -> Dict[str, object]:
    result = subprocess.run(
        ["/usr/bin/codesign", "-d", "--entitlements", ":-", str(path)],
        check=True,
        capture_output=True,
    )
    payload = result.stdout.strip()
    if not payload:
        return {}
    value = plistlib.loads(payload)
    if not isinstance(value, dict):
        raise ValueError("entitlements are not a dictionary")
    return value


def _locate_helper(app: Path) -> Path:
    helpers = [
        path
        for path in app.rglob("saturday-runtime-helper")
        if path.is_file() or path.is_symlink()
    ]
    if len(helpers) != 1:
        raise ValueError("nested helper count is not exactly one")
    helper = helpers[0]
    expected = app / "Contents" / "MacOS" / "saturday-runtime-helper"
    if helper != expected or helper.is_symlink():
        raise ValueError("nested helper path is invalid")
    return helper


def _reject_tracked_app_artifacts(repo_root: Path) -> None:
    result = subprocess.run(
        ["git", "-C", str(repo_root), "ls-files", "-z"],
        check=True,
        capture_output=True,
    )
    for encoded in result.stdout.split(b"\0"):
        if not encoded:
            continue
        try:
            path = encoded.decode("utf-8")
        except UnicodeError as error:
            raise ValueError("tracked path is not UTF-8") from error
        if any(component.endswith(".app") for component in Path(path).parts):
            raise ValueError("tracked app bundle artifact exists")


def verify(app: Path, repo_root: Path) -> Dict[str, object]:
    if app.is_symlink() or repo_root.is_symlink():
        raise ValueError("app bundle or repository root is a symlink")
    app = app.resolve(strict=True)
    repo_root = repo_root.resolve(strict=True)
    if app.suffix != ".app":
        raise ValueError("invalid app bundle")
    validate_tauri_resource_configuration(repo_root)
    stage1_snapshot = validate_stage1_bundle_resources(app, repo_root)
    _reject_tracked_app_artifacts(repo_root)
    subprocess.run(
        ["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)],
        check=True,
    )
    helper = _locate_helper(app)
    subprocess.run(
        ["/usr/bin/codesign", "--verify", "--strict", str(helper)], check=True
    )
    main_executable = app / "Contents" / "MacOS" / "saturday"
    app_team = _requires_hardened_runtime(app)
    main_team = _requires_hardened_runtime(main_executable)
    helper_team = _requires_hardened_runtime(helper)
    if app_team != helper_team or app_team != main_team:
        raise ValueError("app and helper teams differ")
    expected_app_entitlements = {"com.apple.security.device.audio-input": True}
    if _entitlements(app) != expected_app_entitlements:
        raise ValueError("outer app entitlements differ")
    if _entitlements(main_executable) != expected_app_entitlements:
        raise ValueError("main executable entitlements differ")
    if _entitlements(helper) != {}:
        raise ValueError("helper entitlements are not empty")
    mode = stat.S_IMODE(helper.stat().st_mode)
    if mode & 0o222 or not mode & 0o111:
        raise ValueError("helper permissions are not sealed executable permissions")

    identity_path = app / "Contents" / "Resources" / "saturday-resources" / "helper-identity.json"
    if identity_path.is_symlink() or not identity_path.is_file():
        raise ValueError("helper identity resource is missing")
    document = IdentityDocument.from_json_bytes(identity_path.read_bytes())
    expected_generation = source_generation(repo_root, SOURCE_ALLOWLIST)
    if document.source_generation_sha256 != expected_generation:
        raise ValueError("bundle source generation is stale")
    binary = helper.read_bytes()
    if len(binary) != document.binary_size:
        raise ValueError("helper size differs from identity")
    if hashlib.sha256(binary).hexdigest() != document.binary_sha256:
        raise ValueError("helper hash differs from identity")

    configuration = json.loads((repo_root / "src-tauri" / "tauri.conf.json").read_text())
    if configuration["app"]["security"]["capabilities"] != []:
        raise ValueError("webview capabilities are not empty")
    if configuration["bundle"].get("externalBin") != [
        "binaries/saturday-runtime-helper"
    ]:
        raise ValueError("external helper configuration differs")
    main_source = (repo_root / "src-tauri" / "src" / "main.rs").read_text()
    cargo_source = (repo_root / "src-tauri" / "Cargo.toml").read_text()
    if "invoke_handler" in main_source or "tauri-plugin-shell" in cargo_source:
        raise ValueError("unexpected helper command surface")

    self_test = subprocess.run(
        [str(main_executable), "--verify-signed-helper"],
        check=True,
        capture_output=True,
        text=True,
        timeout=10,
        env={"LANG": "C", "LC_ALL": "C"},
    )
    if self_test.stdout.strip() != "SATURDAY_SIGNED_HELPER_OK" or self_test.stderr:
        raise ValueError("signed helper self-test output differs")
    validate_tauri_resource_configuration(repo_root)
    if validate_stage1_bundle_resources(app, repo_root) != stage1_snapshot:
        raise ValueError("Stage 1 resources changed during verification")
    _reject_tracked_app_artifacts(repo_root)
    if source_generation(repo_root, SOURCE_ALLOWLIST) != expected_generation:
        raise ValueError("source generation changed during verification")
    return {
        "binary_sha256": document.binary_sha256,
        "binary_size": document.binary_size,
        "helper_count": 1,
        "matching_nonempty_team": True,
        "self_test": "pass",
        "stage1_resource_count": len(stage1_snapshot),
    }


def main() -> int:
    repo_root = Path(__file__).resolve().parent.parent
    app = repo_root / "src-tauri/target/release/bundle/macos/Saturday.app"
    print(json.dumps(verify(app, repo_root), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
