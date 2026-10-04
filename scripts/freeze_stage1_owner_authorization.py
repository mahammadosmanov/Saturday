#!/usr/bin/env python3
"""Deterministically freeze Stage 1 review and owner-binding resources."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import stat
import uuid
from pathlib import Path
from typing import Any

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
    REVIEW_SCHEMA,
    REVOKED_REVIEW_AGGREGATES,
    TRUST_ANCHOR_PATH,
    UNFROZEN_BINDING_BYTES,
    canonical_json_bytes,
    collect_base_records,
    collect_records,
    review_aggregate,
)

CANDIDATE_ID = "qwen35-4b-q4km-homebrew-llamacpp-0.4.1"
CANDIDATE_SHA256 = "b2672f1a00121c09077bc447da7fff720238012daf2ecd645add58c48124a5bb"
CANDIDATE_REVIEW_AGGREGATE = "9178a4708e4e5a834c936fe8f8ff7a76ee3c45e8aee7fcb90bfe9101989e5a8c"
PUBLIC_KEY_SCHEMA = "saturday.local-provider.stage1-owner-public-key.v1"
OWNER_SIGNATURE_ALGORITHM = "ecdsa-p256-sha256-x962"

EXPECTED_ARTIFACTS = [
    {
        "formula": "llama.cpp",
        "installed_bytes": 19_291_937,
        "layer_sha256": "eb6fb1624c1d7efc90d94092d95b120fbdcff8ab3c0fe148ba6108f9c44d3a2e",
        "repository": "homebrew/core/llama.cpp",
        "transfer_bytes": 7_707_135,
        "version": "0.4.1",
    },
    {
        "formula": "ggml",
        "installed_bytes": 4_775_140,
        "layer_sha256": "31e217a029d28065ffff52cdb09fa975a56717216b663fe95a4ff84dcac15640",
        "repository": "homebrew/core/ggml",
        "transfer_bytes": 1_350_818,
        "version": "0.24.0",
    },
    {
        "formula": "libomp",
        "installed_bytes": 1_811_000,
        "layer_sha256": "41257106da80d8d2f35bd2effc1877292b3462b00ebd5e85108c56dd0fdff879",
        "repository": "homebrew/core/libomp",
        "transfer_bytes": 597_172,
        "version": "23.1.1",
    },
    {
        "formula": "openssl@3",
        "installed_bytes": 37_759_406,
        "layer_sha256": "51eb22ee51d7b6e678ef5dc2be234adb46f9f541ac1d98e5ed08c9a7535789dc",
        "repository": "homebrew/core/openssl/3",
        "transfer_bytes": 10_965_156,
        "version": "3.6.4",
    },
    {
        "formula": "ca-certificates",
        "layer_sha256": "37f5c1413f65c05d86e1279d76699f5d27ad77f8126f5776db13f3d64ee3d010",
        "maximum_extracted_bytes": 16_777_216,
        "post_install_execution": "forbidden",
        "repository": "homebrew/core/ca-certificates",
        "transfer_bytes": 112_184,
        "version": "2026-08-13_1",
    },
]
EXPECTED_LIMITS = {
    "artifact_count": 5,
    "formula_execution_allowed": False,
    "maximum_archive_entries": 20_000,
    "maximum_total_extracted_bytes": 536_870_912,
    "maximum_transfer_bytes": 67_108_864,
    "minimum_free_bytes_before_each_body_read": 2_818_572_288,
    "origin_policy": {
        "allowed_blob_redirect_origin": "https://pkg-containers.githubusercontent.com:443",
        "authorization_forwarding": "same-origin-only",
        "content_length_required": True,
        "redirect_limit": 1,
        "registry": "https://ghcr.io:443",
        "reject_redirect_from_blob_origin": True,
        "service": "ghcr.io",
        "strip_authorization_on_cross_origin": True,
        "token_realm": "https://ghcr.io:443/token",
    },
    "post_install_execution_allowed": False,
    "verified_layer_total_bytes": 20_732_465,
}


def freeze(repo_root: Path) -> dict[str, Any]:
    """Atomically write the base manifest, sidecar, then derived binding."""
    root = Path(repo_root)
    _validate_pinned_anchor(root / TRUST_ANCHOR_PATH)
    candidate = _load_candidate_and_review(root)
    base_records = collect_base_records(root)
    aggregate = review_aggregate(base_records)
    if aggregate in REVOKED_REVIEW_AGGREGATES:
        raise ValueError("review aggregate is revoked")
    manifest = {
        "files": base_records,
        "review_aggregate_sha256": aggregate,
        "schema": REVIEW_SCHEMA,
    }
    manifest_bytes = canonical_json_bytes(manifest)
    manifest_sha256 = hashlib.sha256(manifest_bytes).hexdigest()
    sidecar_bytes = (
        f"{manifest_sha256}  {Path(HARNESS_REVIEW_MANIFEST_PATH).name}\n".encode(
            "ascii"
        )
    )
    sidecar_sha256 = hashlib.sha256(sidecar_bytes).hexdigest()

    existing_binding = _load_existing_binding(root / BINDING_PATH)
    if existing_binding is not None and existing_binding.get("state") == "frozen":
        try:
            desired = _derive_binding(
                root,
                candidate,
                manifest_sha256,
                sidecar_sha256,
                aggregate,
                manifest_bytes,
                sidecar_bytes,
            )
            unchanged = (
                _read_regular(root / HARNESS_REVIEW_MANIFEST_PATH) == manifest_bytes
                and _read_regular(root / HARNESS_REVIEW_SIDECAR_PATH) == sidecar_bytes
                and canonical_json_bytes(existing_binding) == canonical_json_bytes(desired)
            )
        except (OSError, ValueError):
            unchanged = False
        if not unchanged:
            raise ValueError("stage1 owner authorization is already frozen")
        return existing_binding

    _atomic_write(root / HARNESS_REVIEW_MANIFEST_PATH, manifest_bytes)
    _atomic_write(root / HARNESS_REVIEW_SIDECAR_PATH, sidecar_bytes)
    binding = _derive_binding(
        root,
        candidate,
        manifest_sha256,
        sidecar_sha256,
        aggregate,
        manifest_bytes,
        sidecar_bytes,
    )
    binding_bytes = canonical_json_bytes(binding)
    _atomic_write(root / BINDING_PATH, binding_bytes)

    if (
        _read_regular(root / HARNESS_REVIEW_MANIFEST_PATH) != manifest_bytes
        or _read_regular(root / HARNESS_REVIEW_SIDECAR_PATH) != sidecar_bytes
        or _read_regular(root / BINDING_PATH) != binding_bytes
    ):
        raise ValueError("frozen outputs failed identity replay")
    return binding


def _derive_binding(
    root: Path,
    candidate: dict[str, Any],
    manifest_sha256: str,
    sidecar_sha256: str,
    aggregate: str,
    manifest_bytes: bytes,
    sidecar_bytes: bytes,
) -> dict[str, Any]:
    if _read_regular(root / HARNESS_REVIEW_MANIFEST_PATH) != manifest_bytes:
        raise ValueError("harness review manifest identity mismatch")
    if _read_regular(root / HARNESS_REVIEW_SIDECAR_PATH) != sidecar_bytes:
        raise ValueError("harness review sidecar identity mismatch")
    artifacts = _candidate_artifacts(candidate)
    limits = _candidate_limits(candidate)
    if artifacts != EXPECTED_ARTIFACTS or limits != EXPECTED_LIMITS:
        raise ValueError("candidate artifact or limit identity mismatch")
    resource_paths = [
        *BASE_REVIEW_ROSTER,
        HARNESS_REVIEW_MANIFEST_PATH,
        HARNESS_REVIEW_SIDECAR_PATH,
    ]
    return {
        "artifacts": artifacts,
        "candidate": {
            "candidate_id": CANDIDATE_ID,
            "candidate_sha256": CANDIDATE_SHA256,
            "review_aggregate_sha256": CANDIDATE_REVIEW_AGGREGATE,
        },
        "limits": limits,
        "model_transfer": "blocked",
        "resource_roster": collect_records(root, resource_paths),
        "review": {
            "review_aggregate_sha256": aggregate,
            "review_manifest_sha256": manifest_sha256,
            "review_sidecar_sha256": sidecar_sha256,
        },
        "schema": BINDING_SCHEMA,
        "state": "frozen",
    }


def _candidate_artifacts(candidate: dict[str, Any]) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for item in [candidate["runtime"], *candidate["dependencies"]]:
        version = item["version"] + (f"_{item['rebuild']}" if item.get("rebuild") else "")
        record = {
            "formula": item["formula"],
            "layer_sha256": item.get("layer_digest", item.get("layer")),
            "repository": item.get("oci_repository", item.get("repository")),
            "transfer_bytes": item["layer_bytes"],
            "version": version,
        }
        if "installed_bytes" in item:
            record["installed_bytes"] = item["installed_bytes"]
        else:
            record["maximum_extracted_bytes"] = item["maximum_extracted_bytes"]
            if item.get("post_install_forbidden") is not True:
                raise ValueError("post-install policy mismatch")
            record["post_install_execution"] = "forbidden"
        records.append(record)
    return records


def _candidate_limits(candidate: dict[str, Any]) -> dict[str, Any]:
    provenance = candidate["provenance"]
    security = candidate["security"]
    if (
        provenance.get("formula_or_post_install_code_execution_allowed") is not False
        or security.get("no_formula_execution") is not True
    ):
        raise ValueError("candidate execution policy mismatch")
    return {
        "artifact_count": candidate["authorizations"]["bottle_transfer"][
            "artifact_count"
        ],
        "formula_execution_allowed": False,
        "maximum_archive_entries": provenance["maximum_entries"],
        "maximum_total_extracted_bytes": provenance[
            "maximum_total_extracted_bytes"
        ],
        "maximum_transfer_bytes": candidate["authorizations"]["bottle_transfer"][
            "maximum_bytes"
        ],
        "minimum_free_bytes_before_each_body_read": provenance[
            "minimum_free_bytes_before_each_transfer"
        ],
        "origin_policy": provenance["origin_policy"],
        "post_install_execution_allowed": False,
        "verified_layer_total_bytes": provenance["verified_layer_total_bytes"],
    }


def _load_candidate_and_review(root: Path) -> dict[str, Any]:
    candidate_bytes = _read_regular(root / CANDIDATE_PATH)
    if hashlib.sha256(candidate_bytes).hexdigest() != CANDIDATE_SHA256:
        raise ValueError("candidate identity mismatch")
    _validate_sha_sidecar(
        root / CANDIDATE_SIDECAR_PATH,
        CANDIDATE_SHA256,
        Path(CANDIDATE_PATH).name,
    )
    candidate = _decode_object(candidate_bytes)
    if candidate.get("candidate_id") != CANDIDATE_ID:
        raise ValueError("candidate identifier mismatch")

    review_bytes = _read_regular(root / CANDIDATE_REVIEW_MANIFEST_PATH)
    review_sha256 = hashlib.sha256(review_bytes).hexdigest()
    _validate_sha_sidecar(
        root / CANDIDATE_REVIEW_SIDECAR_PATH,
        review_sha256,
        Path(CANDIDATE_REVIEW_MANIFEST_PATH).name,
    )
    review = _decode_object(review_bytes)
    if (
        set(review) != {"schema", "review_aggregate_sha256", "files"}
        or review.get("schema") != REVIEW_SCHEMA
        or review.get("review_aggregate_sha256") != CANDIDATE_REVIEW_AGGREGATE
        or not isinstance(review.get("files"), list)
    ):
        raise ValueError("candidate review identity mismatch")
    expected_paths = {
        "docs/spikes/005-prebuilt-runtime-candidate/README.md",
        CANDIDATE_PATH,
        CANDIDATE_SIDECAR_PATH,
    }
    if {item.get("path") for item in review["files"] if isinstance(item, dict)} != expected_paths:
        raise ValueError("candidate review roster mismatch")
    actual_records = collect_records(root, sorted(expected_paths))
    if review["files"] != actual_records or review_aggregate(actual_records) != CANDIDATE_REVIEW_AGGREGATE:
        raise ValueError("candidate review replay mismatch")
    return candidate


def _validate_pinned_anchor(path: Path) -> None:
    payload = _read_regular(path, 4096)
    anchor = _decode_object(payload)
    if canonical_json_bytes(anchor) != payload or set(anchor) != {
        "algorithm",
        "key_id",
        "public_key_x963_base64",
        "schema",
        "state",
    }:
        raise ValueError("owner trust anchor is invalid")
    if (
        anchor["schema"] != PUBLIC_KEY_SCHEMA
        or anchor["state"] != "pinned"
        or anchor["algorithm"] != OWNER_SIGNATURE_ALGORITHM
        or re.fullmatch(r"[0-9a-f]{64}", str(anchor["key_id"])) is None
    ):
        raise ValueError("owner trust anchor is not pinned")
    try:
        public_key = base64.b64decode(
            str(anchor["public_key_x963_base64"]).encode("ascii"), validate=True
        )
    except (ValueError, UnicodeError) as error:
        raise ValueError("owner trust anchor is invalid") from error
    if (
        len(public_key) != 65
        or public_key[0] != 0x04
        or base64.b64encode(public_key).decode("ascii")
        != anchor["public_key_x963_base64"]
        or hashlib.sha256(public_key).hexdigest() != anchor["key_id"]
    ):
        raise ValueError("owner trust anchor is invalid")


def _validate_sha_sidecar(path: Path, digest: str, name: str) -> None:
    expected = f"{digest}  {name}\n".encode("ascii")
    if _read_regular(path, 4096) != expected:
        raise ValueError("SHA-256 sidecar identity mismatch")


def _load_existing_binding(path: Path) -> dict[str, Any] | None:
    try:
        payload = _read_regular(path, 1 << 20)
    except FileNotFoundError:
        return None
    if payload == UNFROZEN_BINDING_BYTES:
        return {"schema": BINDING_SCHEMA, "state": "unfrozen"}
    value = _decode_object(payload)
    if canonical_json_bytes(value) != payload:
        raise ValueError("existing binding is not canonical")
    return value


def _read_regular(path: Path, maximum: int = 16 * 1024 * 1024) -> bytes:
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode) or path.is_symlink() or before.st_nlink != 1:
        raise ValueError("unsafe regular file")
    if before.st_size > maximum:
        raise ValueError("regular file is oversized")
    payload = path.read_bytes()
    after = path.lstat()
    if (
        (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
        != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)
        or len(payload) != before.st_size
    ):
        raise ValueError("regular file changed while reading")
    return payload


def _decode_object(payload: bytes) -> dict[str, Any]:
    def unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        value: dict[str, Any] = {}
        for key, item in pairs:
            if key in value:
                raise ValueError("duplicate JSON key")
            value[key] = item
        return value

    try:
        value = json.loads(
            payload.decode("utf-8"),
            object_pairs_hook=unique,
            parse_constant=lambda _: (_ for _ in ()).throw(ValueError("non-finite JSON")),
        )
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("invalid JSON") from error
    if not isinstance(value, dict):
        raise ValueError("JSON root is not an object")
    return value


def _atomic_write(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.parent / f".{path.name}.tmp-{os.getpid()}-{uuid.uuid4().hex}"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(temporary, flags, 0o600)
    try:
        offset = 0
        while offset < len(payload):
            written = os.write(descriptor, payload[offset:])
            if written <= 0:
                raise OSError("short write")
            offset += written
        os.fchmod(descriptor, 0o644)
        os.fsync(descriptor)
    except BaseException:
        try:
            temporary.unlink()
        except OSError:
            pass
        raise
    finally:
        os.close(descriptor)
    try:
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except BaseException:
        try:
            temporary.unlink()
        except OSError:
            pass
        raise


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    arguments = parser.parse_args(argv)
    freeze(arguments.repo_root)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
