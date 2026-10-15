from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts import build_voice_workers


class WorkerOwnerKeyPinTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.directory = Path(self.temporary.name) / "voice-owner-build"
        self.directory.mkdir(mode=0o700)
        self.pin_path = self.directory / "voice-owner-key-pin.json"
        self.patch = patch.object(build_voice_workers, "OWNER_KEY_PIN", self.pin_path)
        self.patch.start()
        self.key = bytes.fromhex(
            "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296"
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        )

    def tearDown(self) -> None:
        self.patch.stop()
        self.temporary.cleanup()

    def write_pin(self, values: dict[str, object] | None = None) -> None:
        key = self.key
        record = json.dumps({"payload": {}, "signature_der": "MAg="}, sort_keys=True, separators=(",", ":")).encode()
        witness = json.dumps({"witness": "fixture"}, sort_keys=True, separators=(",", ":")).encode()
        pin = {
            "schema": "saturday.voice.owner-key-pin.v1",
            "application_tag_sha256": build_voice_workers.sha256(build_voice_workers.OWNER_APPLICATION_TAG),
            "owner_public_key_x963_hex": key.hex(),
            "owner_public_key_sha256": build_voice_workers.sha256(key),
            "bootstrap_record_sha256": build_voice_workers.sha256(record),
            "bootstrap_record": build_voice_workers.base64.urlsafe_b64encode(record).rstrip(b"=").decode(),
            "bootstrap_witness_sha256": build_voice_workers.sha256(witness),
            "bootstrap_witness": build_voice_workers.base64.urlsafe_b64encode(witness).rstrip(b"=").decode(),
            **(values or {}),
        }
        self.pin_path.write_bytes(json.dumps(pin, sort_keys=True, separators=(",", ":")).encode() + b"\n")
        self.pin_path.chmod(0o600)

    def test_accepts_canonical_fixed_purpose_pin(self) -> None:
        self.write_pin()
        self.assertEqual(
            build_voice_workers.trusted_owner_public_key(),
            (self.key, build_voice_workers.sha256(self.pin_path.read_bytes())),
        )

    def test_rejects_wrong_purpose_noncanonical_point_digest_and_record(self) -> None:
        cases = (
            {"application_tag_sha256": "0" * 64},
            {"owner_public_key_x963_hex": "05" + self.key.hex()[2:]},
            {"owner_public_key_sha256": "0" * 64},
            {"bootstrap_record_sha256": "0" * 64},
            {"bootstrap_witness_sha256": "0" * 64},
            {"unexpected": "authority"},
        )
        for values in cases:
            with self.subTest(values=values):
                self.write_pin(values)
                with self.assertRaises(ValueError):
                    build_voice_workers.trusted_owner_public_key()

    def test_rejects_duplicate_pin_fields(self) -> None:
        self.pin_path.write_text(
            '{"schema":"saturday.voice.owner-key-pin.v1",'
            '"schema":"saturday.voice.owner-key-pin.v1"}',
            encoding="utf-8",
        )
        self.pin_path.chmod(0o600)
        with self.assertRaises(ValueError):
            build_voice_workers.trusted_owner_public_key()

    def test_rejects_nonprivate_pin_and_symlink(self) -> None:
        self.write_pin()
        self.pin_path.chmod(0o644)
        with self.assertRaises(ValueError):
            build_voice_workers.trusted_owner_public_key()
        self.pin_path.unlink()
        target = Path(self.temporary.name) / "pin.json"
        target.write_text("{}", encoding="utf-8")
        self.pin_path.symlink_to(target)
        with self.assertRaises(ValueError):
            build_voice_workers.trusted_owner_public_key()


if __name__ == "__main__":
    unittest.main()
