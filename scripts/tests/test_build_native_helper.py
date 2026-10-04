import json
import os
import tempfile
import unittest
from pathlib import Path

from scripts.build_native_helper import (
    SOURCE_ALLOWLIST,
    IdentityDocument,
    atomic_publish_bytes,
    source_generation,
    validate_macho,
    validate_target,
)


class BuildNativeHelperTests(unittest.TestCase):
    def test_source_generation_closes_over_native_authorization_production_inputs(self) -> None:
        required = {
            "package.json",
            "scripts/build_native_helper.py",
            "scripts/finalize_signed_helper_bundle.py",
            "scripts/freeze_stage1_owner_authorization.py",
            "scripts/pin_stage1_owner_public_key.py",
            "scripts/prebuilt_runtime_stage1.py",
            "scripts/prepare_stage1_owner_authorization_bundle.py",
            "scripts/stage1_owner_authorization.py",
            "scripts/stage1_owner_roster.py",
            "scripts/tests/test_prepare_stage1_owner_authorization_bundle.py",
            "scripts/verify_signed_helper_bundle.py",
            "scripts/verify_stage1_owner_authorization_bundle.py",
            "src-tauri/Cargo.lock",
            "src-tauri/Cargo.toml",
            "src-tauri/Entitlements.plist",
            "src-tauri/HelperEntitlements.plist",
            "src-tauri/build.rs",
            "src-tauri/native/code_identity.c",
            "src-tauri/native/owner_authorization.c",
            "src-tauri/resources/stage1-owner-binding.json",
            "src-tauri/resources/stage1-owner-public-key.json",
            "src-tauri/src/bin/jarvis-runtime-helper.rs",
            "src-tauri/src/helper/evidence.rs",
            "src-tauri/src/helper/harness.rs",
            "src-tauri/src/helper/manifest.rs",
            "src-tauri/src/helper/mod.rs",
            "src-tauri/src/helper/process.rs",
            "src-tauri/src/helper/protocol.rs",
            "src-tauri/src/helper/sandbox.rs",
            "src-tauri/src/lib.rs",
            "src-tauri/src/main.rs",
            "src-tauri/src/owner_authorization/binding.rs",
            "src-tauri/src/owner_authorization/canonical.rs",
            "src-tauri/src/owner_authorization/cli.rs",
            "src-tauri/src/owner_authorization/deletion.rs",
            "src-tauri/src/owner_authorization/issuer.rs",
            "src-tauri/src/owner_authorization/mod.rs",
            "src-tauri/src/owner_authorization/paths.rs",
            "src-tauri/src/owner_authorization/process.rs",
            "src-tauri/src/platform/code_identity.rs",
            "src-tauri/src/platform/mod.rs",
            "src-tauri/src/platform/owner_authorization.rs",
            "src-tauri/src/signed_helper/helper.rs",
            "src-tauri/src/signed_helper/identity.rs",
            "src-tauri/src/signed_helper/mod.rs",
            "src-tauri/src/signed_helper/supervisor.rs",
            "src-tauri/tauri.conf.json",
        }
        self.assertTrue(required.issubset(SOURCE_ALLOWLIST))
        self.assertFalse(
            any(
                path.startswith(("src-tauri/generated/", "src-tauri/target/", "src-tauri/binaries/"))
                or ".app/" in path
                for path in SOURCE_ALLOWLIST
            )
        )

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for relative in SOURCE_ALLOWLIST:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes((relative + "\n").encode())
            baseline = source_generation(root, SOURCE_ALLOWLIST)
            for relative in sorted(required):
                path = root / relative
                original = path.read_bytes()
                path.write_bytes(original + b"changed\n")
                self.assertNotEqual(
                    source_generation(root, SOURCE_ALLOWLIST), baseline, relative
                )
                path.write_bytes(original)

    def test_owner_key_bridge_is_secure_enclave_only_and_proves_before_delete(self) -> None:
        repository = Path(__file__).resolve().parents[2]
        source = (repository / "src-tauri/native/owner_authorization.c").read_text()
        self.assertIn("kSecAttrTokenIDSecureEnclave", source)
        self.assertIn("kSecAccessControlPrivateKeyUsage", source)
        self.assertIn("kSecAccessControlUserPresence", source)
        self.assertNotIn("kSecAttrTokenIDAppleKeyStore", source)
        self.assertNotIn("openssl", source.casefold())
        self.assertEqual(source.count("SecKeyCreateRandomKey("), 1)
        self.assertEqual(source.count("SecItemCopyMatching("), 1)

        query_start = source.index("static CFDictionaryRef create_exact_key_query")
        query_end = source.index("static ", query_start + 1)
        query_source = source[query_start:query_end]
        self.assertIn("kSecAttrTokenIDSecureEnclave", query_source)

        delete_start = source.index("int32_t jarvis_owner_key_delete_after_proof")
        delete_source = source[delete_start:]
        self.assertEqual(delete_source.count("SecKeyCreateSignature("), 1)
        self.assertEqual(delete_source.count("SecItemDelete("), 1)
        self.assertLess(
            delete_source.index("SecKeyCreateSignature("),
            delete_source.index("SecKeyVerifySignature("),
        )
        self.assertLess(
            delete_source.index("SecKeyVerifySignature("),
            delete_source.index("SecItemDelete("),
        )
        self.assertIn(
            "if (result == JARVIS_OWNER_SUCCESS) {\n"
            "        CFDictionaryRef delete_query",
            delete_source,
        )

    def test_target_is_exactly_supported(self) -> None:
        self.assertEqual(validate_target("aarch64-apple-darwin"), "aarch64-apple-darwin")
        for target in ("", "x86_64-apple-darwin", "../aarch64-apple-darwin"):
            with self.assertRaises(ValueError):
                validate_target(target)

    def test_source_generation_is_ordered_and_content_bound(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "a").write_bytes(b"one")
            (root / "b").write_bytes(b"two")
            first = source_generation(root, ("a", "b"))
            self.assertEqual(first, source_generation(root, ("b", "a")))
            (root / "a").write_bytes(b"changed")
            self.assertNotEqual(first, source_generation(root, ("a", "b")))

    def test_source_generation_rejects_symlinks_and_missing_files(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "real").write_bytes(b"data")
            os.symlink(root / "real", root / "alias")
            with self.assertRaises(ValueError):
                source_generation(root, ("alias",))
            with self.assertRaises(ValueError):
                source_generation(root, ("missing",))

    def test_macho_validation_requires_arm64_64_bit_binary(self) -> None:
        header = bytearray(32)
        header[0:4] = bytes.fromhex("cffaedfe")
        header[4:8] = (0x0100000C).to_bytes(4, "little")
        validate_macho(bytes(header), "aarch64-apple-darwin")
        header[4:8] = (0x01000007).to_bytes(4, "little")
        with self.assertRaises(ValueError):
            validate_macho(bytes(header), "aarch64-apple-darwin")

    def test_atomic_publish_rejects_symlink_destination(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            victim = root / "victim"
            victim.write_bytes(b"unchanged")
            destination = root / "output"
            os.symlink(victim, destination)
            with self.assertRaises(ValueError):
                atomic_publish_bytes(destination, b"new", 0o500)
            self.assertEqual(victim.read_bytes(), b"unchanged")

    def test_identity_document_is_strict_and_round_trips(self) -> None:
        document = IdentityDocument(
            target_triple="aarch64-apple-darwin",
            source_generation_sha256="a" * 64,
            binary_sha256="b" * 64,
            binary_size=10,
        )
        encoded = document.to_json_bytes()
        self.assertEqual(IdentityDocument.from_json_bytes(encoded), document)
        raw = json.loads(encoded)
        raw["extra"] = True
        with self.assertRaises(ValueError):
            IdentityDocument.from_json_bytes(json.dumps(raw).encode())


if __name__ == "__main__":
    unittest.main()
