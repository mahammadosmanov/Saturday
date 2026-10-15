#!/usr/bin/env python3
"""Build and stage the signed-app helper candidate."""

from __future__ import annotations

import base64
import dataclasses
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Iterable, Tuple

_SCRIPT_ROOT = Path(__file__).resolve().parents[1]
if str(_SCRIPT_ROOT) not in sys.path:
    sys.path.insert(0, str(_SCRIPT_ROOT))

try:
    from scripts.stage1_owner_roster import BASE_REVIEW_ROSTER, BINDING_PATH
except ModuleNotFoundError:
    from stage1_owner_roster import BASE_REVIEW_ROSTER, BINDING_PATH

SCHEMA = "saturday.helper-identity.v1"
HELPER_NAME = "saturday-runtime-helper"
PROTOCOL_VERSION = 1
GENERATOR_VERSION = "1"
SUPPORTED_TARGET = "aarch64-apple-darwin"
MAX_HELPER_BYTES = 100 * 1024 * 1024
SHA256_PATTERN = re.compile(r"[0-9a-f]{64}\Z")
SOURCE_ALLOWLIST: Tuple[str, ...] = tuple(sorted(set(BASE_REVIEW_ROSTER) | {
    BINDING_PATH,
    "src-tauri/Cargo.lock",
    "src-tauri/Cargo.toml",
    "src-tauri/Entitlements.plist",
    "src-tauri/HelperEntitlements.plist",
    "scripts/generate_voice_owner_binding.py",
    "scripts/build_native_helper.py",
    "scripts/finalize_signed_helper_bundle.py",
    "scripts/build_voice_workers.py",
    "src-tauri/native/code_identity.c",
    "src-tauri/native/owner_authorization.c",
    "src-tauri/resources/stage1-owner-binding.json",
    "src-tauri/resources/stage1-owner-public-key.json",
    "src-tauri/src/bin/saturday-runtime-helper.rs",
    "src-tauri/src/helper/evidence.rs",
    "src-tauri/src/helper/harness.rs",
    "src-tauri/src/helper/manifest.rs",
    "src-tauri/src/helper/mod.rs",
    "src-tauri/src/helper/process.rs",
    "src-tauri/src/helper/protocol.rs",
    "src-tauri/src/helper/sandbox.rs",
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
    "src-tauri/src/platform/code_identity.rs",
    "src-tauri/src/platform/mod.rs",
    "src-tauri/src/platform/owner_authorization.rs",
    "src-tauri/src/signed_helper/helper.rs",
    "src-tauri/src/signed_helper/identity.rs",
    "src-tauri/src/signed_helper/mod.rs",
    "src-tauri/src/signed_helper/supervisor.rs",
    "src-tauri/tauri.conf.json",
    "src-tauri/tauri.bootstrap.conf.json",
    "src-tauri/tauri.full.conf.json",
    "package-lock.json",
    "package.json",
    "rust-toolchain.toml",
    "src-tauri/capabilities/voice-main.json",
    "src-tauri/native/en.lproj/InfoPlist.strings",
    "src-tauri/native/tr.lproj/InfoPlist.strings",
    "src-tauri/native/process_rss.c",
    "src-tauri/native/process_rss.h",
    "src-tauri/native/voice_assets.h",
    "src-tauri/native/voice_assets.m",
    "src-tauri/native/whisper_bridge.cpp",
    "src-tauri/native/whisper_bridge.h",
    "src-tauri/native/llama_adapter.cpp",
    "src-tauri/native/llama_adapter.h",
    "src-tauri/native/voice_authorization.m",
    "src-tauri/native/voice_capture.h",
    "src-tauri/native/voice_capture.m",
    "src-tauri/native/voice_lifecycle.h",
    "src-tauri/native/voice_lifecycle.m",
    "src-tauri/native/voice_speech.h",
    "src-tauri/native/voice_speech.m",
    "src-tauri/permissions/voice.toml",
    "src-tauri/resources/voice-asset-catalog.json",
    "src-tauri/resources/voice-asset-bindings.json",
    "src-tauri/resources/voice-runtime-build-recipe.json",
    "src-tauri/resources/voice-runtime-sources.json",
    "src-tauri/src/platform/voice_assets.rs",
    "src-tauri/src/platform/voice_authorization.rs",
    "src-tauri/src/platform/voice_capture.rs",
    "src-tauri/src/platform/voice_lifecycle.rs",
    "src-tauri/src/platform/voice_speech.rs",
    "src-tauri/src/voice/asr.rs",
    "src-tauri/src/voice/assets.rs",
    "src-tauri/src/voice/capture.rs",
    "src-tauri/src/voice/context.rs",
    "src-tauri/src/voice/contract.rs",
    "src-tauri/src/voice/conversation.rs",
    "src-tauri/src/voice/ipc.rs",
    "src-tauri/src/voice/llama_backend.rs",
    "src-tauri/src/voice/mod.rs",
    "src-tauri/src/voice/router.rs",
    "src-tauri/src/voice/speech.rs",
    "src-tauri/src/voice/supervisor.rs",
    "src-tauri/src/voice/text.rs",
    "src-tauri/src/voice/worker.rs",
    "src-tauri/src/voice_authorization.rs",
    "src-tauri/src/voice_authorization/catalog.rs",
    "src-tauri/src/voice_authorization/consent.rs",
    "src-tauri/src/voice_authorization/gguf.rs",
    "src-tauri/src/voice_authorization/issuer.rs",
    "src-tauri/src/voice_authorization/ledger.rs",
    "src-tauri/voice-asr-worker/Cargo.toml",
    "src-tauri/voice-asr-worker/build.rs",
    "src-tauri/voice-asr-worker/src/main.rs",
    "src-tauri/voice-llm-worker/Cargo.toml",
    "src-tauri/voice-llm-worker/build.rs",
    "src-tauri/voice-llm-worker/src/main.rs",
    "src-tauri/voice-parent-auth/Cargo.toml",
    "src-tauri/voice-parent-auth/build.rs",
    "src-tauri/voice-parent-auth/native/parent_auth.m",
    "src-tauri/voice-parent-auth/src/bin/inspect_signed_code.rs",
    "src-tauri/voice-parent-auth/src/lib.rs",
    "src-tauri/voice-types/Cargo.toml",
    "src-tauri/voice-types/src/lib.rs",
    "src-tauri/voice-worker-protocol/Cargo.toml",
    "src-tauri/voice-worker-protocol/src/lib.rs",
}))

EXPECTED_BOOTSTRAP_RESOURCE_CONFIGURATION = {
    "generated/helper-identity.json": "saturday-resources/helper-identity.json",
    "generated/stage1": "saturday-resources/stage1",
    "native/en.lproj/InfoPlist.strings": "en.lproj/InfoPlist.strings",
    "native/tr.lproj/InfoPlist.strings": "tr.lproj/InfoPlist.strings",
    "target/voice-owner-build/voice-asset-bindings.json": "saturday-resources/voice-asset-bindings.json",
}

EXPECTED_RESOURCE_CONFIGURATION = {
    **EXPECTED_BOOTSTRAP_RESOURCE_CONFIGURATION,
    "resources/voice-workers/asr/saturday-voice-asr-worker": "saturday-resources/voice-workers/asr/saturday-voice-asr-worker",
    "resources/voice-workers/llm/saturday-voice-llm-worker": "saturday-resources/voice-workers/llm/saturday-voice-llm-worker",
    "resources/voice-workers/worker-artifact-manifest.json": "saturday-resources/voice-workers/worker-artifact-manifest.json",
    "resources/voice-workers/notices": "saturday-resources/voice-workers/notices",
    "resources/voice-workers/voice-runtime-manifest.json": "saturday-resources/voice-runtime-manifest.json",
}

@dataclasses.dataclass(frozen=True)
class IdentityDocument:
    target_triple: str
    source_generation_sha256: str
    binary_sha256: str
    binary_size: int

    def validate(self) -> None:
        validate_target(self.target_triple)
        if not SHA256_PATTERN.fullmatch(self.source_generation_sha256):
            raise ValueError("invalid source generation")
        if not SHA256_PATTERN.fullmatch(self.binary_sha256):
            raise ValueError("invalid binary hash")
        if not 0 < self.binary_size <= MAX_HELPER_BYTES:
            raise ValueError("invalid binary size")

    def to_json_bytes(self) -> bytes:
        self.validate()
        value = {
            "binary_sha256": self.binary_sha256,
            "binary_size": self.binary_size,
            "helper_name": HELPER_NAME,
            "protocol_version": PROTOCOL_VERSION,
            "schema": SCHEMA,
            "source_generation_sha256": self.source_generation_sha256,
            "target_triple": self.target_triple,
        }
        return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()

    @classmethod
    def from_json_bytes(cls, encoded: bytes) -> "IdentityDocument":
        if len(encoded) > 4096:
            raise ValueError("identity document is oversized")
        try:
            value = json.loads(encoded)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValueError("invalid identity JSON") from error
        expected = {
            "binary_sha256",
            "binary_size",
            "helper_name",
            "protocol_version",
            "schema",
            "source_generation_sha256",
            "target_triple",
        }
        if not isinstance(value, dict) or set(value) != expected:
            raise ValueError("identity keys do not match schema")
        if (
            value["schema"] != SCHEMA
            or value["helper_name"] != HELPER_NAME
            or value["protocol_version"] != PROTOCOL_VERSION
            or not isinstance(value["binary_size"], int)
            or isinstance(value["binary_size"], bool)
        ):
            raise ValueError("identity constants do not match")
        document = cls(
            target_triple=value["target_triple"],
            source_generation_sha256=value["source_generation_sha256"],
            binary_sha256=value["binary_sha256"],
            binary_size=value["binary_size"],
        )
        document.validate()
        return document


def validate_target(target: str) -> str:
    if target != SUPPORTED_TARGET:
        raise ValueError("unsupported target triple")
    return target


def _safe_source(root: Path, relative: str) -> Path:
    relative_path = Path(relative)
    if relative_path.is_absolute() or ".." in relative_path.parts:
        raise ValueError("unsafe source path")
    root_resolved = root.resolve(strict=True)
    current = root_resolved
    for part in relative_path.parts:
        current = current / part
        if current.is_symlink():
            raise ValueError("source path contains a symlink")
    if not current.is_file():
        raise ValueError("source file is missing")
    resolved = current.resolve(strict=True)
    try:
        resolved.relative_to(root_resolved)
    except ValueError as error:
        raise ValueError("source escaped root") from error
    return resolved


def source_generation(root: Path, relative_paths: Iterable[str]) -> str:
    digest = hashlib.sha256()
    digest.update(("saturday-helper-generator-v" + GENERATOR_VERSION).encode())
    digest.update(b"\0")
    paths = sorted(set(relative_paths))
    if not paths:
        raise ValueError("source allowlist is empty")
    for relative in paths:
        path = _safe_source(root, relative)
        data = path.read_bytes()
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(len(data).to_bytes(8, "big"))
        digest.update(data)
    return digest.hexdigest()


def _canonical_object(payload: bytes, maximum_bytes: int) -> dict[str, object]:
    if not payload or len(payload) > maximum_bytes:
        raise ValueError("resource JSON size is invalid")

    def unique(pairs: list[tuple[str, object]]) -> dict[str, object]:
        value: dict[str, object] = {}
        for key, item in pairs:
            if key in value:
                raise ValueError("duplicate resource JSON key")
            value[key] = item
        return value

    try:
        value = json.loads(
            payload.decode("utf-8"),
            object_pairs_hook=unique,
            parse_constant=lambda _: (_ for _ in ()).throw(
                ValueError("non-finite resource JSON")
            ),
        )
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("invalid resource JSON") from error
    if not isinstance(value, dict):
        raise ValueError("resource JSON root is not an object")
    encoded = json.dumps(
        value,
        ensure_ascii=True,
        allow_nan=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("ascii")
    if encoded != payload:
        raise ValueError("resource JSON is not canonical")
    return value


def _validate_stage1_resource_states(resources: dict[str, bytes]) -> None:
    binding = _canonical_object(resources["stage1-owner-binding.json"], 1 << 20)
    if set(binding) != {
        "artifacts",
        "candidate",
        "limits",
        "model_transfer",
        "resource_roster",
        "review",
        "schema",
        "state",
    } or (
        binding.get("schema") != "saturday.local-provider.stage1-owner-binding.v1"
        or binding.get("state") != "frozen"
        or binding.get("model_transfer") != "blocked"
    ):
        raise ValueError("Stage 1 binding is not frozen")

    anchor = _canonical_object(resources["stage1-owner-public-key.json"], 4096)
    if set(anchor) != {
        "algorithm",
        "key_id",
        "public_key_x963_base64",
        "schema",
        "state",
    } or (
        anchor.get("schema") != "saturday.local-provider.stage1-owner-public-key.v1"
        or anchor.get("state") != "pinned"
        or anchor.get("algorithm") != "ecdsa-p256-sha256-x962"
        or not isinstance(anchor.get("key_id"), str)
        or SHA256_PATTERN.fullmatch(str(anchor["key_id"])) is None
        or not isinstance(anchor.get("public_key_x963_base64"), str)
    ):
        raise ValueError("Stage 1 public key is not pinned")
    try:
        public_key = base64.b64decode(
            str(anchor["public_key_x963_base64"]).encode("ascii"), validate=True
        )
    except (UnicodeError, ValueError) as error:
        raise ValueError("Stage 1 public key is invalid") from error
    if (
        len(public_key) != 65
        or public_key[0] != 0x04
        or hashlib.sha256(public_key).hexdigest() != anchor["key_id"]
        or base64.b64encode(public_key).decode("ascii")
        != anchor["public_key_x963_base64"]
    ):
        raise ValueError("Stage 1 public key is invalid")


def validate_tauri_resource_configuration(
    repo_root: Path, *, bootstrap: bool = False
) -> None:
    relative = (
        "src-tauri/tauri.bootstrap.conf.json"
        if bootstrap
        else "src-tauri/tauri.full.conf.json"
    )
    configuration_path = _safe_source(repo_root, relative)
    try:
        configuration = json.loads(configuration_path.read_text(encoding="utf-8"))
        resources = configuration["bundle"]["resources"]
        if not bootstrap:
            base_path = _safe_source(repo_root, "src-tauri/tauri.conf.json")
            base_configuration = json.loads(base_path.read_text(encoding="utf-8"))
            resources = {**base_configuration["bundle"]["resources"], **resources}
    except (KeyError, TypeError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("bundle resource configuration is invalid") from error
    expected = (
        EXPECTED_BOOTSTRAP_RESOURCE_CONFIGURATION
        if bootstrap
        else EXPECTED_RESOURCE_CONFIGURATION
    )
    if resources != expected:
        raise ValueError("bundle resource configuration differs")


def validate_stage1_bundle_resources(
    app: Path, repo_root: Path, *, final: bool | None = None
) -> dict[str, str]:
    from scripts import prepare_stage1_owner_authorization_bundle as preparer
    from scripts.verify_stage1_owner_authorization_bundle import verify

    if final is None:
        final = preparer._mode(repo_root) == "final"
    result = verify(app, repo_root, final=final)
    snapshot = result.get("snapshot")
    if not isinstance(snapshot, dict) or not all(
        isinstance(key, str) and isinstance(value, str)
        for key, value in snapshot.items()
    ):
        raise ValueError("Stage 1 verifier returned an invalid snapshot")
    return snapshot


def validate_macho(data: bytes, target: str) -> None:
    validate_target(target)
    if len(data) < 32 or data[:4] != bytes.fromhex("cffaedfe"):
        raise ValueError("helper is not a little-endian 64-bit Mach-O")
    cpu_type = int.from_bytes(data[4:8], "little")
    if cpu_type != 0x0100000C:
        raise ValueError("helper is not arm64 Mach-O")


def _ensure_private_directory(path: Path) -> None:
    if path.is_symlink():
        raise ValueError("output directory is a symlink")
    if path.exists():
        if not path.is_dir():
            raise ValueError("output directory is not a directory")
    else:
        path.mkdir(parents=True, mode=0o700)
    os.chmod(path, 0o700)


def atomic_publish_bytes(destination: Path, data: bytes, mode: int) -> None:
    _ensure_private_directory(destination.parent)
    if destination.is_symlink():
        raise ValueError("destination is a symlink")
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{destination.name}.", dir=str(destination.parent)
    )
    temporary = Path(temporary_name)
    try:
        os.fchmod(descriptor, mode)
        with os.fdopen(descriptor, "wb", closefd=True) as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, destination)
        directory_descriptor = os.open(destination.parent, os.O_RDONLY)
        try:
            os.fsync(directory_descriptor)
        finally:
            os.close(directory_descriptor)
    except BaseException:
        try:
            os.close(descriptor)
        except OSError:
            pass
        temporary.unlink(missing_ok=True)
        raise


def _target_from_environment() -> str:
    target = os.environ.get("TAURI_ENV_TARGET_TRIPLE")
    if target:
        return validate_target(target)
    result = subprocess.run(
        ["rustc", "--print", "host-tuple"],
        check=True,
        capture_output=True,
        text=True,
    )
    return validate_target(result.stdout.strip())


def build(repo_root: Path) -> IdentityDocument:
    repo_root = repo_root.resolve(strict=True)
    target = _target_from_environment()
    generation = source_generation(repo_root, SOURCE_ALLOWLIST)
    cargo_root = repo_root / "src-tauri"
    environment = os.environ.copy()
    environment["SATURDAY_HELPER_BUILD_ID"] = generation
    # Break the build-hook cycle: the helper must exist before Tauri can resolve
    # externalBin, so this helper-only Cargo invocation disables bundling inputs.
    environment["TAURI_CONFIG"] = json.dumps(
        {"bundle": {"externalBin": [], "resources": None}}, separators=(",", ":")
    )
    subprocess.run(
        [
            "cargo",
            "build",
            "--release",
            "--locked",
            "--bin",
            HELPER_NAME,
            "--features",
            "runtime-helper",
            "--target",
            target,
        ],
        cwd=cargo_root,
        env=environment,
        check=True,
    )
    if source_generation(repo_root, SOURCE_ALLOWLIST) != generation:
        raise ValueError("generation inputs changed during helper build")
    artifact = cargo_root / "target" / target / "release" / HELPER_NAME
    if artifact.is_symlink() or not artifact.is_file():
        raise ValueError("helper build artifact is invalid")
    binary = artifact.read_bytes()
    if not 0 < len(binary) <= MAX_HELPER_BYTES:
        raise ValueError("helper build artifact size is invalid")
    validate_macho(binary, target)
    document = IdentityDocument(
        target_triple=target,
        source_generation_sha256=generation,
        binary_sha256=hashlib.sha256(binary).hexdigest(),
        binary_size=len(binary),
    )
    staged_binary = (
        cargo_root / "binaries" / f"{HELPER_NAME}-{target}"
    )
    identity_path = cargo_root / "generated" / "helper-identity.json"
    atomic_publish_bytes(staged_binary, binary, 0o700)
    atomic_publish_bytes(identity_path, document.to_json_bytes(), 0o600)
    return document


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    document = build(repository)
    print(
        json.dumps(
            {
                "binary_sha256": document.binary_sha256,
                "binary_size": document.binary_size,
                "source_generation_sha256": document.source_generation_sha256,
                "target_triple": document.target_triple,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
