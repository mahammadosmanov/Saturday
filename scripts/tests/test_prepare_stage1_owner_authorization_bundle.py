from __future__ import annotations

import hashlib
import json
import os
import shutil
import stat
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts import freeze_stage1_owner_authorization as freezer
from scripts import stage1_owner_roster as roster

try:
    from scripts import prepare_stage1_owner_authorization_bundle as prepare
    from scripts import verify_stage1_owner_authorization_bundle as verifier
except ImportError:
    prepare = None
    verifier = None


ROOT = Path(__file__).resolve().parents[2]
PINNED_ANCHOR = b'{"algorithm":"ecdsa-p256-sha256-x962","key_id":"f0b9315a0459dab7ab5fa6903e733c917761154216cd90e5d979da6c852c602a","public_key_x963_base64":"BBERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERE=","schema":"saturday.local-provider.stage1-owner-public-key.v1","state":"pinned"}'
UNPROVISIONED_ANCHOR = b'{"schema":"saturday.local-provider.stage1-owner-public-key.v1","state":"unprovisioned"}'


class PrepareStage1OwnerAuthorizationBundleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.assertIsNotNone(prepare, "prepare bundle module is absent")
        self.assertIsNotNone(verifier, "verify bundle module is absent")

    def _bootstrap_root(self, destination: Path) -> Path:
        resources = destination / "src-tauri/resources"
        resources.mkdir(parents=True)
        (resources / "stage1-owner-public-key.json").write_bytes(UNPROVISIONED_ANCHOR)
        (resources / "stage1-owner-binding.json").write_bytes(roster.UNFROZEN_BINDING_BYTES)
        os.chmod(resources / "stage1-owner-public-key.json", 0o644)
        os.chmod(resources / "stage1-owner-binding.json", 0o644)
        return destination

    def _final_root(self, destination: Path) -> Path:
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
        freezer.freeze(destination)
        return destination

    def test_package_hooks_and_tauri_resource_mapping_are_exact(self) -> None:
        package = json.loads((ROOT / "package.json").read_text())
        self.assertEqual(
            package["scripts"]["test:stage1-owner-authorization"],
            "python3 -m unittest scripts.tests.test_stage1_owner_authorization scripts.tests.test_pin_stage1_owner_public_key scripts.tests.test_freeze_stage1_owner_authorization scripts.tests.test_prepare_stage1_owner_authorization_bundle -v",
        )
        self.assertEqual(
            package["scripts"]["prepare:stage1-owner-authorization"],
            "python3 scripts/prepare_stage1_owner_authorization_bundle.py",
        )
        self.assertEqual(
            package["scripts"]["verify:stage1-owner-authorization"],
            "python3 scripts/verify_stage1_owner_authorization_bundle.py",
        )
        configuration = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())
        self.assertEqual(
            configuration["build"]["beforeDevCommand"],
            "npm run prepare:stage1-owner-authorization && npm run build:native-helper && npm run dev",
        )
        self.assertEqual(
            configuration["build"]["beforeBuildCommand"],
            "npm run prepare:stage1-owner-authorization && npm run build:native-helper && npm run build",
        )
        self.assertEqual(
            configuration["bundle"]["resources"],
            {
                "generated/helper-identity.json": "saturday-resources/helper-identity.json",
                "generated/stage1": "saturday-resources/stage1",
            },
        )

    def test_bootstrap_mode_contains_only_exact_records_and_marker(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self._bootstrap_root(Path(temporary))
            result = prepare.prepare(root)
            generated = root / "src-tauri/generated/stage1"
            self.assertEqual(result["mode"], "bootstrap")
            self.assertEqual(
                {path.name for path in generated.iterdir()},
                {
                    "bootstrap-only.json",
                    "stage1-owner-binding.json",
                    "stage1-owner-public-key.json",
                },
            )
            self.assertEqual(
                (generated / "bootstrap-only.json").read_bytes(),
                prepare.BOOTSTRAP_MARKER_BYTES,
            )
            self.assertEqual(
                (generated / "stage1-owner-binding.json").read_bytes(),
                roster.UNFROZEN_BINDING_BYTES,
            )
            self.assertEqual(
                (generated / "stage1-owner-public-key.json").read_bytes(),
                UNPROVISIONED_ANCHOR,
            )
            self.assertEqual(verifier.verify_prepared(root, final=False)["mode"], "bootstrap")
            with self.assertRaisesRegex(ValueError, "final"):
                verifier.verify_prepared(root, final=True)

    def test_final_mode_copies_exact_closed_roster_and_verifies_app_bundle(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self._final_root(Path(temporary))
            result = prepare.prepare(root)
            generated = root / "src-tauri/generated/stage1"
            self.assertEqual(result["mode"], "final")
            self.assertEqual(
                {path.relative_to(generated).as_posix() for path in generated.rglob("*") if path.is_file()},
                set(prepare.FINAL_FILE_MAP),
            )
            self.assertEqual(stat.S_IMODE(generated.lstat().st_mode), 0o755)
            for directory in (path for path in generated.rglob("*") if path.is_dir()):
                self.assertEqual(stat.S_IMODE(directory.lstat().st_mode), 0o755)
            for relative, source_relative in prepare.FINAL_FILE_MAP.items():
                destination = generated / relative
                source = root / source_relative
                metadata = destination.lstat()
                self.assertTrue(stat.S_ISREG(metadata.st_mode))
                self.assertEqual(metadata.st_nlink, 1)
                self.assertEqual(stat.S_IMODE(metadata.st_mode), 0o644)
                self.assertEqual(destination.read_bytes(), source.read_bytes())
                self.assertEqual(
                    hashlib.sha256(destination.read_bytes()).hexdigest(),
                    hashlib.sha256(source.read_bytes()).hexdigest(),
                )
            self.assertEqual(verifier.verify_prepared(root, final=True)["mode"], "final")

            app = root / "fake/Saturday.app"
            bundled = app / "Contents/Resources/saturday-resources/stage1"
            bundled.parent.mkdir(parents=True)
            shutil.copytree(generated, bundled, copy_function=shutil.copy2)
            self.assertEqual(
                verifier.verify(app, root, final=True)["stage1_resource_count"],
                len(prepare.FINAL_FILE_MAP),
            )

    def test_final_mode_rejects_stale_source_sidecar_wrong_anchor_and_dirty_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self._final_root(Path(temporary))
            prepare.prepare(root)
            generated = root / "src-tauri/generated/stage1"

            stale = root / roster.BASE_REVIEW_ROSTER[0]
            stale.write_bytes(stale.read_bytes() + b"\nstale")
            with self.assertRaises(ValueError):
                prepare.prepare(root)
            stale.write_bytes((ROOT / roster.BASE_REVIEW_ROSTER[0]).read_bytes())

            sidecar = root / roster.HARNESS_REVIEW_SIDECAR_PATH
            sidecar.write_bytes(b"0" * 64 + b"  harness-review-manifest.json\n")
            with self.assertRaises(ValueError):
                prepare.prepare(root)
            sidecar.write_bytes(
                f"{hashlib.sha256((root / roster.HARNESS_REVIEW_MANIFEST_PATH).read_bytes()).hexdigest()}  harness-review-manifest.json\n".encode("ascii")
            )

            (root / roster.TRUST_ANCHOR_PATH).write_bytes(UNPROVISIONED_ANCHOR)
            with self.assertRaises(ValueError):
                prepare.prepare(root)
            (root / roster.TRUST_ANCHOR_PATH).write_bytes(PINNED_ANCHOR)

            os.chmod(generated, 0o700)
            unexpected = generated / "unexpected"
            unexpected.write_bytes(b"dirty")
            with self.assertRaisesRegex(ValueError, "prepared destination"):
                prepare.prepare(root)

    def test_rejects_generated_symlink_and_copy_time_source_change(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self._bootstrap_root(Path(temporary))
            generated = root / "src-tauri/generated/stage1"
            generated.parent.mkdir(parents=True)
            outside = root / "outside"
            outside.mkdir()
            generated.symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "symlink"):
                prepare.prepare(root)

        with tempfile.TemporaryDirectory() as temporary:
            root = self._final_root(Path(temporary))
            original = prepare._copy_regular
            mutated = False

            def mutate_once(source: Path, destination: Path) -> None:
                nonlocal mutated
                original(source, destination)
                if not mutated:
                    mutated = True
                    source.write_bytes(source.read_bytes() + b"\nchanged-during-copy")

            with mock.patch.object(prepare, "_copy_regular", side_effect=mutate_once):
                with self.assertRaisesRegex(ValueError, "changed while copying"):
                    prepare.prepare(root)
            self.assertFalse((root / "src-tauri/generated/stage1").exists())


if __name__ == "__main__":
    unittest.main()
