import os
import hashlib
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from scripts.build_native_helper import (
    validate_stage1_bundle_resources,
    validate_tauri_resource_configuration,
)
from scripts import freeze_stage1_owner_authorization as freezer
from scripts import prepare_stage1_owner_authorization_bundle as preparer
from scripts import stage1_owner_roster as roster
from scripts.finalize_signed_helper_bundle import finalize
from scripts.verify_signed_helper_bundle import (
    _entitlements,
    _locate_helper,
    _reject_tracked_app_artifacts,
    _requires_hardened_runtime,
    _team_identifier,
    verify,
)

ROOT = Path(__file__).resolve().parents[2]
PINNED_ANCHOR = b'{"algorithm":"ecdsa-p256-sha256-x962","key_id":"f0b9315a0459dab7ab5fa6903e733c917761154216cd90e5d979da6c852c602a","public_key_x963_base64":"BBERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERE=","schema":"saturday.local-provider.stage1-owner-public-key.v1","state":"pinned"}'


class SignedHelperBundleScriptTests(unittest.TestCase):
    def test_verifier_rejects_any_tracked_app_bundle_member(self) -> None:
        clean = SimpleNamespace(stdout=b"src/main.rs\0", stderr=b"")
        with mock.patch("subprocess.run", return_value=clean):
            _reject_tracked_app_artifacts(Path("repo"))
        tracked = SimpleNamespace(
            stdout=b"release/Saturday.app/Contents/MacOS/saturday\0",
            stderr=b"",
        )
        with mock.patch("subprocess.run", return_value=tracked):
            with self.assertRaisesRegex(ValueError, "tracked app bundle"):
                _reject_tracked_app_artifacts(Path("repo"))

    def test_verifier_rejects_stage1_resource_failure_before_codesign(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            app = root / "Candidate.app"
            app.mkdir()
            with mock.patch(
                "scripts.verify_signed_helper_bundle.validate_tauri_resource_configuration"
            ), mock.patch(
                "scripts.verify_signed_helper_bundle.validate_stage1_bundle_resources",
                side_effect=ValueError("Stage 1 resources rejected"),
            ) as resource_validation, mock.patch("subprocess.run") as codesign:
                with self.assertRaisesRegex(ValueError, "Stage 1 resources rejected"):
                    verify(app, root)
            resource_validation.assert_called_once_with(app.resolve(), root.resolve())
            codesign.assert_not_called()

    def test_stage1_resource_root_and_configuration_are_exact_and_duplicate_free(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for relative in roster.BASE_REVIEW_ROSTER:
                source = ROOT / relative
                target = root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, target)
            public_key = root / roster.TRUST_ANCHOR_PATH
            public_key.write_bytes(PINNED_ANCHOR)
            binding = root / roster.BINDING_PATH
            binding.parent.mkdir(parents=True, exist_ok=True)
            binding.write_bytes(roster.UNFROZEN_BINDING_BYTES)
            freezer.freeze(root)
            preparer.prepare(root)
            configuration = root / "src-tauri/tauri.conf.json"

            app = root / "src-tauri/target/release/bundle/macos/Saturday.app"
            resource_root = app / "Contents/Resources/saturday-resources/stage1"
            resource_root.parent.mkdir(parents=True)
            shutil.copytree(
                root / preparer.GENERATED_RELATIVE,
                resource_root,
                copy_function=shutil.copy2,
            )

            validate_tauri_resource_configuration(root)
            baseline = validate_stage1_bundle_resources(app, root)
            self.assertEqual(set(baseline), set(preparer.FINAL_FILE_MAP))

            os.chmod(resource_root, 0o755)
            extra = resource_root / "extra"
            extra.write_bytes(b"no")
            with self.assertRaisesRegex(ValueError, "composition"):
                validate_stage1_bundle_resources(app, root)
            extra.unlink()
            os.chmod(resource_root, 0o555)

            duplicate = app / "Contents/Resources/duplicate/prebuilt_runtime_stage1.py"
            duplicate.parent.mkdir(parents=True)
            duplicate.write_bytes((resource_root / "prebuilt_runtime_stage1.py").read_bytes())
            with self.assertRaisesRegex(ValueError, "duplicate"):
                validate_stage1_bundle_resources(app, root)
            duplicate.unlink()

            bundled_script = resource_root / "prebuilt_runtime_stage1.py"
            os.chmod(bundled_script, 0o644)
            bundled_script.write_bytes(b"mutated")
            os.chmod(bundled_script, 0o444)
            with self.assertRaisesRegex(ValueError, "differs"):
                validate_stage1_bundle_resources(app, root)

            configuration.write_text(
                json.dumps({"bundle": {"resources": {"unexpected": "path"}}})
            )
            with self.assertRaisesRegex(ValueError, "resource configuration"):
                validate_tauri_resource_configuration(root)

    def test_team_identifier_requires_exact_nonempty_team(self) -> None:
        completed = SimpleNamespace(
            stdout="", stderr="Identifier=helper\nTeamIdentifier=ABCDEFGHIJ\n"
        )
        with mock.patch("subprocess.run", return_value=completed):
            self.assertEqual(_team_identifier(Path("helper")), "ABCDEFGHIJ")
        completed.stderr = "Identifier=helper\nTeamIdentifier=not set\n"
        with mock.patch("subprocess.run", return_value=completed):
            with self.assertRaises(ValueError):
                _team_identifier(Path("helper"))

    def test_hardened_runtime_and_entitlements_are_exact(self) -> None:
        runtime = SimpleNamespace(
            stdout="",
            stderr=(
                "CodeDirectory v=20500 flags=0x10000(runtime)\n"
                "TeamIdentifier=ABCDEFGHIJ\n"
            ),
        )
        with mock.patch("subprocess.run", return_value=runtime):
            self.assertEqual(
                _requires_hardened_runtime(Path("helper")), "ABCDEFGHIJ"
            )
        no_runtime = SimpleNamespace(
            stdout="", stderr="flags=0x0(none)\nTeamIdentifier=ABCDEFGHIJ\n"
        )
        with mock.patch("subprocess.run", return_value=no_runtime):
            with self.assertRaises(ValueError):
                _requires_hardened_runtime(Path("helper"))
        empty_plist = SimpleNamespace(
            stdout=b'<?xml version="1.0"?><plist version="1.0"><dict/></plist>',
            stderr=b"",
        )
        with mock.patch("subprocess.run", return_value=empty_plist):
            self.assertEqual(_entitlements(Path("helper")), {})

    def test_helper_location_is_exact_and_unique(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            app = Path(temporary) / "Candidate.app"
            expected = app / "Contents/MacOS/saturday-runtime-helper"
            expected.parent.mkdir(parents=True)
            expected.write_bytes(b"helper")
            self.assertEqual(_locate_helper(app), expected)
            duplicate = app / "Contents/Resources/saturday-runtime-helper"
            duplicate.parent.mkdir(parents=True)
            duplicate.write_bytes(b"duplicate")
            with self.assertRaises(ValueError):
                _locate_helper(app)

    def test_helper_location_rejects_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            app = Path(temporary) / "Candidate.app"
            expected = app / "Contents/MacOS/saturday-runtime-helper"
            expected.parent.mkdir(parents=True)
            victim = Path(temporary) / "victim"
            victim.write_bytes(b"helper")
            os.symlink(victim, expected)
            with self.assertRaises(ValueError):
                _locate_helper(app)

    def test_finalizer_rejects_stage1_resource_failure_before_codesign(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tauri = root / "src-tauri"
            entitlements = tauri / "Entitlements.plist"
            entitlements.parent.mkdir(parents=True)
            entitlements.write_text("plist")
            app = tauri / "target/release/bundle/macos/Saturday.app"
            app.mkdir(parents=True)
            with mock.patch(
                "scripts.finalize_signed_helper_bundle.validate_tauri_resource_configuration"
            ), mock.patch(
                "scripts.finalize_signed_helper_bundle.validate_stage1_bundle_resources",
                side_effect=ValueError("Stage 1 resources rejected"),
            ) as resource_validation, mock.patch("subprocess.run") as codesign:
                with self.assertRaisesRegex(ValueError, "Stage 1 resources rejected"):
                    finalize(app, "identity", entitlements)
            resource_validation.assert_called_once_with(app.resolve(), root.resolve())
            codesign.assert_not_called()

    def test_finalizer_rejects_ad_hoc_identity_before_touching_bundle(self) -> None:
        with self.assertRaises(ValueError):
            finalize(Path("missing.app"), "-", Path("missing-entitlements"))
        with self.assertRaises(ValueError):
            finalize(Path("missing.app"), "", Path("missing-entitlements"))

    def test_finalizer_rejects_symlinked_expected_bundle(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tauri = root / "src-tauri"
            tauri.mkdir()
            entitlements = tauri / "Entitlements.plist"
            entitlements.write_text("plist")
            expected = tauri / "target/release/bundle/macos/Saturday.app"
            expected.parent.mkdir(parents=True)
            actual = root / "actual.app"
            actual.mkdir()
            os.symlink(actual, expected)
            with self.assertRaises(ValueError):
                finalize(expected, "identity", entitlements)

    def test_finalizer_uses_exact_staged_bytes_and_rejects_staged_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tauri = root / "src-tauri"
            entitlements = tauri / "Entitlements.plist"
            helper_entitlements = tauri / "HelperEntitlements.plist"
            entitlements.parent.mkdir(parents=True)
            entitlements.write_text("app")
            helper_entitlements.write_text("helper")
            app = tauri / "target/release/bundle/macos/Saturday.app"
            bundled_helper = app / "Contents/MacOS/saturday-runtime-helper"
            identity_path = app / "Contents/Resources/saturday-resources/helper-identity.json"
            bundled_helper.parent.mkdir(parents=True)
            identity_path.parent.mkdir(parents=True)
            bundled_helper.write_bytes(b"substituted")

            staged = tauri / "binaries/saturday-runtime-helper-aarch64-apple-darwin"
            staged.parent.mkdir()
            trusted = bytearray(32)
            trusted[0:4] = bytes.fromhex("cffaedfe")
            trusted[4:8] = (0x0100000C).to_bytes(4, "little")
            staged.write_bytes(trusted)
            identity_path.write_text(
                json.dumps(
                    {
                        "binary_sha256": hashlib.sha256(trusted).hexdigest(),
                        "binary_size": len(trusted),
                        "helper_name": "saturday-runtime-helper",
                        "protocol_version": 1,
                        "schema": "saturday.helper-identity.v1",
                        "source_generation_sha256": "a" * 64,
                        "target_triple": "aarch64-apple-darwin",
                    },
                    sort_keys=True,
                    separators=(",", ":"),
                )
                + "\n"
            )
            helper_before = (
                bundled_helper.read_bytes(),
                bundled_helper.stat().st_mode,
                bundled_helper.stat().st_mtime_ns,
            )
            identity_before = (
                identity_path.read_bytes(),
                identity_path.stat().st_mode,
                identity_path.stat().st_mtime_ns,
            )
            with mock.patch(
                "scripts.finalize_signed_helper_bundle.source_generation",
                return_value="a" * 64,
            ), mock.patch(
                "scripts.finalize_signed_helper_bundle.validate_tauri_resource_configuration"
            ), mock.patch(
                "scripts.finalize_signed_helper_bundle.validate_stage1_bundle_resources",
                return_value={"resource": "sealed"},
            ), mock.patch(
                "subprocess.run",
                side_effect=subprocess.CalledProcessError(1, ["codesign"]),
            ) as rejected_signing:
                with self.assertRaises(subprocess.CalledProcessError):
                    finalize(app, "identity", entitlements)
            self.assertEqual(rejected_signing.call_count, 1)
            rejected_signing.assert_called_once_with(
                [
                    "/usr/bin/codesign",
                    "--verify",
                    "--deep",
                    "--strict",
                    str(app.resolve()),
                ],
                check=True,
            )
            self.assertEqual(
                (
                    bundled_helper.read_bytes(),
                    bundled_helper.stat().st_mode,
                    bundled_helper.stat().st_mtime_ns,
                ),
                helper_before,
            )
            self.assertEqual(
                (
                    identity_path.read_bytes(),
                    identity_path.stat().st_mode,
                    identity_path.stat().st_mtime_ns,
                ),
                identity_before,
            )

            with mock.patch(
                "scripts.finalize_signed_helper_bundle.source_generation",
                return_value="a" * 64,
            ), mock.patch(
                "scripts.finalize_signed_helper_bundle.validate_tauri_resource_configuration"
            ), mock.patch(
                "scripts.finalize_signed_helper_bundle.validate_stage1_bundle_resources",
                return_value={"resource": "sealed"},
            ), mock.patch("subprocess.run"):
                finalize(app, "identity", entitlements)
            self.assertEqual(bundled_helper.read_bytes(), bytes(trusted))

            staged.write_bytes(bytes(trusted) + b"changed")
            with mock.patch(
                "scripts.finalize_signed_helper_bundle.source_generation",
                return_value="a" * 64,
            ), mock.patch(
                "scripts.finalize_signed_helper_bundle.validate_tauri_resource_configuration"
            ), mock.patch(
                "scripts.finalize_signed_helper_bundle.validate_stage1_bundle_resources",
                return_value={"resource": "sealed"},
            ), mock.patch("subprocess.run"):
                with self.assertRaises(ValueError):
                    finalize(app, "identity", entitlements)


if __name__ == "__main__":
    unittest.main()
