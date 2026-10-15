from __future__ import annotations

import base64
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts import promote_voice_runtimes as promoter


class PromoteVoiceRuntimesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        base = Path(self.temp.name).resolve()
        self.container = base / "container"
        workspace = self.container / "mounted-workspace"
        self.root = workspace / "output"
        self.root.mkdir(parents=True)
        self.image = self.container / "workspace.sparseimage"
        self.image.write_bytes(b"synthetic immutable image bytes")
        self.resources = base / "resources"
        self.resources.mkdir()
        self.destination = self.resources / "voice-runtime-build"
        self.specs = {
            "asr": ("test-whisper", "libSaturdayWhisperRuntime.a", b"asr archive bytes", "asr-commit", "asr-tree", "asr-seal"),
            "llm": ("test-llama", "libSaturdayLlamaRuntime.a", b"llm archive bytes", "llm-commit", "llm-tree", "llm-seal"),
        }
        source_runtimes = []
        providers = []
        for role, (provider, archive_name, archive_bytes, commit, tree, seal) in self.specs.items():
            archive_rel = f"{role}/archives/{archive_name}"
            archive = self.root / archive_rel
            archive.parent.mkdir(parents=True)
            archive.write_bytes(archive_bytes)
            header_path = "include/provider.h"
            notice_path = "notices/LICENSE"
            header_data = f"{role} public header\n".encode()
            notice_data = f"{role} license notice\n".encode()
            (self.root / role / header_path).parent.mkdir(parents=True)
            (self.root / role / header_path).write_bytes(header_data)
            (self.root / role / notice_path).parent.mkdir(parents=True)
            (self.root / role / notice_path).write_bytes(notice_data)
            source_runtimes.append({
                "id": provider,
                "commit": commit,
                "tree": tree,
                "source_seal_sha256": seal,
                "license_files": ["LICENSE"],
            })
            providers.append({
                "role": role,
                "provider": provider,
                "archive": f"output/{archive_rel}",
                "archive_bytes": len(archive_bytes),
                "archive_sha256": hashlib.sha256(archive_bytes).hexdigest(),
                "source_commit": commit,
                "source_tree": tree,
                "source_seal": seal,
                "headers": [{"path": header_path, "sha256": hashlib.sha256(header_data).hexdigest()}],
                "notices": [{"path": notice_path, "sha256": hashlib.sha256(notice_data).hexdigest()}],
                "promoted_resource_root": f"src-tauri/resources/voice-runtime-build/{role}",
                "promoted_tree_sha256": "unpromoted",
            })
        manifest_bytes = json.dumps({"schema": "saturday.voice-runtime-sources.v1", "runtimes": source_runtimes}, sort_keys=True, separators=(",", ":")).encode()
        recipe_bytes = json.dumps({"schema": "saturday.voice-runtime-build-recipe.v1"}, sort_keys=True, separators=(",", ":")).encode()
        (self.resources / "voice-runtime-sources.json").write_bytes(manifest_bytes)
        (self.resources / "voice-runtime-build-recipe.json").write_bytes(recipe_bytes)
        self.manifest_sha = hashlib.sha256(manifest_bytes).hexdigest()
        self.recipe_sha = hashlib.sha256(recipe_bytes).hexdigest()
        receipt = {
            "schema": "saturday.voice-runtime-artifacts.v1",
            "state": "built_unreviewed_not_ready_for_product_use",
            "product_readiness": "requires independent artifact review and native purpose binding",
            "workspace_image": "workspace.sparseimage",
            "recipe_sha256": self.recipe_sha,
            "source_manifest_sha256": self.manifest_sha,
            "providers": providers,
        }
        self.receipt_bytes = (json.dumps(receipt, sort_keys=True, separators=(",", ":")) + "\n").encode()
        (self.root / "build-artifact-manifest.json").write_bytes(self.receipt_bytes)
        self.expected_providers = {
            role: (provider, archive, len(archive_data), hashlib.sha256(archive_data).hexdigest(), commit, tree, seal)
            for role, (provider, archive, archive_data, commit, tree, seal) in self.specs.items()
        }
        self.patches = [
            patch.object(promoter, "RESOURCES", self.resources),
            patch.object(promoter, "DESTINATION", self.destination),
            patch.object(promoter, "SOURCE_RECEIPT_SHA256", hashlib.sha256(self.receipt_bytes).hexdigest()),
            patch.object(promoter, "IMAGE_SHA256", hashlib.sha256(self.image.read_bytes()).hexdigest()),
            patch.object(promoter, "RECIPE_SHA256", self.recipe_sha),
            patch.object(promoter, "SOURCE_MANIFEST_SHA256", self.manifest_sha),
            patch.object(promoter, "PROVIDERS", self.expected_providers),
        ]
        for patcher in self.patches:
            patcher.start()
            self.addCleanup(patcher.stop)

    def test_rejects_changed_archive_header_and_notice(self):
        for relative in (
            "asr/archives/libSaturdayWhisperRuntime.a",
            "asr/include/provider.h",
            "llm/notices/LICENSE",
        ):
            with self.subTest(path=relative):
                path = self.root / relative
                original = path.read_bytes()
                path.write_bytes(original + b"tampered")
                with self.assertRaises(ValueError):
                    promoter._validate_source(self.root)
                path.write_bytes(original)
                promoter._validate_source(self.root)

    def test_rejects_source_symlink(self):
        path = self.root / "asr/include/provider.h"
        target = self.root / "asr/include/trusted-header"
        target.write_bytes(path.read_bytes())
        path.unlink()
        path.symlink_to(target)
        with self.assertRaises(ValueError):
            promoter._validate_source(self.root)

    def test_rejects_destination_overwrite_and_symlink(self):
        self.destination.symlink_to(self.container, target_is_directory=True)
        with self.assertRaises(FileExistsError):
            promoter.promote(self.root)
        self.destination.unlink()
        self.destination.mkdir()
        with self.assertRaises(FileExistsError):
            promoter.promote(self.root)

    def test_promotes_worker_paths_digest_and_bound_approval(self):
        original_receipt = (self.root / "build-artifact-manifest.json").read_bytes()
        original_image = self.image.read_bytes()
        destination = promoter.promote(self.root)
        receipt = json.loads((destination / "build-artifact-manifest.json").read_text())
        self.assertEqual((self.root / "build-artifact-manifest.json").read_bytes(), original_receipt)
        self.assertEqual(self.image.read_bytes(), original_image)
        self.assertEqual({key: receipt[key] for key in promoter.APPROVAL}, promoter.APPROVAL)
        self.assertEqual(receipt["source_receipt_sha256"], hashlib.sha256(original_receipt).hexdigest())
        self.assertEqual(base64.b64decode(receipt["source_receipt_bytes_base64"], validate=True), original_receipt)
        self.assertEqual(receipt["review_attestation"]["source_image_sha256"], promoter.IMAGE_SHA256)
        self.assertIn("MUST remain in separate ASR and LLM worker executables", receipt["review_attestation"]["symbol_overlap_constraint"])
        self.assertEqual(len(receipt["review_attestation"]["preserved_warnings"]), 4)
        self.assertEqual([item["archive"] for item in receipt["providers"]], ["asr/libSaturdayWhisperRuntime.a", "llm/libSaturdayLlamaRuntime.a"])
        for role in ("asr", "llm"):
            provider = next(item for item in receipt["providers"] if item["role"] == role)
            self.assertEqual(provider["promoted_tree_sha256"], promoter.promoted_tree_digest(destination / role))
            self.assertTrue((destination / provider["archive"]).is_file())
            names = {item.relative_to(destination / role).as_posix() for item in (destination / role).rglob("*") if item.is_file()}
            self.assertEqual(names, {provider["archive"].split("/", 1)[1], "include/provider.h", "notices/LICENSE"})


if __name__ == "__main__":
    unittest.main()
