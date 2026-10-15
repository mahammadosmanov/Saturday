#!/usr/bin/env python3
"""Build only the two fixed, isolated voice worker roles after runtime approval."""

from __future__ import annotations

import base64
import hashlib
import json
import os
import platform
import plistlib
import re
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TAURI = REPO / "src-tauri"
RUNTIME = TAURI / "resources" / "voice-runtime-build"
RECEIPT = RUNTIME / "build-artifact-manifest.json"
TARGET = "aarch64-apple-darwin"
ROLES = {
    "asr": {
        "package": "saturday-voice-asr-worker",
        "binary": "saturday-voice-asr-worker",
        "provider": "whisper-cpp",
        "source_commit": "d1be6fde11ac6e0407606b4e42fe72d34add8037",
        "source_tree": "6fe2755682242a5c01aba65a22b3c0305d747841",
        "source_archive_sha256": "645445da884df6dbd52b30f82d7be7c01d1767c938714e477e786b88d529e9f4",
        "archive": "asr/libSaturdayWhisperRuntime.a",
        "bridge": "whisper_bridge.cpp",
        "bridge_header": "whisper_bridge.h",
        "delegate": "asr.rs",
    },
    "llm": {
        "package": "saturday-voice-llm-worker",
        "binary": "saturday-voice-llm-worker",
        "provider": "llama-cpp",
        "archive": "llm/libSaturdayLlamaRuntime.a",
        "source_commit": "8345f333951c661d166b00e6f9362e553768f292",
        "source_tree": "88538af4019aa9d18026ff43e446c34fb7456adf",
        "source_archive_sha256": "84050e783dbd0c4a4678f90f253b0881df04e90a98c20f73ae3709751c4a4850",
        "bridge": "llama_adapter.cpp",
        "bridge_header": "llama_adapter.h",
        "delegate": "llm.rs",
    },
}
APPROVED = {
    "state": "independently_reviewed_and_main_approved",
    "product_readiness": "approved_for_product_use",
    "review_status": "independent_artifact_review_passed",
    "main_approval": True,
}

OWNER_APPLICATION_TAG = b"dev.rohtolos.saturday.voice.owner-key.v1"
OWNER_KEY_PIN = TAURI / "target" / "voice-owner-build" / "voice-owner-key-pin.json"

def reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate owner binding field: {key}")
        result[key] = value
    return result


def trusted_owner_public_key() -> tuple[bytes, str]:
    directory = OWNER_KEY_PIN.parent
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError("private final owner-key build directory is missing")
    directory_metadata = directory.stat()
    if directory_metadata.st_uid != os.getuid() or stat.S_IMODE(directory_metadata.st_mode) != 0o700:
        raise ValueError("private final owner-key build directory must be current-user-owned mode 0700")
    metadata = OWNER_KEY_PIN.lstat()
    if (
        OWNER_KEY_PIN.is_symlink()
        or not stat.S_ISREG(metadata.st_mode)
        or metadata.st_nlink != 1
        or metadata.st_uid != os.getuid()
        or metadata.st_mode & 0o077
        or metadata.st_size > 1_048_576
    ):
        raise ValueError("final owner key pin must be a private, bounded, regular file")
    pin_fd = os.open(OWNER_KEY_PIN, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0))
    try:
        opened = os.fstat(pin_fd)
        if (opened.st_dev, opened.st_ino) != (metadata.st_dev, metadata.st_ino):
            raise ValueError("owner-key pin changed during the bounded open")
        with os.fdopen(pin_fd, "rb", closefd=False) as pin_file:
            pin_bytes = pin_file.read(1_048_577)
    finally:
        os.close(pin_fd)
    if len(pin_bytes) != metadata.st_size or len(pin_bytes) > 1_048_576:
        raise ValueError("owner-key pin size changed during the bounded read")
    pin = json.loads(pin_bytes, object_pairs_hook=reject_duplicate_keys)
    expected_fields = {
        "schema",
        "application_tag_sha256",
        "owner_public_key_x963_hex",
        "owner_public_key_sha256",
        "bootstrap_record_sha256",
        "bootstrap_record",
        "bootstrap_witness_sha256",
        "bootstrap_witness",
    }
    if not isinstance(pin, dict) or set(pin) != expected_fields:
        raise ValueError("final owner key pin has unexpected fields")
    if pin["schema"] != "saturday.voice.owner-key-pin.v1":
        raise ValueError("unexpected final owner key pin schema")
    if pin["application_tag_sha256"] != sha256(OWNER_APPLICATION_TAG):
        raise ValueError("final owner-key application tag does not match the fixed purpose")
    key_hex = pin["owner_public_key_x963_hex"]
    if not isinstance(key_hex, str) or len(key_hex) != 130 or not re.fullmatch(r"04[0-9a-f]{128}", key_hex):
        raise ValueError("final owner key is not canonical uncompressed P-256 X9.63")
    key = bytes.fromhex(key_hex)
    if key[1:] == bytes(64) or pin["owner_public_key_sha256"] != sha256(key):
        raise ValueError("final owner-key pin digest or public point is invalid")

    def decode_record(field: str, digest_field: str, maximum_bytes: int) -> bytes:
        encoded = pin[field]
        if not isinstance(encoded, str) or not re.fullmatch(r"[A-Za-z0-9_-]+", encoded):
            raise ValueError(f"{field} is not canonical unpadded base64url")
        padding = "=" * ((4 - len(encoded) % 4) % 4)
        decoded = base64.urlsafe_b64decode(encoded + padding)
        if (
            not decoded
            or len(decoded) > maximum_bytes
            or base64.urlsafe_b64encode(decoded).rstrip(b"=").decode("ascii") != encoded
            or pin[digest_field] != sha256(decoded)
        ):
            raise ValueError(f"{field} does not match its canonical digest")
        return decoded

    decode_record("bootstrap_record", "bootstrap_record_sha256", 8192)
    decode_record("bootstrap_witness", "bootstrap_witness_sha256", 8192)
    canonical_pin = json.dumps(pin, sort_keys=True, separators=(",", ":")).encode("ascii") + b"\n"
    if canonical_pin != pin_bytes:
        raise ValueError("owner key pin is not canonical JSON")
    return key, sha256(pin_bytes)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()

def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def hash_roster(entries: list[tuple[str, str]]) -> str:
    digest = hashlib.sha256()
    for logical_path, file_sha in sorted(entries):
        digest.update(logical_path.encode("utf-8"))
        digest.update(b"\0")
        digest.update(file_sha.encode("ascii"))
        digest.update(b"\n")
    return digest.hexdigest()


def tree_hash(path: Path) -> str:
    if path.is_symlink() or not path.is_dir():
        raise ValueError(f"expected real directory: {path}")
    entries: list[tuple[str, str]] = []
    for candidate in sorted(path.rglob("*")):
        if candidate.is_symlink():
            raise ValueError(f"symlink in sealed tree: {candidate}")
        if candidate.is_dir():
            continue
        if not candidate.is_file():
            raise ValueError(f"non-regular item in sealed tree: {candidate}")
        relative = candidate.relative_to(path).as_posix()
        entries.append((relative, file_sha256(candidate)))
    return hash_roster(entries)


def promoted_tree_digest(root: Path) -> str:
    if root.is_symlink() or not root.is_dir():
        raise ValueError(f"expected promoted provider directory: {root}")
    digest = hashlib.sha256()
    entries = []
    for candidate in root.rglob("*"):
        if candidate.is_symlink():
            raise ValueError(f"symlink in promoted runtime tree: {candidate}")
        if candidate.is_dir():
            continue
        if not candidate.is_file():
            raise ValueError(f"non-regular item in promoted runtime tree: {candidate}")
        relative = candidate.relative_to(root).as_posix()
        metadata = candidate.stat()
        entries.append((relative, stat.S_IMODE(metadata.st_mode), metadata.st_size, bytes.fromhex(file_sha256(candidate))))
    for relative, mode, size, file_sha in sorted(entries):
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(f"{mode:04o}".encode("ascii"))
        digest.update(b"\0")
        digest.update(str(size).encode("ascii"))
        digest.update(b"\0")
        digest.update(file_sha)
        digest.update(b"\n")
    return digest.hexdigest()


def require_sha(value: object, label: str) -> str:
    if not isinstance(value, str) or len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
        raise ValueError(f"{label} is not lowercase SHA-256")
    return value


def reviewed_provider(role: str, receipt: dict) -> tuple[dict, Path, str, str]:
    if receipt.get("schema") != "saturday.voice-runtime-artifacts.v1":
        raise ValueError("unexpected runtime artifact receipt schema")
    for field, expected in APPROVED.items():
        if receipt.get(field) != expected:
            raise ValueError(f"runtime artifact receipt is not approved: {field}")
    definition = ROLES[role]
    providers = receipt.get("providers")
    if not isinstance(providers, list) or len(providers) != 2:
        raise ValueError("receipt must contain exactly the two separate provider roles")
    matches = [item for item in providers if isinstance(item, dict) and item.get("provider") == definition["provider"]]
    if len(matches) != 1:
        raise ValueError(f"receipt must contain exactly one {definition['provider']} record")
    provider = matches[0]
    if provider.get("archive") != definition["archive"]:
        raise ValueError(f"{role} archive path differs from fixed role destination")
    archive = RUNTIME / definition["archive"]
    actual_sha = file_sha256(archive)
    if require_sha(provider.get("archive_sha256"), f"{role} archive SHA") != actual_sha:
        raise ValueError(f"{role} archive does not match its reviewed digest")
    source_manifest_path = TAURI / "resources" / "voice-runtime-sources.json"
    source_manifest_bytes = source_manifest_path.read_bytes()
    if sha256(source_manifest_bytes) != require_sha(receipt.get("source_manifest_sha256"), "runtime source manifest SHA"):
        raise ValueError("runtime source manifest does not match the build receipt")
    source_manifest = json.loads(source_manifest_bytes)
    if source_manifest.get("schema") != "saturday.voice-runtime-sources.v1" or not isinstance(source_manifest.get("runtimes"), list) or len(source_manifest["runtimes"]) != 2:
        raise ValueError("invalid pinned runtime source manifest")
    source_matches = [item for item in source_manifest["runtimes"] if item.get("id") == definition["provider"]]
    if len(source_matches) != 1:
        raise ValueError(f"{role} source manifest entry is missing or duplicated")
    source = source_matches[0]
    for key, manifest_key in (("source_commit", "commit"), ("source_tree", "tree"), ("source_archive_sha256", "archive_sha256")):
        if source.get(manifest_key) != definition[key] or provider.get(key) != definition[key]:
            raise ValueError(f"{role} provider differs from the exact pinned source identity")
    expected_frameworks = ["Accelerate", "Foundation", "Metal", "MetalKit"]
    expected_link_order = expected_frameworks + ["c++"]
    expected_link_flags = ["-framework", "Accelerate", "-framework", "Foundation", "-framework", "Metal", "-framework", "MetalKit", "-lc++"]
    if provider.get("framework_dependencies") != expected_frameworks or provider.get("framework_link_order") != expected_link_order or provider.get("framework_link_flags") != expected_link_flags:
        raise ValueError(f"{role} runtime framework link contract differs")
    if provider.get("archive_bytes") != archive.stat().st_size:
        raise ValueError(f"{role} runtime archive size differs from reviewed receipt")
    headers = provider.get("headers")
    if not isinstance(headers, list) or not headers:
        raise ValueError(f"{role} header roster is empty")
    include = RUNTIME / role / "include"
    expected: dict[str, str] = {}
    header_roster = []
    for entry in headers:
        if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
            raise ValueError(f"invalid {role} header roster item")
        path = Path(entry["path"])
        if len(path.parts) != 2 or path.parts[0] != "include" or path.name in expected:
            raise ValueError(f"{role} header paths must be unique include/<basename> entries")
        expected[path.name] = require_sha(entry.get("sha256"), f"{role} header {path.name} SHA")
        header_roster.append((entry["path"], expected[path.name]))
    actual_names: set[str] = set()
    for candidate in include.rglob("*"):
        if candidate.is_symlink():
            raise ValueError(f"symlink in {role} header tree: {candidate}")
        if candidate.is_file():
            if candidate.parent != include:
                raise ValueError(f"nested {role} header path is not allowed: {candidate}")
            actual_names.add(candidate.name)
            if file_sha256(candidate) != expected.get(candidate.name):
                raise ValueError(f"{role} header hash mismatch: {candidate.name}")
    if actual_names != set(expected):
        raise ValueError(f"{role} header tree differs from the reviewed roster")
    notices = provider.get("notices")
    if not isinstance(notices, list):
        raise ValueError(f"{role} legal notice roster is absent")
    for notice in notices:
        if not isinstance(notice, dict) or not isinstance(notice.get("path"), str):
            raise ValueError(f"invalid {role} notice roster item")
        path = Path(notice["path"])
        if not path.parts or path.parts[0] != "notices" or ".." in path.parts:
            raise ValueError(f"{role} notice path escapes its fixed role directory")
        notice_path = RUNTIME / role / path
        if require_sha(notice.get("sha256"), f"{role} notice SHA") != file_sha256(notice_path):
            raise ValueError(f"{role} notice hash mismatch: {path}")
    recipe_path = TAURI / "resources" / "voice-runtime-build-recipe.json"
    if require_sha(receipt.get("recipe_sha256"), "runtime recipe SHA") != file_sha256(recipe_path):
        raise ValueError("runtime build recipe does not match the artifact receipt")
    require_sha(receipt.get("source_manifest_sha256"), "runtime source-generation SHA")
    actual_tree = promoted_tree_digest(RUNTIME / role)
    expected_tree = require_sha(provider.get("promoted_tree_sha256"), f"{role} promoted tree SHA")
    if actual_tree != expected_tree:
        raise ValueError(f"{role} promoted tree differs from reviewed receipt")
    return provider, archive, hash_roster(header_roster), actual_tree


def seal_inputs(role: str, definition: dict, archive: Path, receipt_bytes: bytes, archive_sha: str, header_seal: str, promoted_tree: str, receipt: dict, owner_public_key: bytes, owner_key_pin_sha256: str) -> tuple[str, str, list[dict[str, str]]]:
    tauri = TAURI
    native = tauri / "native"
    package = tauri / ("voice-asr-worker" if role == "asr" else "voice-llm-worker")
    sources: list[tuple[str, Path]] = [
        ("src-tauri/Cargo.lock", tauri / "Cargo.lock"),
        ("src-tauri/Cargo.toml", tauri / "Cargo.toml"),
        ("src-tauri/tauri.conf.json", tauri / "tauri.conf.json"),
        ("src-tauri/resources/voice-runtime-build/build-artifact-manifest.json", RECEIPT),
        ("src-tauri/resources/voice-runtime-sources.json", tauri / "resources" / "voice-runtime-sources.json"),
        ("src-tauri/resources/voice-runtime-build/archive", archive),
        ("src-tauri/voice-worker-protocol/Cargo.toml", tauri / "voice-worker-protocol/Cargo.toml"),
        ("src-tauri/voice-worker-protocol/src/lib.rs", tauri / "voice-worker-protocol/src/lib.rs"),
        ("src-tauri/voice-types/Cargo.toml", tauri / "voice-types/Cargo.toml"),
        ("src-tauri/voice-types/src/lib.rs", tauri / "voice-types/src/lib.rs"),
        ("src-tauri/resources/voice-runtime-build/headers", RUNTIME / role / "include"),
        ("src-tauri/resources/voice-runtime-build-recipe.json", tauri / "resources" / "voice-runtime-build-recipe.json"),
        ("scripts/build_voice_workers.py", REPO / "scripts" / "build_voice_workers.py"),
        ("scripts/voice_worker_build.rs", REPO / "scripts" / "voice_worker_build.rs"),
        ("src-tauri/voice-parent-auth", tauri / "voice-parent-auth"),
        ("src-tauri/native/worker_bridge.cpp", native / definition["bridge"]),
        ("src-tauri/native/worker_bridge.h", native / definition["bridge_header"]),
        ("src-tauri/voice-worker/Cargo.toml", package / "Cargo.toml"),
        ("src-tauri/voice-worker/build.rs", package / "build.rs"),
        ("src-tauri/voice-worker/src/main.rs", package / "src" / "main.rs"),
        ("src-tauri/voice-worker/src/delegate.rs", package / "src" / definition["delegate"]),
    ]
    roster: list[tuple[str, str]] = []
    files: list[dict[str, str]] = []
    for logical, path in sources:
        file_sha = tree_hash(path) if path.is_dir() else file_sha256(path)
        roster.append((logical, file_sha))
        files.append({"path": logical, "sha256": file_sha})
    owner_key_sha256 = sha256(owner_public_key)
    owner_key_path = "src-tauri/generated/trusted_owner_public_key.x963"
    owner_pin_path = "src-tauri/generated/voice-owner-key-pin.json"
    roster.extend(((owner_key_path, owner_key_sha256), (owner_pin_path, owner_key_pin_sha256)))
    files.extend((
        {"path": owner_key_path, "sha256": owner_key_sha256},
        {"path": owner_pin_path, "sha256": owner_key_pin_sha256},
    ))
    source_seal = hash_roster(roster)
    resource_seal = hash_roster([
        ("receipt", sha256(receipt_bytes)),
        ("archive", archive_sha),
        ("headers", header_seal),
        ("promoted-tree", promoted_tree),
        ("runtime-recipe", receipt["recipe_sha256"]),
        ("runtime-source-generation", receipt["source_manifest_sha256"]),
        ("trusted-owner-public-key-x963", owner_key_sha256),
        ("owner-key-pin-record", owner_key_pin_sha256),
    ])
    return source_seal, resource_seal, files


def checked_identity_inspector(inspector: Path, code: Path) -> dict[str, str]:
    result = subprocess.run(
        [str(inspector), str(code)],
        check=True,
        capture_output=True,
        text=True,
    )
    identity = json.loads(result.stdout)
    if not isinstance(identity, dict) or set(identity) != {
        "team_identifier",
        "identifier",
        "designated_requirement_sha256",
    }:
        raise ValueError("signed-code inspector returned an unexpected identity")
    if (
        not isinstance(identity["team_identifier"], str)
        or not re.fullmatch(r"[A-Z0-9]{10}", identity["team_identifier"])
        or not isinstance(identity["identifier"], str)
        or not identity["identifier"]
    ):
        raise ValueError("signed-code inspector returned an invalid code identity")
    require_sha(identity["designated_requirement_sha256"], "designated requirement SHA")
    return identity


def approved_signing_identity(environment: dict[str, str]) -> str:
    identity = environment.get("APPLE_SIGNING_IDENTITY", "")
    if not identity or identity == "-":
        raise ValueError("APPLE_SIGNING_IDENTITY must name the approved Apple Development identity")
    result = subprocess.run(
        ["/usr/bin/security", "find-identity", "-v", "-p", "codesigning"],
        check=True,
        capture_output=True,
        text=True,
    )
    development = []
    for line in result.stdout.splitlines():
        match = re.fullmatch(r'\s*\d+\)\s+([0-9A-F]{40})\s+"([^"]+)"', line)
        if match and match.group(2).startswith("Apple Development:"):
            development.append((match.group(1), match.group(2)))
    if len(development) != 1 or identity not in development[0]:
        raise ValueError("the configured signing identity is not the single approved Apple Development identity")
    return identity


def sign_worker(
    code: Path,
    role: str,
    identity: str,
    inspector: Path,
) -> dict[str, str]:
    expected_identifier = f"dev.rohtolos.saturday.voice.{role}-worker"
    subprocess.run(
        [
            "/usr/bin/codesign",
            "--force",
            "--options",
            "runtime",
            "--sign",
            identity,
            "--identifier",
            expected_identifier,
            str(code),
        ],
        check=True,
    )
    subprocess.run(
        ["/usr/bin/codesign", "--verify", "--strict", "--verbose=2", str(code)],
        check=True,
    )
    signed_entitlements = subprocess.run(
        ["/usr/bin/codesign", "-d", "--entitlements", ":-", str(code)],
        check=True,
        capture_output=True,
    )
    entitlement_bytes = signed_entitlements.stdout.strip()
    entitlements = plistlib.loads(entitlement_bytes) if entitlement_bytes else {}
    if entitlements != {}:
        raise ValueError(f"{role} worker must have no custom entitlements")
    final = checked_identity_inspector(inspector, code)
    if final["identifier"] != expected_identifier:
        raise ValueError(f"{role} worker has an unexpected signed identifier")
    return final


def regular_resource_record(path: Path, bundle_path: str) -> dict:
    metadata = path.lstat()
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
        raise ValueError(f"invalid generated voice resource: {path}")
    return {
        "path": bundle_path,
        "bytes": metadata.st_size,
        "mode": stat.S_IMODE(metadata.st_mode),
        "sha256": file_sha256(path),
    }


def main() -> int:
    if sys.platform != "darwin" or platform.machine() != "arm64":
        raise SystemExit("voice worker build requires the reviewed arm64 macOS host")
    owner_public_key, owner_key_pin_sha256 = trusted_owner_public_key()
    private_parent = tempfile.TemporaryDirectory(prefix="saturday-voice-worker-build-")
    parent = Path(private_parent.name).resolve(strict=True)
    if not parent.is_dir() or stat.S_IMODE(parent.stat().st_mode) != 0o700:
        raise SystemExit("internal worker build directory is not private")
    if not RECEIPT.is_file() or RECEIPT.is_symlink():
        raise SystemExit("reviewed runtime build receipt is missing")
    receipt_bytes = RECEIPT.read_bytes()
    receipt = json.loads(receipt_bytes)
    for role in ROLES:
        reviewed_provider(role, receipt)
    destination = TAURI / "resources" / "voice-workers"
    if destination.exists() or destination.is_symlink():
        raise SystemExit(f"refusing to overwrite existing worker artifacts: {destination}")
    signing_identity = approved_signing_identity(os.environ)
    with tempfile.TemporaryDirectory(prefix="voice-worker-delivery-", dir=parent) as work_dir:
        work_root = Path(work_dir)
        target_dir = work_root / "target"
        staging = work_root / "voice-workers"
        staging.mkdir(mode=0o700)
        cargo_home = work_root / "cargo-home"
        cargo_home.mkdir(mode=0o700)
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(parent),
            "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
            "CARGO_HOME": str(cargo_home),
            "CARGO_TARGET_DIR": str(target_dir),
            "SATURDAY_VOICE_TRUSTED_OWNER_PUBLIC_KEY_X963_HEX": owner_public_key.hex(),
            "SATURDAY_VOICE_TRUSTED_OWNER_PUBLIC_KEY_SHA256": sha256(owner_public_key),
        }
        for role, definition in ROLES.items():
            subprocess.run([
                "cargo", "build", "--locked", "--release", "--target", TARGET,
                "--manifest-path", str(TAURI / "Cargo.toml"), "-p", definition["package"],
            ], cwd=REPO, env=env, check=True)
            built = target_dir / TARGET / "release" / definition["binary"]
            role_dir = staging / role
            role_dir.mkdir(mode=0o700)
            output = role_dir / definition["binary"]
            shutil.copyfile(built, output)
            output.chmod(0o700)
        subprocess.run(
            [
                "cargo",
                "build",
                "--locked",
                "--release",
                "--target",
                TARGET,
                "--manifest-path",
                str(TAURI / "Cargo.toml"),
                "-p",
                "saturday-voice-parent-auth",
                "--features",
                "inspect-signed-code",
                "--bin",
                "voice-parent-auth-inspect",
            ],
            cwd=REPO,
            env=env,
            check=True,
        )
        inspector = target_dir / TARGET / "release" / "voice-parent-auth-inspect"
        role_records = []
        source_generations = []
        runtime_workers = []
        for role, definition in ROLES.items():
            provider, archive, header_seal, promoted_tree = reviewed_provider(role, receipt)
            archive_sha = file_sha256(archive)
            source_seal, resource_seal, files = seal_inputs(
                role, definition, archive, receipt_bytes, archive_sha, header_seal, promoted_tree, receipt,
                owner_public_key, owner_key_pin_sha256)
            identity = sign_worker(
                binary,
                role,
                signing_identity,
                inspector,
            )
            binary.chmod(0o500)
            binary_sha = file_sha256(binary)
            binary_bytes = binary.stat().st_size
            source_generation = require_sha(provider.get("source_seal"), f"{role} source generation SHA")
            source_tree = promoted_tree
            source_revision = provider.get("source_commit")
            if not isinstance(source_revision, str) or not re.fullmatch(r"[0-9a-f]{40}", source_revision):
                raise ValueError(f"{role} source revision is invalid")
            source_generations.append({
                "role": role,
                "source_revision": source_revision,
                "source_tree_sha256": source_tree,
                "source_generation_sha256": source_generation,
            })
            team_sha = sha256(identity["team_identifier"].encode("utf-8"))
            role_records.append({
                "role": role,
                "package": definition["package"],
                "bundle_path": f"voice-workers/{role}/{definition['binary']}",
                "binary_bytes": binary_bytes,
                "binary_sha256": binary_sha,
                "team_id_sha256": team_sha,
                "designated_requirement_sha256": identity["designated_requirement_sha256"],
                "source_seal_sha256": source_seal,
                "resource_seal_sha256": resource_seal,
                "runtime_provider": provider["provider"],
                "runtime_archive_sha256": archive_sha,
                "source_files": files,
            })
            runtime_workers.append({
                "role": role,
                "asset_id": "whisper-base" if role == "asr" else "qwen3-4b-q4-k-m",
                "path": f"voice-workers/{role}/{definition['binary']}",
                "bytes": binary_bytes,
                "sha256": binary_sha,
                "team_id_sha256": team_sha,
                "designated_requirement_sha256": identity["designated_requirement_sha256"],
                "source_seal_sha256": source_seal,
                "resource_seal_sha256": resource_seal,
                "max_rss_bytes": 4 * 1024 * 1024 * 1024 if role == "asr" else 8 * 1024 * 1024 * 1024,
            })
            notices = provider.get("notices")
            if not isinstance(notices, list):
                raise ValueError(f"{role} legal notice roster is missing")
            for notice in notices:
                relative = Path(notice["path"])
                if not relative.parts or relative.parts[0] != "notices" or ".." in relative.parts:
                    raise ValueError(f"invalid {role} legal notice path")
                source = RUNTIME / role / relative
                if require_sha(notice.get("sha256"), f"{role} notice SHA") != file_sha256(source):
                    raise ValueError(f"{role} notice changed after review")
                target = staging / "notices" / role / Path(*relative.parts[1:])
                target.parent.mkdir(parents=True, exist_ok=True, mode=0o755)
                shutil.copyfile(source, target)
                target.chmod(0o444)
        artifact_manifest = {
            "schema": "saturday.voice-worker-artifacts.v1",
            "state": "signed_workers_ready_for_owner_binding",
            "target": TARGET,
            "runtime_receipt_sha256": sha256(receipt_bytes),
            "roles": role_records,
            "signing": "single_approved_apple_development_identity",
        }
        artifact_manifest_path = staging / "worker-artifact-manifest.json"
        artifact_manifest_path.write_text(
            json.dumps(artifact_manifest, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        artifact_manifest_path.chmod(0o444)
        runtime_resources = [
            regular_resource_record(
                artifact_manifest_path,
                "voice-workers/worker-artifact-manifest.json",
            )
        ]
        notices_root = staging / "notices"
        for path in sorted(notices_root.rglob("*")):
            if path.is_dir():
                continue
            relative = path.relative_to(staging).as_posix()
            runtime_resources.append(regular_resource_record(path, f"voice-workers/{relative}"))
        runtime_manifest = {
            "schema": "saturday.voice.runtime-resource-closure.v1",
            "source_generations": source_generations,
            "workers": runtime_workers,
            "resources": sorted(runtime_resources, key=lambda item: item["path"]),
        }
        runtime_manifest_path = staging / "voice-runtime-manifest.json"
        runtime_manifest_path.write_text(
            json.dumps(runtime_manifest, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        runtime_manifest_path.chmod(0o444)
        if destination.exists() or destination.is_symlink():
            raise SystemExit(f"refusing to overwrite existing worker artifacts: {destination}")
        os.rename(staging, destination)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
