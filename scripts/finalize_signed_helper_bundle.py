#!/usr/bin/env python3
"""Finalize the nested helper identity after macOS code signing."""

from __future__ import annotations

import plistlib
import hashlib
import json
import os
import stat
import subprocess
import tempfile
from pathlib import Path

try:
    from scripts.build_native_helper import (
        SOURCE_ALLOWLIST,
        IdentityDocument,
        atomic_publish_bytes,
        source_generation,
        validate_macho,
        validate_stage1_bundle_resources,
        validate_tauri_resource_configuration,
    )
    from scripts.verify_signed_helper_bundle import (
        _entitlements,
        _expected_app_entitlements,
        _requires_hardened_runtime,
    )
except ModuleNotFoundError:
    from build_native_helper import (
        SOURCE_ALLOWLIST,
        IdentityDocument,
        atomic_publish_bytes,
        source_generation,
        validate_macho,
        validate_stage1_bundle_resources,
        validate_tauri_resource_configuration,
    )
    from verify_signed_helper_bundle import (
        _entitlements,
        _expected_app_entitlements,
        _requires_hardened_runtime,
    )

DEFAULT_APP = Path(
    "src-tauri/target/release/bundle/macos/S.A.T.U.R.D.A.Y.app"
)


def _sealed_bundle_snapshot(
    helper: Path, identity_path: Path, stage1: dict[str, str]
) -> dict[str, str]:
    snapshot = {f"stage1/{name}": value for name, value in stage1.items()}
    for name, path in (("helper", helper), ("helper-identity", identity_path)):
        metadata = path.lstat()
        if path.is_symlink() or not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
            raise ValueError("sealed bundle member is invalid")
        payload = path.read_bytes()
        snapshot[name] = (
            f"{hashlib.sha256(payload).hexdigest()}:{len(payload)}:"
            f"{stat.S_IMODE(metadata.st_mode):04o}"
        )
    return snapshot
VOICE_WORKERS = (
    (
        "voice-workers/asr/saturday-voice-asr-worker",
        "dev.rohtolos.saturday.voice.asr-worker",
    ),
    (
        "voice-workers/llm/saturday-voice-llm-worker",
        "dev.rohtolos.saturday.voice.llm-worker",
    ),
)


def _apple_development_team(path: Path) -> str:
    team_id = _requires_hardened_runtime(path)
    result = subprocess.run(
        ["/usr/bin/codesign", "-d", "--verbose=4", str(path)],
        check=True,
        capture_output=True,
        text=True,
    )
    details = result.stdout + result.stderr
    if not any(
        line.startswith("Authority=Apple Development:")
        for line in details.splitlines()
    ):
        raise ValueError("voice signing identity is not Apple Development")
    return team_id

def _code_identifier(path: Path) -> str:
    result = subprocess.run(
        ["/usr/bin/codesign", "-d", "--verbose=4", str(path)],
        check=True,
        capture_output=True,
        text=True,
    )
    identifiers = [
        line.removeprefix("Identifier=")
        for line in (result.stdout + result.stderr).splitlines()
        if line.startswith("Identifier=")
    ]
    if len(identifiers) != 1 or not identifiers[0]:
        raise ValueError("signed code identifier is invalid")
    return identifiers[0]




def _bootstrap_binding(app: Path, repository: Path) -> bool:
    bundled = app / "Contents" / "Resources" / "saturday-resources" / "voice-asset-bindings.json"
    generated = repository / "src-tauri" / "target" / "voice-owner-build" / "voice-asset-bindings.json"
    for path in (bundled, generated):
        metadata = path.lstat()
        if path.is_symlink() or not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 256 * 1024:
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


def _verify_voice_workers(app: Path, team_id: str, *, bootstrap: bool = False) -> None:
    resources = app / "Contents" / "Resources" / "saturday-resources"
    if bootstrap:
        absent = (
            *(resources / relative for relative, _ in VOICE_WORKERS),
            resources / "voice-workers" / "worker-artifact-manifest.json",
            resources / "voice-runtime-manifest.json",
            resources / "voice-workers" / "notices",
        )
        if any(path.exists() or path.is_symlink() for path in absent):
            raise ValueError("bootstrap bundle must not contain voice worker resources")
        return
    for relative, identifier in VOICE_WORKERS:
        worker = resources / relative
        metadata = worker.lstat()
        if worker.is_symlink() or not stat.S_ISREG(metadata.st_mode):
            raise ValueError("voice worker resource is invalid")
        subprocess.run(
            ["/usr/bin/codesign", "--verify", "--strict", str(worker)],
            check=True,
        )
        if (
            _apple_development_team(worker) != team_id
            or _code_identifier(worker) != identifier
        ):
            raise ValueError("voice worker identity differs from the fixed role")
        worker_entitlements = _entitlements(worker)
        if worker_entitlements:
            raise ValueError("voice worker entitlements must be empty")
    for relative in (
        "voice-workers/worker-artifact-manifest.json",
        "voice-runtime-manifest.json",
    ):
        path = resources / relative
        metadata = path.lstat()
        if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
            raise ValueError("voice runtime manifest resource is invalid")
    notices = resources / "voice-workers" / "notices"
    if notices.is_symlink() or not notices.is_dir():
        raise ValueError("voice runtime notices directory is invalid")




def finalize(app: Path, signing_identity: str, entitlements: Path) -> IdentityDocument:
    if not signing_identity or signing_identity == "-":
        raise ValueError("a non-ad-hoc signing identity is required")
    if app.is_symlink():
        raise ValueError("invalid app bundle")
    entitlements_input = entitlements.absolute()
    if entitlements_input.is_symlink():
        raise ValueError("app entitlements path is a symlink")
    expected_app = (
        entitlements_input.parent
        / "target/release/bundle/macos/S.A.T.U.R.D.A.Y.app"
    ).absolute()
    supplied_app = app.absolute()
    if supplied_app != expected_app or supplied_app.suffix != ".app":
        raise ValueError("invalid app bundle")
    entitlements = entitlements_input.resolve(strict=True)
    repository = entitlements.parent.parent.resolve(strict=True)
    app = supplied_app.resolve(strict=True)
    if app != expected_app.resolve(strict=True):
        raise ValueError("app bundle escaped expected build root")
    bootstrap = _bootstrap_binding(app, repository)
    validate_tauri_resource_configuration(repository, bootstrap=bootstrap)
    try:
        app_entitlements = plistlib.loads(entitlements.read_bytes())
    except (OSError, ValueError, plistlib.InvalidFileException) as error:
        raise ValueError("app entitlements are invalid") from error
    if app_entitlements != _expected_app_entitlements():
        raise ValueError("app entitlements differ from the supported surface")
    initial_stage1_snapshot = validate_stage1_bundle_resources(app, repository)
    helper = app / "Contents" / "MacOS" / "saturday-runtime-helper"
    identity_path = app / "Contents" / "Resources" / "saturday-resources" / "helper-identity.json"
    helper_entitlements = entitlements.with_name("HelperEntitlements.plist")
    if (
        helper.is_symlink()
        or not helper.is_file()
        or identity_path.is_symlink()
        or not identity_path.is_file()
        or helper_entitlements.is_symlink()
        or not helper_entitlements.is_file()
    ):
        raise ValueError("invalid nested helper layout")
    subprocess.run(
        ["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)],
        check=True,
    )
    subprocess.run(
        ["/usr/bin/codesign", "--verify", "--strict", str(helper)], check=True
    )
    original = IdentityDocument.from_json_bytes(identity_path.read_bytes())
    expected_generation = source_generation(repository, SOURCE_ALLOWLIST)
    if original.source_generation_sha256 != expected_generation:
        raise ValueError("bundle source generation is stale")
    staged = (
        entitlements.parent
        / "binaries"
        / f"saturday-runtime-helper-{original.target_triple}"
    )
    descriptor = os.open(staged, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_nlink != 1
            or metadata.st_size != original.binary_size
        ):
            raise ValueError("staged helper identity is invalid")
        with os.fdopen(descriptor, "rb", closefd=False) as stream:
            unsigned_binary = stream.read(original.binary_size + 1)
    finally:
        os.close(descriptor)
    if len(unsigned_binary) != original.binary_size:
        raise ValueError("staged helper size differs")
    if hashlib.sha256(unsigned_binary).hexdigest() != original.binary_sha256:
        raise ValueError("staged helper hash differs")
    validate_macho(unsigned_binary, original.target_triple)
    signing_parent = entitlements.parent / "target"
    with tempfile.TemporaryDirectory(
        prefix=".saturday-helper-sign-", dir=signing_parent
    ) as temporary:
        os.chmod(temporary, 0o700)
        signing_path = Path(temporary) / "saturday-runtime-helper"
        atomic_publish_bytes(signing_path, unsigned_binary, 0o700)
        subprocess.run(
            [
                "/usr/bin/codesign",
                "--force",
                "--options",
                "runtime",
                "--sign",
                signing_identity,
                "--entitlements",
                str(helper_entitlements),
                str(signing_path),
            ],
            check=True,
        )
        subprocess.run(
            ["/usr/bin/codesign", "--verify", "--strict", str(signing_path)],
            check=True,
        )
        if signing_path.is_symlink() or not signing_path.is_file():
            raise ValueError("signed helper staging path is invalid")
        team_id = _apple_development_team(signing_path)
        binary = signing_path.read_bytes()
    atomic_publish_bytes(helper, binary, 0o500)
    document = IdentityDocument(
        target_triple=original.target_triple,
        source_generation_sha256=original.source_generation_sha256,
        binary_sha256=hashlib.sha256(binary).hexdigest(),
        binary_size=len(binary),
    )
    atomic_publish_bytes(identity_path, document.to_json_bytes(), 0o600)
    pre_sign_stage1_snapshot = validate_stage1_bundle_resources(app, repository)
    if pre_sign_stage1_snapshot != initial_stage1_snapshot:
        raise ValueError("Stage 1 resources changed during finalization")
    sealed_snapshot = _sealed_bundle_snapshot(
        helper, identity_path, pre_sign_stage1_snapshot
    )
    _verify_voice_workers(app, team_id, bootstrap=bootstrap)
    subprocess.run(
        [
            "/usr/bin/codesign",
            "--force",
            "--options",
            "runtime",
            "--sign",
            signing_identity,
            "--entitlements",
            str(entitlements),
            str(app),
        ],
        check=True,
    )
    subprocess.run(
        ["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)],
        check=True,
    )
    if _apple_development_team(app) != team_id:
        raise ValueError("signed app team differs from its nested helper")
    _verify_voice_workers(app, team_id, bootstrap=bootstrap)
    if _entitlements(app) != _expected_app_entitlements():
        raise ValueError("signed app entitlements differ")
    validate_tauri_resource_configuration(repository, bootstrap=bootstrap)
    final_stage1_snapshot = validate_stage1_bundle_resources(app, repository)
    if _sealed_bundle_snapshot(helper, identity_path, final_stage1_snapshot) != sealed_snapshot:
        raise ValueError("sealed bundle content changed after final signing")
    if source_generation(repository, SOURCE_ALLOWLIST) != expected_generation:
        raise ValueError("source generation changed during finalization")
    return document


def main() -> int:
    signing_identity = os.environ.get("APPLE_SIGNING_IDENTITY", "")
    document = finalize(DEFAULT_APP, signing_identity, Path("src-tauri/Entitlements.plist"))
    print(
        json.dumps(
            {
                "binary_sha256": document.binary_sha256,
                "binary_size": document.binary_size,
                "finalized": True,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
