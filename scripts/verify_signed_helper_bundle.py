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

def _expected_app_entitlements() -> Dict[str, object]:
    return {"com.apple.security.device.audio-input": True}

VOICE_COMMANDS = (
    "voice_get_status",
    "voice_start",
    "voice_finish",
    "voice_submit_text",
    "voice_confirm_transcript",
    "voice_cancel",
    "voice_clear_session",
    "voice_select_preferences",
    "voice_ack_intent",
    "voice_assets_get_status",
    "voice_assets_request_install",
    "voice_assets_disable",
    "voice_assets_delete",
    "voice_assets_rollback",
    "voice_test_output",
)
VOICE_CAPABILITY = {
    "$schema": "../gen/schemas/desktop-schema.json",
    "identifier": "voice-main",
    "description": "Local voice control for the main Saturday window only.",
    "windows": ["main"],
    "permissions": ["voice-session", "voice-assets", "voice-test-output"],
}


def _validate_voice_command_surface(repo_root: Path) -> None:
    source_root = repo_root / "src-tauri"

    def read_json(relative: str) -> Dict[str, object]:
        path = source_root / relative
        if path.is_symlink() or not path.is_file():
            raise ValueError("voice command surface configuration is invalid")
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (UnicodeError, json.JSONDecodeError) as error:
            raise ValueError("voice command surface configuration is invalid") from error
        if not isinstance(value, dict):
            raise ValueError("voice command surface configuration is invalid")
        return value

    for name in ("tauri.conf.json", "tauri.bootstrap.conf.json"):
        configuration = read_json(name)
        try:
            capabilities = configuration["app"]["security"]["capabilities"]
        except (KeyError, TypeError) as error:
            raise ValueError("webview capabilities differ") from error
        if capabilities != ["voice-main"]:
            raise ValueError("webview capabilities differ")

    full_overlay = read_json("tauri.full.conf.json")
    full_app = full_overlay.get("app", {})
    if not isinstance(full_app, dict):
        raise ValueError("webview capabilities differ")
    full_security = full_app.get("security", {})
    if not isinstance(full_security, dict) or full_security.get(
        "capabilities", ["voice-main"]
    ) != ["voice-main"]:
        raise ValueError("webview capabilities differ")

    capability_path = source_root / "capabilities" / "voice-main.json"
    if capability_path.is_symlink() or not capability_path.is_file():
        raise ValueError("webview capabilities differ")
    try:
        capability = json.loads(capability_path.read_text(encoding="utf-8"))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("webview capabilities differ") from error
    if capability != VOICE_CAPABILITY:
        raise ValueError("webview capabilities differ")

    main_path = source_root / "src" / "main.rs"
    cargo_path = source_root / "Cargo.toml"
    if (
        main_path.is_symlink()
        or not main_path.is_file()
        or cargo_path.is_symlink()
        or not cargo_path.is_file()
    ):
        raise ValueError("voice command surface source is invalid")
    main_source = main_path.read_text(encoding="utf-8")
    cargo_source = cargo_path.read_text(encoding="utf-8")
    handlers = re.findall(
        r"\.invoke_handler\s*\(\s*tauri::generate_handler!\s*\[(.*?)\]\s*\)",
        main_source,
        re.DOTALL,
    )
    if main_source.count("invoke_handler") != 1 or len(handlers) != 1:
        raise ValueError("voice command surface differs")
    commands = [entry.strip() for entry in handlers[0].split(",") if entry.strip()]
    expected = [f"saturday::voice::ipc::{command}" for command in VOICE_COMMANDS]
    if commands != expected or "tauri-plugin-shell" in cargo_source:
        raise ValueError("voice command surface differs")


def _bootstrap_binding(app: Path, repo_root: Path) -> bool:
    bundled = (
        app
        / "Contents"
        / "Resources"
        / "saturday-resources"
        / "voice-asset-bindings.json"
    )
    generated = (
        repo_root
        / "src-tauri"
        / "target"
        / "voice-owner-build"
        / "voice-asset-bindings.json"
    )
    for path in (bundled, generated):
        metadata = path.lstat()
        if (
            path.is_symlink()
            or not stat.S_ISREG(metadata.st_mode)
            or metadata.st_size > 256 * 1024
        ):
            raise ValueError("voice binding resource is invalid")
    encoded = bundled.read_bytes()
    if encoded != generated.read_bytes():
        raise ValueError("packaged voice binding differs from its generated input")

    def unique(pairs: list[tuple[str, object]]) -> dict[str, object]:
        value: dict[str, object] = {}
        for key, item in pairs:
            if key in value:
                raise ValueError("duplicate voice binding member")
            value[key] = item
        return value

    try:
        binding = json.loads(encoded, object_pairs_hook=unique)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("voice binding resource is invalid") from error
    owner = binding.get("owner_key") if isinstance(binding, dict) else None
    if not isinstance(owner, dict):
        raise ValueError("voice binding owner key is invalid")
    if owner.get("state") == "bootstrap":
        return True
    if owner.get("state") == "key_pinned":
        return False
    raise ValueError("voice binding is neither bootstrap nor final")



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
    bootstrap = _bootstrap_binding(app, repo_root)
    validate_tauri_resource_configuration(repo_root, bootstrap=bootstrap)
    _validate_voice_command_surface(repo_root)
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
    if _entitlements(app) != _expected_app_entitlements():
        raise ValueError("app entitlements differ")
    if _entitlements(main_executable) != _expected_app_entitlements():
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
    if configuration["bundle"].get("externalBin") != [
        "binaries/saturday-runtime-helper"
    ]:
        raise ValueError("external helper configuration differs")

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
    validate_tauri_resource_configuration(repo_root, bootstrap=bootstrap)
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
    app = repo_root / "src-tauri/target/release/bundle/macos/S.A.T.U.R.D.A.Y.app"
    print(json.dumps(verify(app, repo_root), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
