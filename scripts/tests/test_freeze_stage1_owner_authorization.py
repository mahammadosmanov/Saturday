from __future__ import annotations

import hashlib
import json
import os
import shutil
import stat
import tempfile
import unittest
from pathlib import Path

from scripts import freeze_stage1_owner_authorization as freezer
from scripts import prebuilt_runtime_stage1 as stage1
from scripts import stage1_owner_roster as roster


ROOT = Path(__file__).resolve().parents[2]
CANDIDATE_ID = "qwen35-4b-q4km-homebrew-llamacpp-0.4.1"
CANDIDATE_SHA256 = "08767112eaa0111244e5bb38381f8373615627ddb2af788a8fdd82464aa3a592"
CANDIDATE_REVIEW_AGGREGATE = "c034041a5ea9b5203e5e8385b010079764ff3daf13be00d5163e934cc11d447c"
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
PINNED_ANCHOR = b'{"algorithm":"ecdsa-p256-sha256-x962","key_id":"f0b9315a0459dab7ab5fa6903e733c917761154216cd90e5d979da6c852c602a","public_key_x963_base64":"BBERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERE=","schema":"jarvis.local-provider.stage1-owner-public-key.v1","state":"pinned"}'


class FreezeStage1OwnerAuthorizationTests(unittest.TestCase):
    def fixture_root(self, destination: Path) -> Path:
        for relative in roster.BASE_REVIEW_ROSTER:
            source = ROOT / relative
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        anchor = destination / roster.TRUST_ANCHOR_PATH
        anchor.write_bytes(PINNED_ANCHOR)
        os.chmod(anchor, 0o644)
        binding = destination / roster.BINDING_PATH
        binding.parent.mkdir(parents=True, exist_ok=True)
        binding.write_bytes(roster.UNFROZEN_BINDING_BYTES)
        os.chmod(binding, 0o644)
        return destination

    def test_base_roster_is_closed_non_circular_and_records_exact_files(self) -> None:
        forbidden = (
            "harness-review-manifest.json",
            "harness-review-manifest.json.sha256",
            "stage1-owner-binding.json",
        )
        self.assertEqual(tuple(roster.BASE_REVIEW_ROSTER), tuple(sorted(roster.BASE_REVIEW_ROSTER)))
        self.assertEqual(len(roster.BASE_REVIEW_ROSTER), len(set(roster.BASE_REVIEW_ROSTER)))
        self.assertIn("scripts/prepare_stage1_owner_authorization_bundle.py", roster.BASE_REVIEW_ROSTER)
        self.assertIn("scripts/verify_stage1_owner_authorization_bundle.py", roster.BASE_REVIEW_ROSTER)
        self.assertIn("scripts/tests/test_prepare_stage1_owner_authorization_bundle.py", roster.BASE_REVIEW_ROSTER)
        self.assertIn("src-tauri/src/owner_authorization/deletion.rs", roster.BASE_REVIEW_ROSTER)
        self.assertIn("src-tauri/src/owner_authorization/process.rs", roster.BASE_REVIEW_ROSTER)
        self.assertIn("src-tauri/tests/stage1_owner_authorization.rs", roster.BASE_REVIEW_ROSTER)
        for relative in roster.BASE_REVIEW_ROSTER:
            self.assertFalse(any(relative.endswith(value) for value in forbidden), relative)
            self.assertNotIn("src-tauri/generated/", relative)
            self.assertNotIn("src-tauri/target/", relative)
            self.assertNotIn("src-tauri/binaries/", relative)
            self.assertFalse(relative.endswith(".app"), relative)

        with tempfile.TemporaryDirectory() as temporary:
            root = self.fixture_root(Path(temporary))
            records = roster.collect_base_records(root)
            self.assertEqual([item["path"] for item in records], sorted(roster.BASE_REVIEW_ROSTER))
            for record in records:
                path = root / record["path"]
                metadata = path.lstat()
                self.assertTrue(stat.S_ISREG(metadata.st_mode))
                self.assertEqual(metadata.st_nlink, 1)
                self.assertEqual(record["bytes"], metadata.st_size)
                self.assertEqual(record["mode"], stat.S_IMODE(metadata.st_mode))
                self.assertEqual(record["sha256"], hashlib.sha256(path.read_bytes()).hexdigest())
            expected = hashlib.sha256(roster.canonical_json_bytes(records)).hexdigest()
            self.assertEqual(roster.review_aggregate(records), expected)

    def test_freeze_writes_manifest_sidecar_then_exact_derived_binding(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self.fixture_root(Path(temporary))
            binding = freezer.freeze(root)
            manifest_path = root / roster.HARNESS_REVIEW_MANIFEST_PATH
            sidecar_path = root / roster.HARNESS_REVIEW_SIDECAR_PATH
            binding_path = root / roster.BINDING_PATH

            manifest_bytes = manifest_path.read_bytes()
            sidecar_bytes = sidecar_path.read_bytes()
            binding_bytes = binding_path.read_bytes()
            manifest_sha256 = hashlib.sha256(manifest_bytes).hexdigest()
            sidecar_sha256 = hashlib.sha256(sidecar_bytes).hexdigest()
            manifest = json.loads(manifest_bytes)

            self.assertEqual(
                sidecar_bytes,
                f"{manifest_sha256}  harness-review-manifest.json\n".encode("ascii"),
            )
            self.assertEqual(manifest_bytes, roster.canonical_json_bytes(manifest))
            self.assertEqual(binding_bytes, roster.canonical_json_bytes(binding))
            self.assertEqual(manifest["files"], roster.collect_base_records(root))
            self.assertEqual(
                manifest["review_aggregate_sha256"],
                roster.review_aggregate(manifest["files"]),
            )
            self.assertNotIn(manifest["review_aggregate_sha256"], roster.REVOKED_REVIEW_AGGREGATES)

            self.assertEqual(binding["schema"], roster.BINDING_SCHEMA)
            self.assertEqual(binding["state"], "frozen")
            self.assertEqual(binding["model_transfer"], "blocked")
            self.assertEqual(
                binding["candidate"],
                {
                    "candidate_id": CANDIDATE_ID,
                    "candidate_sha256": CANDIDATE_SHA256,
                    "review_aggregate_sha256": CANDIDATE_REVIEW_AGGREGATE,
                },
            )
            self.assertEqual(binding["review"], {
                "review_aggregate_sha256": manifest["review_aggregate_sha256"],
                "review_manifest_sha256": manifest_sha256,
                "review_sidecar_sha256": sidecar_sha256,
            })
            self.assertEqual(binding["artifacts"], EXPECTED_ARTIFACTS)
            self.assertEqual(binding["limits"], EXPECTED_LIMITS)
            self.assertEqual(
                [item["path"] for item in binding["resource_roster"]],
                sorted([*roster.BASE_REVIEW_ROSTER, roster.HARNESS_REVIEW_MANIFEST_PATH, roster.HARNESS_REVIEW_SIDECAR_PATH]),
            )
            self.assertNotIn(roster.BINDING_PATH, {item["path"] for item in binding["resource_roster"]})
            self.assertEqual(freezer.freeze(root), binding)

    def test_frozen_outputs_fail_closed_after_any_roster_change(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self.fixture_root(Path(temporary))
            freezer.freeze(root)
            manifest_path = root / roster.HARNESS_REVIEW_MANIFEST_PATH
            sidecar_path = root / roster.HARNESS_REVIEW_SIDECAR_PATH
            binding_path = root / roster.BINDING_PATH
            before = (manifest_path.read_bytes(), sidecar_path.read_bytes(), binding_path.read_bytes())
            changed = root / roster.BASE_REVIEW_ROSTER[0]
            changed.write_bytes(changed.read_bytes() + b"\nchanged")
            with self.assertRaisesRegex(ValueError, "already frozen"):
                freezer.freeze(root)
            self.assertEqual(
                (manifest_path.read_bytes(), sidecar_path.read_bytes(), binding_path.read_bytes()),
                before,
            )

    def test_stage1_replays_exact_script_relative_resource_roster(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self.fixture_root(Path(temporary))
            binding = freezer.freeze(root)
            resources = stage1.Stage1Resources.from_script(
                root / "scripts/prebuilt_runtime_stage1.py"
            )
            self.assertEqual(resources.root, root.resolve())
            self.assertEqual(resources.binding, root.resolve() / roster.BINDING_PATH)
            stage1._verify_resource_roster(resources.root, binding["resource_roster"])
            source = root / "scripts/prebuilt_runtime_stage1.py"
            source.write_bytes(source.read_bytes() + b"\n# post-freeze mutation\n")
            with self.assertRaisesRegex(ValueError, "resource roster identity"):
                stage1._verify_resource_roster(root, binding["resource_roster"])


if __name__ == "__main__":
    unittest.main()
