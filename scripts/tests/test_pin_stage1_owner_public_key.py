from __future__ import annotations

import base64
import hashlib
import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

from scripts import pin_stage1_owner_public_key as pin
from scripts import stage1_owner_authorization as auth


class PinStage1OwnerPublicKeyTests(unittest.TestCase):
    KEY = b"\x04" + b"\x11" * 64
    UNPROVISIONED = auth.canonical_json_bytes(
        {"schema": auth.PUBLIC_KEY_SCHEMA, "state": "unprovisioned"}
    )

    def bootstrap(self, **changes: object) -> bytes:
        record: dict[str, object] = {
            "algorithm": pin.OWNER_SIGNATURE_ALGORITHM,
            "application_tag": auth.OWNER_KEY_TAG,
            "key_id": hashlib.sha256(self.KEY).hexdigest(),
            "public_key_x963_base64": base64.b64encode(self.KEY).decode("ascii"),
            "schema": auth.PUBLIC_KEY_SCHEMA,
        }
        record.update(changes)
        return auth.canonical_json_bytes(record)

    def pinned(self, key: bytes | None = None) -> bytes:
        key = self.KEY if key is None else key
        return auth.canonical_json_bytes(
            {
                "algorithm": pin.OWNER_SIGNATURE_ALGORITHM,
                "key_id": hashlib.sha256(key).hexdigest(),
                "public_key_x963_base64": base64.b64encode(key).decode("ascii"),
                "schema": auth.PUBLIC_KEY_SCHEMA,
                "state": "pinned",
            }
        )

    def make_tree(self, root: Path, bootstrap: bytes | None = None) -> tuple[Path, Path, Path]:
        home = root / "home"
        bootstrap_directory = (
            home
            / "Library"
            / "Application Support"
            / "dev.rohtolos.saturday"
            / "Stage1OwnerAuthorization"
            / "bootstrap"
        )
        bootstrap_directory.mkdir(parents=True, mode=0o700)
        for directory in [
            home,
            home / "Library",
            home / "Library" / "Application Support",
            home / "Library" / "Application Support" / "dev.rohtolos.saturday",
            home
            / "Library"
            / "Application Support"
            / "dev.rohtolos.saturday"
            / "Stage1OwnerAuthorization",
            bootstrap_directory,
        ]:
            directory.chmod(0o700)
        bootstrap_path = bootstrap_directory / "owner-public-key.json"
        bootstrap_path.write_bytes(self.bootstrap() if bootstrap is None else bootstrap)
        bootstrap_path.chmod(0o600)
        resource = root / "stage1-owner-public-key.json"
        resource.write_bytes(self.UNPROVISIONED)
        resource.chmod(0o644)
        return home, bootstrap_path, resource

    def test_pins_from_exact_account_bootstrap_and_is_idempotent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            home, _, resource = self.make_tree(Path(directory))
            with mock.patch.object(pin, "PUBLIC_KEY_RESOURCE", resource), mock.patch.object(
                pin, "_effective_account_home", return_value=home
            ):
                metadata = pin.pin_from_canonical_bootstrap()
            key_id = hashlib.sha256(self.KEY).hexdigest()
            self.assertEqual(metadata, {"key_id": key_id, "pinned": True})
            self.assertEqual(resource.read_bytes(), self.pinned())
            self.assertEqual(resource.stat().st_mode & 0o777, 0o644)
            self.assertEqual(list(resource.parent.glob(".stage1-owner-public-key.json.tmp*")), [])

            with mock.patch.object(pin, "PUBLIC_KEY_RESOURCE", resource), mock.patch.object(
                pin, "_effective_account_home", return_value=home
            ), mock.patch.object(pin, "_atomic_replace") as atomic_replace:
                self.assertEqual(pin.pin_from_canonical_bootstrap(), metadata)
                atomic_replace.assert_not_called()

    def test_main_accepts_no_paths_and_prints_only_canonical_public_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            home, _, resource = self.make_tree(Path(directory))
            stdout = io.StringIO()
            with mock.patch.object(pin, "PUBLIC_KEY_RESOURCE", resource), mock.patch.object(
                pin, "_effective_account_home", return_value=home
            ), redirect_stdout(stdout):
                self.assertEqual(pin.main([]), 0)
            expected = auth.canonical_json_bytes(
                {"key_id": hashlib.sha256(self.KEY).hexdigest(), "pinned": True}
            ).decode("ascii")
            self.assertEqual(stdout.getvalue(), expected + "\n")
            self.assertNotIn("private", stdout.getvalue())
            self.assertNotIn("signature", stdout.getvalue())

            stderr = io.StringIO()
            with redirect_stderr(stderr):
                self.assertEqual(pin.main(["unexpected"]), 64)
            self.assertEqual(stderr.getvalue(), "SATURDAY_STAGE1_OWNER_PUBLIC_KEY_PIN_ERROR:arguments\n")

    def test_rejects_symlink_hardlink_wrong_mode_wrong_owner_and_invalid_records(self) -> None:
        invalid_records = [
            self.bootstrap(application_tag="wrong"),
            self.bootstrap(algorithm="wrong"),
            self.bootstrap(key_id="00" * 32),
            self.bootstrap(
                public_key_x963_base64=base64.b64encode(b"\x03" + b"\x11" * 64).decode(
                    "ascii"
                )
            ),
            self.bootstrap(
                public_key_x963_base64=base64.b64encode(b"\x04" + b"\x11" * 63).decode(
                    "ascii"
                )
            ),
            self.bootstrap(extra="forbidden"),
            self.bootstrap(private_key="forbidden"),
            self.bootstrap(signature="forbidden"),
        ]
        for invalid in invalid_records:
            with self.subTest(invalid=invalid), tempfile.TemporaryDirectory() as directory:
                home, _, resource = self.make_tree(Path(directory), invalid)
                before = resource.read_bytes()
                with mock.patch.object(pin, "PUBLIC_KEY_RESOURCE", resource), mock.patch.object(
                    pin, "_effective_account_home", return_value=home
                ), self.assertRaises(pin.PinError):
                    pin.pin_from_canonical_bootstrap()
                self.assertEqual(resource.read_bytes(), before)

        for case in ("symlink", "hardlink", "mode", "owner"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                home, bootstrap_path, resource = self.make_tree(root)
                if case == "symlink":
                    target = root / "outside.json"
                    target.write_bytes(self.bootstrap())
                    bootstrap_path.unlink()
                    bootstrap_path.symlink_to(target)
                elif case == "hardlink":
                    os.link(bootstrap_path, root / "second-link.json")
                elif case == "mode":
                    bootstrap_path.chmod(0o644)
                before = resource.read_bytes()
                owner_context = (
                    mock.patch.object(os, "geteuid", return_value=os.geteuid() + 1)
                    if case == "owner"
                    else mock.patch.object(os, "geteuid", wraps=os.geteuid)
                )
                with mock.patch.object(pin, "PUBLIC_KEY_RESOURCE", resource), mock.patch.object(
                    pin, "_effective_account_home", return_value=home
                ), owner_context, self.assertRaises(pin.PinError):
                    pin.pin_from_canonical_bootstrap()
                self.assertEqual(resource.read_bytes(), before)

    def test_refuses_mismatched_pinned_resource_and_exposes_package_command(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            home, _, resource = self.make_tree(Path(directory))
            mismatch = self.pinned(b"\x04" + b"\x22" * 64)
            resource.write_bytes(mismatch)
            with mock.patch.object(pin, "PUBLIC_KEY_RESOURCE", resource), mock.patch.object(
                pin, "_effective_account_home", return_value=home
            ), self.assertRaises(pin.PinError):
                pin.pin_from_canonical_bootstrap()
            self.assertEqual(resource.read_bytes(), mismatch)

        repository = Path(__file__).resolve().parents[2]
        package = json.loads((repository / "package.json").read_text(encoding="utf-8"))
        self.assertEqual(
            package["scripts"]["pin:stage1-owner-public-key"],
            "python3 scripts/pin_stage1_owner_public_key.py",
        )
        source = Path(pin.__file__).read_text(encoding="utf-8")
        self.assertNotIn("subprocess", source)
        self.assertNotIn("os.system", source)
        self.assertNotIn("shell=True", source)


if __name__ == "__main__":
    unittest.main()
