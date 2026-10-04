#!/usr/bin/env python3
"""Finalize the nested helper identity after macOS code signing."""

from __future__ import annotations

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

DEFAULT_APP = Path(
    "src-tauri/target/release/bundle/macos/Saturday.app"
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
        / "target/release/bundle/macos/Saturday.app"
    ).absolute()
    supplied_app = app.absolute()
    if supplied_app != expected_app or supplied_app.suffix != ".app":
        raise ValueError("invalid app bundle")
    entitlements = entitlements_input.resolve(strict=True)
    repository = entitlements.parent.parent.resolve(strict=True)
    app = supplied_app.resolve(strict=True)
    if app != expected_app.resolve(strict=True):
        raise ValueError("app bundle escaped expected build root")
    validate_tauri_resource_configuration(repository)
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
    validate_tauri_resource_configuration(repository)
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
