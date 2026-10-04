from __future__ import annotations

import base64
import hashlib
import os
from typing import Any, Mapping
from unittest import mock

from scripts import stage1_owner_authorization as auth

PUBLIC_KEY = b"\x04" + bytes(range(1, 65))
KEY_ID = hashlib.sha256(PUBLIC_KEY).hexdigest()
GENERATION = "00112233445566778899aabbccddeeff"
NONCE = bytes(range(32))
NOW = 1_795_000_000


class AcceptingVerifier:
    def verify(self, public_key_x963: bytes, message: bytes, signature_der: bytes) -> bool:
        return True


def _artifacts(candidate: Mapping[str, Any]) -> list[dict[str, Any]]:
    output = []
    for item in [candidate["runtime"], *candidate["dependencies"]]:
        record = {
            "formula": item["formula"],
            "version": item["version"] + (f"_{item['rebuild']}" if item.get("rebuild") else ""),
            "repository": item.get("oci_repository", item.get("repository")),
            "layer_sha256": item.get("layer_digest", item.get("layer")),
            "transfer_bytes": item["layer_bytes"],
        }
        if "installed_bytes" in item:
            record["installed_bytes"] = item["installed_bytes"]
        else:
            record["maximum_extracted_bytes"] = item["maximum_extracted_bytes"]
            record["post_install_execution"] = "forbidden"
        output.append(record)
    return output


def _limits(candidate: Mapping[str, Any]) -> dict[str, Any]:
    provenance = candidate["provenance"]
    return {
        "artifact_count": candidate["authorizations"]["bottle_transfer"]["artifact_count"],
        "formula_execution_allowed": False,
        "maximum_archive_entries": provenance["maximum_entries"],
        "maximum_total_extracted_bytes": provenance["maximum_total_extracted_bytes"],
        "maximum_transfer_bytes": provenance["bottle_transfer_cap_bytes"],
        "minimum_free_bytes_before_each_body_read": provenance["minimum_free_bytes_before_each_transfer"],
        "origin_policy": provenance["origin_policy"],
        "post_install_execution_allowed": False,
        "verified_layer_total_bytes": provenance["verified_layer_total_bytes"],
    }


def verified_authorization(
    candidate: Mapping[str, Any],
    directory_fds: Mapping[str, int],
    *,
    candidate_sha256: str,
    harness_aggregate: str = "44" * 32,
    candidate_review_aggregate: str = "33" * 32,
):
    artifacts = _artifacts(candidate)
    limits = _limits(candidate)
    candidate_binding = {
        "candidate_id": candidate["candidate_id"],
        "candidate_sha256": candidate_sha256,
        "review_aggregate_sha256": candidate_review_aggregate,
    }
    review = {
        "review_aggregate_sha256": harness_aggregate,
        "review_manifest_sha256": "55" * 32,
        "review_sidecar_sha256": "66" * 32,
    }
    binding_value = {
        "artifacts": artifacts,
        "candidate": candidate_binding,
        "limits": limits,
        "model_transfer": "blocked",
        "resource_roster": [{"bytes": 1, "mode": 420, "path": "fixture/resource", "sha256": "00" * 32}],
        "review": review,
        "schema": auth.BINDING_SCHEMA,
        "state": "frozen",
    }
    binding_bytes = auth.canonical_json_bytes(binding_value)
    identities = {name: os.fstat(fd) for name, fd in directory_fds.items()}
    generation = GENERATION
    ident = lambda value: {"device": value.st_dev, "inode": value.st_ino, "uid": value.st_uid}
    payload = {
        "artifacts": artifacts,
        "binding_sha256": hashlib.sha256(binding_bytes).hexdigest(),
        "candidate": candidate_binding,
        "expires_at_unix": NOW + auth.AUTHORIZATION_TTL_SECONDS,
        "generation": generation,
        "issued_at_unix": NOW,
        "ledger": ident(identities["ledger"]),
        "limits": limits,
        "model_transfer": "blocked",
        "nonce_sha256": hashlib.sha256(NONCE).hexdigest(),
        "purpose": "stage1_bottle_transfer",
        "receipt": {**ident(identities["receipt_parent"]), "name": f"stage1-{generation}.json"},
        "review": {"review_aggregate_sha256": harness_aggregate, "review_manifest_sha256": review["review_manifest_sha256"]},
        "schema": auth.PAYLOAD_SCHEMA,
        "state": "granted",
        "workspace_parent": ident(identities["workspace_parent"]),
    }
    envelope = {
        "algorithm": auth.OWNER_SIGNATURE_ALGORITHM,
        "key_id": KEY_ID,
        "payload_base64": base64.b64encode(auth.canonical_json_bytes(payload)).decode("ascii"),
        "schema": auth.ENVELOPE_SCHEMA,
        "signature_base64": "MAA=",
    }
    anchor = auth.canonical_json_bytes({
        "algorithm": auth.OWNER_SIGNATURE_ALGORITHM,
        "key_id": KEY_ID,
        "public_key_x963_base64": base64.b64encode(PUBLIC_KEY).decode("ascii"),
        "schema": auth.PUBLIC_KEY_SCHEMA,
        "state": "pinned",
    })
    patches = (
        mock.patch.object(auth, "CANDIDATE_BINDING", candidate_binding),
        mock.patch.object(auth, "ARTIFACT_BINDINGS", artifacts),
        mock.patch.object(auth, "LIMIT_BINDING", limits),
    )
    with patches[0], patches[1], patches[2]:
        verified = auth.verify_owner_authorization(
            auth.canonical_json_bytes(envelope), NONCE, anchor, binding_bytes,
            identities, NOW, AcceptingVerifier(),
        )
    return verified
