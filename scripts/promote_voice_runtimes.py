#!/usr/bin/env python3
"""Promote the exactly reviewed voice runtime archives into worker build inputs."""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import tempfile

REPO = Path(__file__).resolve().parents[1]
RESOURCES = REPO / "src-tauri/resources"
DESTINATION = RESOURCES / "voice-runtime-build"
SOURCE_RECEIPT_SHA256 = "f6ae8c823a400e112de2859e910783bcb03801d9f86d0c556cb6785a9a9960b9"
IMAGE_SHA256 = "325521f8e26cb914d133e7c0c745930e06a33a64565b52ec0975791654d597ee"
RECIPE_SHA256 = "c79d253f693ee4c0b777abbfcdd12208ab54df5f6b7dfb1379e076db8acf5164"
SOURCE_MANIFEST_SHA256 = "7a678fc51f3205b5cb1e9fc9c2ed4e6c5da5e6e7dd6df9117fb34559436ab34f"
APPROVAL = {
    "state": "independently_reviewed_and_main_approved",
    "product_readiness": "approved_for_product_use",
    "review_status": "independent_artifact_review_passed",
    "main_approval": True,
}
PROVIDERS = {
    "asr": ("whisper-cpp", "libSaturdayWhisperRuntime.a", 8093656, "98e5f5f2ae4d1a7eb51ccb54dc4a040733a62e42c302b4cdbad9e0b3c9a5ef40", "d1be6fde11ac6e0407606b4e42fe72d34add8037", "6fe2755682242a5c01aba65a22b3c0305d747841", "21dfc56592320dd17736b7b00d95c082e1a7df21e1c7a6a6588c2d3cfb3b1024"),
    "llm": ("llama-cpp", "libSaturdayLlamaRuntime.a", 13554544, "3aa7fe16f8da6577c789f4875014ad831295d8c2ff63d980d499c2460f898624", "8345f333951c661d166b00e6f9362e553768f292", "88538af4019aa9d18026ff43e446c34fb7456adf", "a711d95375d6126a234579595a24a42dbb2a425ae6cee504c6f7e6ab57d3b005"),
}
WARNINGS = ["ggml_src_ggml-cpu_hbm.cpp.o", "ggml_src_ggml-cpu_amx_amx.cpp.o", "ggml_src_ggml-cpu_amx_mmq.cpp.o", "ggml_src_ggml.cpp.o"]


def digest_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def _lstat_regular(path: Path) -> os.stat_result:
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode):
        raise ValueError(f"not a regular file: {path}")
    return info


def _lstat_directory(path: Path) -> os.stat_result:
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode):
        raise ValueError(f"not a real directory: {path}")
    return info


def _safe_file(root: Path, rel: str) -> Path:
    posix = PurePosixPath(rel)
    if posix.is_absolute() or not posix.parts or any(part in ("", ".", "..") for part in posix.parts):
        raise ValueError(f"unsafe roster path: {rel}")
    current = root
    for component in posix.parts[:-1]:
        current = current / component
        _lstat_directory(current)
    target = current / posix.parts[-1]
    _lstat_regular(target)
    return target


def promoted_tree_digest(root: Path) -> str:
    """Byte-compatible with build_voice_workers.promoted_tree_digest."""
    _lstat_directory(root)
    entries = []
    for candidate in root.rglob("*"):
        info = candidate.lstat()
        if stat.S_ISDIR(info.st_mode):
            continue
        if not stat.S_ISREG(info.st_mode):
            raise ValueError(f"symlink or non-regular item in promoted tree: {candidate}")
        rel = candidate.relative_to(root).as_posix()
        entries.append((rel, stat.S_IMODE(info.st_mode), info.st_size, bytes.fromhex(digest_file(candidate))))
    digest = hashlib.sha256()
    for rel, mode, size, file_hash in sorted(entries):
        digest.update(rel.encode("utf-8") + b"\0")
        digest.update(f"{mode:04o}".encode("ascii") + b"\0")
        digest.update(str(size).encode("ascii") + b"\0")
        digest.update(file_hash + b"\n")
    return digest.hexdigest()


def _json(path: Path) -> dict:
    data = path.read_bytes()
    value = json.loads(data, object_pairs_hook=_unique_object)
    if not isinstance(value, dict):
        raise ValueError(f"expected JSON object: {path}")
    return value


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _provider_rosters(receipt: dict, source_manifest: dict) -> dict[str, dict]:
    records = receipt.get("providers")
    if not isinstance(records, list) or len(records) != 2:
        raise ValueError("receipt provider roster mismatch")
    by_role = {record.get("role"): record for record in records if isinstance(record, dict)}
    if set(by_role) != set(PROVIDERS):
        raise ValueError("receipt roles mismatch")
    source_by_id = {item["id"]: item for item in source_manifest.get("runtimes", [])}
    for role, spec in PROVIDERS.items():
        provider, archive_name, size, archive_sha, commit, tree, seal = spec
        record = by_role[role]
        source = source_by_id.get(provider)
        if source is None or record.get("provider") != provider or record.get("source_commit") != commit or record.get("source_tree") != tree or record.get("source_seal") != seal:
            raise ValueError(f"pinned source identity mismatch: {role}")
        if (source.get("commit"), source.get("tree"), source.get("source_seal_sha256")) != (commit, tree, seal):
            raise ValueError(f"repository source identity mismatch: {role}")
        expected_archive = f"output/{role}/archives/{archive_name}"
        if (record.get("archive"), record.get("archive_bytes"), record.get("archive_sha256")) != (expected_archive, size, archive_sha):
            raise ValueError(f"pinned archive mismatch: {role}")
        if not isinstance(record.get("headers"), list) or not record["headers"] or not isinstance(record.get("notices"), list) or not record["notices"]:
            raise ValueError(f"missing headers/notices roster: {role}")
        source_notice_paths = {"notices/" + p for p in source["license_files"]}
        if {item.get("path") for item in record["notices"]} != source_notice_paths:
            raise ValueError(f"notice roster differs from pinned source manifest: {role}")
        if any(not isinstance(item, dict) or not isinstance(item.get("path"), str) or not isinstance(item.get("sha256"), str) for item in record["headers"] + record["notices"]):
            raise ValueError(f"invalid file roster: {role}")
    return by_role


def _validate_source(root: Path) -> tuple[bytes, dict, dict]:
    absolute = Path(os.path.abspath(root))
    for component in (absolute, *absolute.parents):
        if component.is_symlink():
            raise ValueError(f"symlink in artifact-root path: {component}")
    if not root.is_dir():
        raise ValueError("artifact root must be a real directory")
    receipt_path = _safe_file(root, "build-artifact-manifest.json")
    receipt_bytes = receipt_path.read_bytes()
    if hashlib.sha256(receipt_bytes).hexdigest() != SOURCE_RECEIPT_SHA256:
        raise ValueError("source receipt digest is not the reviewed receipt")
    receipt = json.loads(receipt_bytes, object_pairs_hook=_unique_object)
    if receipt.get("schema") != "saturday.voice-runtime-artifacts.v1" or receipt.get("state") != "built_unreviewed_not_ready_for_product_use":
        raise ValueError("source receipt schema/state mismatch")
    if receipt.get("recipe_sha256") != RECIPE_SHA256 or receipt.get("source_manifest_sha256") != SOURCE_MANIFEST_SHA256:
        raise ValueError("source recipe/manifest digest mismatch")
    if receipt.get("workspace_image") != "workspace.sparseimage":
        raise ValueError("unexpected source image identity")
    image = _safe_file(root.parent.parent, "workspace.sparseimage")
    if digest_file(image) != IMAGE_SHA256:
        raise ValueError("source image digest mismatch")
    manifest_path = RESOURCES / "voice-runtime-sources.json"
    recipe_path = RESOURCES / "voice-runtime-build-recipe.json"
    if digest_file(manifest_path) != SOURCE_MANIFEST_SHA256 or digest_file(recipe_path) != RECIPE_SHA256:
        raise ValueError("repository source manifest/recipe differs from pinned review")
    source_manifest = _json(manifest_path)
    recipe = _json(recipe_path)
    if recipe.get("schema") != "saturday.voice-runtime-build-recipe.v1":
        raise ValueError("unexpected recipe schema")
    by_role = _provider_rosters(receipt, source_manifest)
    for role, record in by_role.items():
        provider, archive_name, size, archive_sha, *_ = PROVIDERS[role]
        expected_files = {"build-artifact-manifest.json"}
        archive_rel = f"{role}/archives/{archive_name}"
        archive = _safe_file(root, archive_rel)
        info = archive.stat()
        if info.st_size != size or digest_file(archive) != archive_sha:
            raise ValueError(f"archive verification failed: {role}")
        expected_files.add(archive_rel)
        for category in ("headers", "notices"):
            for item in record[category]:
                rel = f"{role}/{item['path']}"
                path = _safe_file(root, rel)
                if digest_file(path) != item["sha256"]:
                    raise ValueError(f"rostered file digest mismatch: {rel}")
                expected_files.add(rel)
        actual_files = set()
        for path in root.rglob("*"):
            info = path.lstat()
            if stat.S_ISDIR(info.st_mode):
                continue
            if not stat.S_ISREG(info.st_mode):
                raise ValueError(f"symlink or non-regular source item: {path}")
            actual_files.add(path.relative_to(root).as_posix())
        role_expected = {p for p in expected_files if p == "build-artifact-manifest.json" or p.startswith(role + "/")}
        if not role_expected.issubset(actual_files):
            raise ValueError(f"missing source roster file: {role}")
    expected_all = {"build-artifact-manifest.json"}
    for role, record in by_role.items():
        name = PROVIDERS[role][1]
        expected_all.add(f"{role}/archives/{name}")
        expected_all.update(f"{role}/{item['path']}" for category in ("headers", "notices") for item in record[category])
    actual = {p.relative_to(root).as_posix() for p in root.rglob("*") if stat.S_ISREG(p.lstat().st_mode)}
    if actual != expected_all:
        raise ValueError("source tree contains missing or unrostered files")
    return receipt_bytes, receipt, by_role


def promote(artifact_root: Path) -> Path:
    receipt_bytes, source_receipt, records = _validate_source(artifact_root)
    if RESOURCES.is_symlink() or not RESOURCES.is_dir():
        raise ValueError("resources directory must be real")
    if DESTINATION.exists() or DESTINATION.is_symlink():
        raise FileExistsError(f"destination already exists: {DESTINATION}")
    stage = Path(tempfile.mkdtemp(prefix=".voice-runtime-build-promote-", dir=RESOURCES))
    try:
        os.chmod(stage, 0o700)
        derived = json.loads(receipt_bytes)
        derived["source_receipt_sha256"] = SOURCE_RECEIPT_SHA256
        derived["source_receipt_bytes_base64"] = base64.b64encode(receipt_bytes).decode("ascii")
        derived["review_attestation"] = {
            "source_receipt_sha256": SOURCE_RECEIPT_SHA256,
            "source_image_sha256": IMAGE_SHA256,
            "independent_reviews": ["SourceProof: GREEN", "ContainmentProof: GREEN"],
            "provenance_caveat": "Archive download origin is producer-reported; bytes and staged source tree match the pinned GitHub commit tree, but the original transport chain was not independently authenticated.",
            "symbol_overlap_constraint": "The archives share 2,358 defined global names and MUST remain in separate ASR and LLM worker executables; they MUST NOT be co-linked.",
            "preserved_warnings": [f"libtool: no symbols: {name}" for name in WARNINGS],
            "original_receipt_approval": "The original receipt remains built_unreviewed_not_ready_for_product_use and is not approved; this is a derived product receipt.",
        }
        derived_records = {record["role"]: record for record in derived["providers"]}
        for role, record in records.items():
            derived_record = derived_records[role]
            spec = PROVIDERS[role]
            _, archive_name, _, _, _, _, _ = spec
            role_dir = stage / role
            role_dir.mkdir(mode=0o700)
            entries = [(f"{role}/archives/{archive_name}", role_dir / archive_name)]
            for category in ("headers", "notices"):
                for item in record[category]:
                    entries.append((f"{role}/{item['path']}", role_dir / item["path"]))
            for src_rel, dest in entries:
                src = _safe_file(artifact_root, src_rel)
                source_fd = os.open(src, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
                try:
                    before = os.fstat(source_fd)
                    if not stat.S_ISREG(before.st_mode):
                        raise ValueError(f"source is not a regular file: {src_rel}")
                    path_before = src.lstat()
                    if (before.st_dev, before.st_ino) != (path_before.st_dev, path_before.st_ino):
                        raise ValueError(f"source changed before copy: {src_rel}")
                    dest.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                    copied_hash = hashlib.sha256()
                    with os.fdopen(source_fd, "rb", closefd=False) as source, dest.open("xb") as output:
                        for block in iter(lambda: source.read(1024 * 1024), b""):
                            copied_hash.update(block)
                            output.write(block)
                    os.chmod(dest, stat.S_IMODE(before.st_mode))
                    after = os.fstat(source_fd)
                    path_after = src.lstat()
                    if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns) or (after.st_dev, after.st_ino) != (path_after.st_dev, path_after.st_ino):
                        raise ValueError(f"source changed during copy: {src_rel}")
                finally:
                    os.close(source_fd)
                expected_hash = next((item["sha256"] for cat in ("headers", "notices") for item in record[cat] if f"{role}/{item['path']}" == src_rel), spec[3] if src_rel.endswith(archive_name) else None)
                if copied_hash.hexdigest() != expected_hash or digest_file(dest) != expected_hash:
                    raise ValueError(f"copied file digest mismatch: {src_rel}")
            derived_record["archive"] = f"{role}/{archive_name}"
            derived_record["promoted_resource_root"] = f"src-tauri/resources/voice-runtime-build/{role}"
            derived_record["promoted_tree_sha256"] = promoted_tree_digest(role_dir)
        derived.update(APPROVAL)
        receipt_tmp = stage / "build-artifact-manifest.json"
        receipt_tmp.write_text(json.dumps(derived, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        os.chmod(receipt_tmp, 0o600)
        if DESTINATION.exists() or DESTINATION.is_symlink():
            raise FileExistsError(f"destination appeared during promotion: {DESTINATION}")
        os.rename(stage, DESTINATION)
        return DESTINATION
    except BaseException:
        if stage.exists() and not stage.is_symlink():
            shutil.rmtree(stage)
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact-root", required=True, type=Path)
    args = parser.parse_args()
    try:
        print(promote(args.artifact_root))
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
