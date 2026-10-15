from __future__ import annotations

import hashlib
import json
import unittest

from scripts import generate_voice_owner_binding as binding


class GenerateVoiceOwnerBindingTests(unittest.TestCase):
    def test_bootstrap_catalog_replaces_owner_key_with_application_tag_hash(self) -> None:
        source = binding.ROOT / "src-tauri/resources/voice-asset-bindings.json"
        expected = json.loads(source.read_bytes())
        expected["owner_key"] = {
            "state": "bootstrap",
            "application_tag_sha256": hashlib.sha256(
                binding.APPLICATION_TAG.encode()
            ).hexdigest(),
        }

        self.assertEqual(json.loads(binding.bootstrap_catalog()), expected)


if __name__ == "__main__":
    unittest.main()
