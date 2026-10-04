from __future__ import annotations

import base64
import copy
import ctypes
import fcntl
import hashlib
import inspect
import json
import os
import stat
import subprocess
import tempfile
import unittest
from dataclasses import FrozenInstanceError
from pathlib import Path
from unittest import mock

from scripts import stage1_owner_authorization as auth


EXPECTED_PAYLOAD = b'{"binding_sha256":"2222222222222222222222222222222222222222222222222222222222222222","expires_at_unix":1795000300,"generation":"00112233445566778899aabbccddeeff","issued_at_unix":1795000000,"model_transfer":"blocked","nonce_sha256":"1111111111111111111111111111111111111111111111111111111111111111","purpose":"stage1_bottle_transfer","schema":"saturday.local-provider.stage1-owner-authorization-payload.v1","state":"granted"}'


class CanonicalOwnerAuthorizationTests(unittest.TestCase):
    def test_fixture_is_exact_ascii_sorted_json(self) -> None:
        payload = {
            "schema": auth.PAYLOAD_SCHEMA,
            "state": "granted",
            "purpose": "stage1_bottle_transfer",
            "generation": "00112233445566778899aabbccddeeff",
            "issued_at_unix": 1795000000,
            "expires_at_unix": 1795000300,
            "nonce_sha256": "11" * 32,
            "binding_sha256": "22" * 32,
            "model_transfer": "blocked",
        }
        self.assertEqual(auth.canonical_json_bytes(payload), EXPECTED_PAYLOAD)

    def test_duplicate_nonfinite_nonascii_and_noncanonical_base64_fail(self) -> None:
        for payload in (
            b'{"a":1,"a":2}',
            b'{"a":NaN}',
            b'{"a":Infinity}',
            b'{"a":-Infinity}',
        ):
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                auth.decode_json_object(payload, 1024)
        with self.assertRaises(ValueError):
            auth.canonical_json_bytes({"a": "é"})
        with self.assertRaises(ValueError):
            auth.decode_canonical_base64("YQ", 8)

    def test_json_is_bounded_object_only_and_exactly_canonical(self) -> None:
        for payload in (
            b'',
            b'[]',
            b'{"a":null}',
            b'{"a":1.5}',
            b'{"a":"line\\nfeed"}',
            b'{"b":1,"a":2}',
            b'{"a":1}\n',
        ):
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                auth.decode_json_object(payload, 1024)
        with self.assertRaises(ValueError):
            auth.decode_json_object(b'{"a":1}', 6)
        self.assertEqual(auth.decode_json_object(b'{"a":1}', 7), {"a": 1})

    def test_schema_integer_callers_reject_booleans(self) -> None:
        document = auth.decode_json_object(b'{"issued_at_unix":true}', 1024)

        def require_i64(value: object) -> int:
            if type(value) is not int:
                raise ValueError("field must be a signed integer")
            return value

        with self.assertRaises(ValueError):
            require_i64(document["issued_at_unix"])

    def test_hex_and_base64_enforce_exact_alphabet_length_and_size(self) -> None:
        self.assertEqual(auth.decode_lower_hex("0011aaff", 4), b"\x00\x11\xaa\xff")
        for value in ("0011AAff", "0011aaf", "0011aaff00", "0011aagf"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                auth.decode_lower_hex(value, 4)

        self.assertEqual(auth.decode_canonical_base64("YQ==", 1), b"a")
        for value in ("YQ", "YQ=", "YR==", "YQ===", "YQ==\n"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                auth.decode_canonical_base64(value, 8)
        with self.assertRaises(ValueError):
            auth.decode_canonical_base64("YWI=", 1)

    def test_checked_in_literal_matches_the_rust_fixture(self) -> None:
        repository = Path(__file__).resolve().parents[2]
        result = subprocess.run(
            [
                "cargo",
                "test",
                "--offline",
                "--manifest-path",
                "src-tauri/Cargo.toml",
                "owner_authorization::canonical::tests::fixture_is_exact_ascii_sorted_json",
                "--",
                "--exact",
            ],
            cwd=repository,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_constants_match_the_native_contract(self) -> None:
        self.assertEqual(
            auth.OWNER_KEY_TAG,
            "dev.rohtolos.saturday.stage1-owner-authorization.v1",
        )
        self.assertEqual(
            auth.PUBLIC_KEY_SCHEMA,
            "saturday.local-provider.stage1-owner-public-key.v1",
        )
        self.assertEqual(
            auth.BINDING_SCHEMA,
            "saturday.local-provider.stage1-owner-binding.v1",
        )
        self.assertEqual(
            auth.ENVELOPE_SCHEMA,
            "saturday.local-provider.stage1-owner-authorization-envelope.v1",
        )
        self.assertEqual(
            auth.PAYLOAD_SCHEMA,
            "saturday.local-provider.stage1-owner-authorization-payload.v1",
        )
        self.assertEqual(
            auth.SIGNING_DOMAIN,
            b"SATURDAY-STAGE1-OWNER-AUTHORIZATION-V1\0",
        )
        self.assertEqual(auth.AUTHORIZATION_TTL_SECONDS, 300)


class FrozenBindingReplayTests(unittest.TestCase):
    FIXTURE = Path(__file__).with_name("fixtures") / "stage1-owner-binding-frozen.json"

    def test_python_replays_the_shared_exact_frozen_binding(self) -> None:
        payload = self.FIXTURE.read_bytes()
        binding = auth.parse_frozen_binding(payload)
        self.assertEqual(binding.binding_sha256, hashlib.sha256(payload).hexdigest())
        self.assertEqual(
            binding.candidate,
            {
                "candidate_id": "qwen35-4b-q4km-homebrew-llamacpp-0.4.1",
                "candidate_sha256": "b2672f1a00121c09077bc447da7fff720238012daf2ecd645add58c48124a5bb",
                "review_aggregate_sha256": "9178a4708e4e5a834c936fe8f8ff7a76ee3c45e8aee7fcb90bfe9101989e5a8c",
            },
        )
        self.assertEqual([item["formula"] for item in binding.artifacts], [
            "llama.cpp", "ggml", "libomp", "openssl@3", "ca-certificates"
        ])
        self.assertEqual(binding.limits["artifact_count"], 5)
        self.assertEqual(binding.model_transfer, "blocked")
        self.assertEqual(binding.resource_roster[0]["path"], "fixture/resource")
        with self.assertRaises(FrozenInstanceError):
            binding.model_transfer = "allowed"

    def test_shared_full_payload_and_envelope_fixtures_are_canonical_and_linked(self) -> None:
        payload_path = self.FIXTURE.with_name("stage1-owner-payload.json")
        envelope_path = self.FIXTURE.with_name("stage1-owner-envelope.json")
        payload_bytes = payload_path.read_bytes()
        envelope_bytes = envelope_path.read_bytes()
        payload = json.loads(payload_bytes)
        envelope = json.loads(envelope_bytes)
        self.assertEqual(auth.canonical_json_bytes(payload), payload_bytes)
        self.assertEqual(auth.canonical_json_bytes(envelope), envelope_bytes)
        self.assertEqual(base64.b64decode(envelope["payload_base64"]), payload_bytes)
        self.assertEqual(len(payload["artifacts"]), 5)
        self.assertEqual(payload["model_transfer"], "blocked")

    def test_frozen_binding_rejects_noncanonical_unknown_revoked_and_mutated_values(self) -> None:
        original = json.loads(self.FIXTURE.read_bytes())
        mutations = []

        def changed(callback):
            value = copy.deepcopy(original)
            callback(value)
            mutations.append(value)

        changed(lambda value: value.update(extra=True))
        changed(lambda value: value.update(state="unfrozen"))
        changed(lambda value: value.update(model_transfer="allowed"))
        changed(lambda value: value["candidate"].update(candidate_sha256="00" * 32))
        changed(lambda value: value["review"].update(review_aggregate_sha256="f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0"))
        changed(lambda value: value["artifacts"].reverse())
        changed(lambda value: value["artifacts"][0].update(transfer_bytes=1))
        changed(lambda value: value["limits"].update(maximum_archive_entries=1))
        changed(lambda value: value["resource_roster"][0].update(path="../escape"))
        changed(lambda value: value["resource_roster"].append(copy.deepcopy(value["resource_roster"][0])))
        for value in mutations:
            with self.subTest(value=value), self.assertRaises(ValueError):
                auth.parse_frozen_binding(auth.canonical_json_bytes(value))
        with self.assertRaises(ValueError):
            auth.parse_frozen_binding(self.FIXTURE.read_bytes() + b"\n")


class FakeVerifier:
    def __init__(self, result: bool = True) -> None:
        self.result = result
        self.calls: list[tuple[bytes, bytes, bytes]] = []

    def verify(
        self, public_key_x963: bytes, message: bytes, signature_der: bytes
    ) -> bool:
        self.calls.append((public_key_x963, message, signature_der))
        return self.result


class SignatureEnvelopeTests(unittest.TestCase):
    PUBLIC_KEY = b"\x04" + b"\x11" * 64
    SIGNATURE = b"\x30\x00"
    PAYLOAD = auth.canonical_json_bytes(
        {"schema": auth.PAYLOAD_SCHEMA, "state": "granted"}
    )

    def envelope(self, **changes: object) -> bytes:
        value: dict[str, object] = {
            "algorithm": auth.OWNER_SIGNATURE_ALGORITHM,
            "key_id": hashlib.sha256(self.PUBLIC_KEY).hexdigest(),
            "payload_base64": base64.b64encode(self.PAYLOAD).decode("ascii"),
            "schema": auth.ENVELOPE_SCHEMA,
            "signature_base64": base64.b64encode(self.SIGNATURE).decode("ascii"),
        }
        value.update(changes)
        return auth.canonical_json_bytes(value)

    def test_rejects_malformed_before_verifier_and_returns_frozen_result_after_one_call(self) -> None:
        verifier = FakeVerifier()
        with self.assertRaises(ValueError):
            auth.parse_and_verify_envelope(b"{}", self.PUBLIC_KEY, verifier)
        self.assertEqual(verifier.calls, [])

        envelope = self.envelope()
        parsed = auth.parse_and_verify_envelope(envelope, self.PUBLIC_KEY, verifier)
        self.assertEqual(
            verifier.calls,
            [(self.PUBLIC_KEY, auth.SIGNING_DOMAIN + self.PAYLOAD, self.SIGNATURE)],
        )
        self.assertEqual(parsed.key_id, hashlib.sha256(self.PUBLIC_KEY).hexdigest())
        self.assertEqual(parsed.payload_bytes, self.PAYLOAD)
        self.assertEqual(parsed.payload, {"schema": auth.PAYLOAD_SCHEMA, "state": "granted"})
        self.assertEqual(parsed.signature_der, self.SIGNATURE)
        self.assertEqual(parsed.envelope_sha256, hashlib.sha256(envelope).hexdigest())
        with self.assertRaises(FrozenInstanceError):
            parsed.key_id = "00" * 32

    def test_rejects_false_verification_and_mutated_fields(self) -> None:
        invalid = [
            self.envelope(
                payload_base64=base64.b64encode(
                    auth.canonical_json_bytes(
                        {"schema": auth.PAYLOAD_SCHEMA, "state": "denied"}
                    )
                ).decode("ascii")
            ),
            self.envelope(signature_base64=base64.b64encode(b"\x30\x01").decode("ascii")),
            self.envelope(key_id="00" * 32),
            self.envelope(algorithm="wrong"),
            self.envelope(schema="wrong"),
            self.envelope(payload_base64="YQ"),
        ]
        for envelope in invalid:
            verifier = FakeVerifier(result=False)
            with self.subTest(envelope=envelope), self.assertRaises(
                (ValueError, auth.SignatureVerificationError)
            ):
                auth.parse_and_verify_envelope(envelope, self.PUBLIC_KEY, verifier)

    def test_recomputes_key_id_and_validates_public_key_before_verification(self) -> None:
        for public_key in [b"\x04" + b"\x11" * 63, b"\x03" + b"\x11" * 64]:
            verifier = FakeVerifier()
            with self.subTest(public_key=public_key), self.assertRaises(ValueError):
                auth.parse_and_verify_envelope(self.envelope(), public_key, verifier)
            self.assertEqual(verifier.calls, [])

    def test_invalid_envelopes_reach_no_filesystem_lock_or_transport_effect(self) -> None:
        invalid = [
            b"{}",
            self.envelope(key_id="00" * 32),
            self.envelope(algorithm="wrong"),
            self.envelope(schema="wrong"),
            self.envelope(payload_base64="YQ"),
            self.envelope(signature_base64=base64.b64encode(b"\x30\x01").decode("ascii")),
        ]
        for envelope in invalid:
            verifier = FakeVerifier(result=False)
            with self.subTest(envelope=envelope), mock.patch.object(
                os, "open", side_effect=AssertionError("filesystem effect")
            ), mock.patch.object(
                os, "mkdir", side_effect=AssertionError("filesystem effect")
            ), mock.patch.object(
                fcntl, "flock", side_effect=AssertionError("lock effect")
            ), mock.patch(
                "scripts.prebuilt_runtime_stage1.OCITransport",
                side_effect=AssertionError("transport effect"),
            ), self.assertRaises((ValueError, auth.SignatureVerificationError)):
                auth.parse_and_verify_envelope(envelope, self.PUBLIC_KEY, verifier)


class OwnerAuthorizationSemanticTests(unittest.TestCase):
    PUBLIC_KEY = b"\x04" + b"\x11" * 64
    SIGNATURE = b"\x30\x00"
    NOW = 1_795_000_000
    GENERATION = "00112233445566778899aabbccddeeff"
    NONCE = b"n" * 32
    BINDING_BYTES = (
        Path(__file__).with_name("fixtures") / "stage1-owner-binding-frozen.json"
    ).read_bytes()
    PINNED_ANCHOR = auth.canonical_json_bytes(
        {
            "algorithm": auth.OWNER_SIGNATURE_ALGORITHM,
            "key_id": hashlib.sha256(PUBLIC_KEY).hexdigest(),
            "public_key_x963_base64": base64.b64encode(PUBLIC_KEY).decode("ascii"),
            "schema": auth.PUBLIC_KEY_SCHEMA,
            "state": "pinned",
        }
    )

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.descriptors: dict[str, int] = {}
        root = Path(self.temporary.name)
        for name in ("ledger", "workspace_parent", "receipt_parent"):
            path = root / name
            path.mkdir(mode=0o700)
            self.descriptors[name] = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
        self.directory_stats = {
            name: os.fstat(descriptor) for name, descriptor in self.descriptors.items()
        }

    def tearDown(self) -> None:
        for descriptor in self.descriptors.values():
            os.close(descriptor)
        self.temporary.cleanup()

    @staticmethod
    def identity(info: os.stat_result) -> dict[str, int]:
        return {"device": info.st_dev, "inode": info.st_ino, "uid": info.st_uid}

    def payload(self) -> dict[str, object]:
        binding = auth.parse_frozen_binding(self.BINDING_BYTES)
        return {
            "artifacts": copy.deepcopy(binding.artifacts),
            "binding_sha256": binding.binding_sha256,
            "candidate": copy.deepcopy(binding.candidate),
            "expires_at_unix": self.NOW + auth.AUTHORIZATION_TTL_SECONDS,
            "generation": self.GENERATION,
            "issued_at_unix": self.NOW,
            "ledger": self.identity(self.directory_stats["ledger"]),
            "limits": copy.deepcopy(binding.limits),
            "model_transfer": "blocked",
            "nonce_sha256": hashlib.sha256(self.NONCE).hexdigest(),
            "purpose": "stage1_bottle_transfer",
            "receipt": {
                **self.identity(self.directory_stats["receipt_parent"]),
                "name": f"stage1-{self.GENERATION}.json",
            },
            "review": {
                "review_aggregate_sha256": binding.review[
                    "review_aggregate_sha256"
                ],
                "review_manifest_sha256": binding.review[
                    "review_manifest_sha256"
                ],
            },
            "schema": auth.PAYLOAD_SCHEMA,
            "state": "granted",
            "workspace_parent": self.identity(
                self.directory_stats["workspace_parent"]
            ),
        }

    def envelope(self, payload: dict[str, object]) -> bytes:
        payload_bytes = auth.canonical_json_bytes(payload)
        return auth.canonical_json_bytes(
            {
                "algorithm": auth.OWNER_SIGNATURE_ALGORITHM,
                "key_id": hashlib.sha256(self.PUBLIC_KEY).hexdigest(),
                "payload_base64": base64.b64encode(payload_bytes).decode("ascii"),
                "schema": auth.ENVELOPE_SCHEMA,
                "signature_base64": base64.b64encode(self.SIGNATURE).decode("ascii"),
            }
        )

    def verify(
        self,
        payload: dict[str, object],
        *,
        nonce: bytes | None = None,
        now_unix: int | None = None,
        directory_stats: dict[str, os.stat_result] | None = None,
        verifier: FakeVerifier | None = None,
    ) -> auth.VerifiedOwnerAuthorization:
        return auth.verify_owner_authorization(
            self.envelope(payload),
            self.NONCE if nonce is None else nonce,
            self.PINNED_ANCHOR,
            self.BINDING_BYTES,
            self.directory_stats if directory_stats is None else directory_stats,
            self.NOW if now_unix is None else now_unix,
            FakeVerifier() if verifier is None else verifier,
        )

    def test_accepts_exact_signed_payload_and_returns_only_redacted_authority(self) -> None:
        verifier = FakeVerifier()
        verified = self.verify(self.payload(), verifier=verifier)
        self.assertEqual(verified.generation, self.GENERATION)
        self.assertEqual(verified.receipt_name, f"stage1-{self.GENERATION}.json")
        self.assertEqual(verified.nonce_sha256, hashlib.sha256(self.NONCE).hexdigest())
        self.assertEqual(verified.key_id, hashlib.sha256(self.PUBLIC_KEY).hexdigest())
        self.assertEqual(verified.binding_sha256, hashlib.sha256(self.BINDING_BYTES).hexdigest())
        self.assertNotIn("signature", verified.__dict__)
        self.assertNotIn("public_key", verified.__dict__)
        self.assertEqual(len(verifier.calls), 1)
        with self.assertRaises(FrozenInstanceError):
            verified.generation = "11" * 16

    def test_semantic_mutation_matrix_fails_before_any_effect(self) -> None:
        original = self.payload()
        cases: list[tuple[str, dict[str, object], bytes, int, dict[str, os.stat_result]]] = []

        def add(label, callback, *, nonce=None, now=None, stats=None):
            value = copy.deepcopy(original)
            callback(value)
            cases.append(
                (
                    label,
                    value,
                    self.NONCE if nonce is None else nonce,
                    self.NOW if now is None else now,
                    self.directory_stats if stats is None else stats,
                )
            )

        add("future issue", lambda value: value.update(issued_at_unix=self.NOW + 1))
        add("wrong ttl", lambda value: value.update(expires_at_unix=self.NOW + 299))
        add("expired", lambda _value: None, now=self.NOW + auth.AUTHORIZATION_TTL_SECONDS)
        add("generation length", lambda value: value.update(generation="00" * 15))
        add("generation case", lambda value: value.update(generation=self.GENERATION.upper()))
        add("nonce length", lambda _value: None, nonce=b"short")
        add("nonce hash", lambda value: value.update(nonce_sha256="00" * 32))
        add("binding", lambda value: value.update(binding_sha256="00" * 32))
        add("candidate id", lambda value: value["candidate"].update(candidate_id="other"))
        add("candidate sha", lambda value: value["candidate"].update(candidate_sha256="00" * 32))
        add("candidate review", lambda value: value["candidate"].update(review_aggregate_sha256="00" * 32))
        add("revoked review", lambda value: value["review"].update(review_aggregate_sha256="f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0"))
        add("manifest", lambda value: value["review"].update(review_manifest_sha256="00" * 32))
        add("artifact order", lambda value: value["artifacts"].reverse())
        add("artifact missing", lambda value: value["artifacts"].pop())
        add("artifact extra", lambda value: value["artifacts"].append(copy.deepcopy(value["artifacts"][0])))
        add("artifact changed", lambda value: value["artifacts"][0].update(transfer_bytes=1))
        add("installed limit", lambda value: value["artifacts"][0].update(installed_bytes=1))
        add("extraction limit", lambda value: value["artifacts"][4].update(maximum_extracted_bytes=1))
        for field in (
            "artifact_count",
            "verified_layer_total_bytes",
            "maximum_transfer_bytes",
            "minimum_free_bytes_before_each_body_read",
            "maximum_archive_entries",
            "maximum_total_extracted_bytes",
        ):
            add(field, lambda value, field=field: value["limits"].update({field: 1}))
        for role in ("ledger", "workspace_parent", "receipt"):
            for field in ("device", "inode", "uid"):
                add(f"{role} {field}", lambda value, role=role, field=field: value[role].update({field: value[role][field] + 1}))
        add("receipt name", lambda value: value["receipt"].update(name="attacker.json"))
        add("model", lambda value: value.update(model_transfer="allowed"))
        add("extra", lambda value: value.update(extra=True))

        for label, payload, nonce, now_unix, directory_stats in cases:
            verifier = FakeVerifier()
            with self.subTest(label=label), mock.patch.object(
                os, "open", side_effect=AssertionError("filesystem effect")
            ), mock.patch.object(
                os, "mkdir", side_effect=AssertionError("filesystem effect")
            ), mock.patch.object(
                fcntl, "flock", side_effect=AssertionError("lock effect")
            ), mock.patch(
                "scripts.prebuilt_runtime_stage1.OCITransport",
                side_effect=AssertionError("transport effect"),
            ), self.assertRaises((ValueError, PermissionError, auth.SignatureVerificationError)):
                auth.verify_owner_authorization(
                    self.envelope(payload),
                    nonce,
                    self.PINNED_ANCHOR,
                    self.BINDING_BYTES,
                    directory_stats,
                    now_unix,
                    verifier,
                )
            self.assertEqual(len(verifier.calls), 1)

    def test_false_signature_stops_before_semantics_or_effects(self) -> None:
        verifier = FakeVerifier(False)
        envelope = self.envelope(self.payload())
        with mock.patch.object(
            auth, "parse_frozen_binding", side_effect=AssertionError("binding parsed")
        ), mock.patch.object(
            os, "open", side_effect=AssertionError("filesystem effect")
        ), self.assertRaises(auth.SignatureVerificationError):
            auth.verify_owner_authorization(
                envelope,
                self.NONCE,
                self.PINNED_ANCHOR,
                self.BINDING_BYTES,
                self.directory_stats,
                self.NOW,
                verifier,
            )


class SoftwareP256Fixture:
    CORE_FOUNDATION = "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation"
    SECURITY = "/System/Library/Frameworks/Security.framework/Security"

    def __init__(self) -> None:
        self.cf = ctypes.CDLL(self.CORE_FOUNDATION)
        self.security = ctypes.CDLL(self.SECURITY)
        self._bind()
        self.private_key = self._create_private_key()

    def __enter__(self) -> "SoftwareP256Fixture":
        return self

    def __exit__(self, *_: object) -> None:
        self.cf.CFRelease(self.private_key)

    def public_key_x963(self) -> bytes:
        public_key = self.security.SecKeyCopyPublicKey(self.private_key)
        if not public_key:
            raise RuntimeError("software public key creation failed")
        error = ctypes.c_void_p()
        data = self.security.SecKeyCopyExternalRepresentation(
            public_key, ctypes.byref(error)
        )
        try:
            if not data:
                raise RuntimeError("software public key export failed")
            return self._data_bytes(data)
        finally:
            if data:
                self.cf.CFRelease(data)
            self.cf.CFRelease(public_key)
            self._release_error(error)

    def sign(self, message: bytes) -> bytes:
        data = self._create_data(message)
        error = ctypes.c_void_p()
        signature = self.security.SecKeyCreateSignature(
            self.private_key,
            self.algorithm,
            data,
            ctypes.byref(error),
        )
        try:
            if not signature:
                raise RuntimeError("software signature creation failed")
            return self._data_bytes(signature)
        finally:
            if signature:
                self.cf.CFRelease(signature)
            self.cf.CFRelease(data)
            self._release_error(error)

    def _bind(self) -> None:
        self.cf.CFDataCreate.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_uint8),
            ctypes.c_long,
        ]
        self.cf.CFDataCreate.restype = ctypes.c_void_p
        self.cf.CFDataGetLength.argtypes = [ctypes.c_void_p]
        self.cf.CFDataGetLength.restype = ctypes.c_long
        self.cf.CFDataGetBytePtr.argtypes = [ctypes.c_void_p]
        self.cf.CFDataGetBytePtr.restype = ctypes.POINTER(ctypes.c_uint8)
        self.cf.CFNumberCreate.argtypes = [
            ctypes.c_void_p,
            ctypes.c_long,
            ctypes.c_void_p,
        ]
        self.cf.CFNumberCreate.restype = ctypes.c_void_p
        self.cf.CFDictionaryCreate.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_void_p),
            ctypes.POINTER(ctypes.c_void_p),
            ctypes.c_long,
            ctypes.c_void_p,
            ctypes.c_void_p,
        ]
        self.cf.CFDictionaryCreate.restype = ctypes.c_void_p
        self.cf.CFRelease.argtypes = [ctypes.c_void_p]
        self.cf.CFRelease.restype = None

        self.security.SecKeyCreateRandomKey.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self.security.SecKeyCreateRandomKey.restype = ctypes.c_void_p
        self.security.SecKeyCopyPublicKey.argtypes = [ctypes.c_void_p]
        self.security.SecKeyCopyPublicKey.restype = ctypes.c_void_p
        self.security.SecKeyCopyExternalRepresentation.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self.security.SecKeyCopyExternalRepresentation.restype = ctypes.c_void_p
        self.security.SecKeyCreateSignature.argtypes = [
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self.security.SecKeyCreateSignature.restype = ctypes.c_void_p

        self.attr_key_type = self._constant("kSecAttrKeyType")
        self.attr_key_size = self._constant("kSecAttrKeySizeInBits")
        self.key_type_ec = self._constant("kSecAttrKeyTypeECSECPrimeRandom")
        self.algorithm = self._constant(
            "kSecKeyAlgorithmECDSASignatureMessageX962SHA256"
        )
        self.key_callbacks = self._symbol_address(
            self.cf, "kCFTypeDictionaryKeyCallBacks"
        )
        self.value_callbacks = self._symbol_address(
            self.cf, "kCFTypeDictionaryValueCallBacks"
        )

    def _create_private_key(self) -> int:
        key_size = ctypes.c_int32(256)
        number = self.cf.CFNumberCreate(None, 3, ctypes.byref(key_size))
        if not number:
            raise RuntimeError("software key-size creation failed")
        keys = (ctypes.c_void_p * 2)(self.attr_key_type, self.attr_key_size)
        values = (ctypes.c_void_p * 2)(self.key_type_ec, number)
        attributes = self.cf.CFDictionaryCreate(
            None,
            keys,
            values,
            2,
            self.key_callbacks,
            self.value_callbacks,
        )
        error = ctypes.c_void_p()
        try:
            if not attributes:
                raise RuntimeError("software key attributes failed")
            private_key = self.security.SecKeyCreateRandomKey(
                attributes, ctypes.byref(error)
            )
            if not private_key:
                raise RuntimeError("software key creation failed")
            return private_key
        finally:
            if attributes:
                self.cf.CFRelease(attributes)
            self.cf.CFRelease(number)
            self._release_error(error)

    def _create_data(self, value: bytes) -> int:
        buffer = (ctypes.c_uint8 * len(value)).from_buffer_copy(value)
        data = self.cf.CFDataCreate(None, buffer, len(value))
        if not data:
            raise RuntimeError("software data creation failed")
        return data

    def _data_bytes(self, data: int) -> bytes:
        length = self.cf.CFDataGetLength(data)
        pointer = self.cf.CFDataGetBytePtr(data)
        if length <= 0 or not pointer:
            raise RuntimeError("software data export failed")
        return ctypes.string_at(pointer, length)

    def _release_error(self, error: ctypes.c_void_p) -> None:
        if error.value:
            self.cf.CFRelease(error.value)

    def _constant(self, name: str) -> int:
        value = ctypes.c_void_p.in_dll(self.security, name).value
        if value is None:
            raise RuntimeError("Security.framework constant unavailable")
        return value

    @staticmethod
    def _symbol_address(library: ctypes.CDLL, name: str) -> int:
        return ctypes.addressof(ctypes.c_byte.in_dll(library, name))


class SecurityFrameworkVerificationTests(unittest.TestCase):
    def test_verifies_ephemeral_software_p256_signature_and_rejects_mutations(self) -> None:
        message = b"stage1-owner-authorization-security-framework-fixture"
        with SoftwareP256Fixture() as first, SoftwareP256Fixture() as second:
            public_key = first.public_key_x963()
            signature = first.sign(message)
            verifier = auth.SecurityFrameworkSignatureVerifier()
            self.assertTrue(verifier.verify(public_key, message, signature))
            self.assertFalse(verifier.verify(public_key, message + b"!", signature))
            changed_signature = signature[:-1] + bytes([signature[-1] ^ 1])
            self.assertFalse(verifier.verify(public_key, message, changed_signature))
            self.assertFalse(
                verifier.verify(second.public_key_x963(), message, signature)
            )

    def test_partial_cfdata_setup_releases_prior_objects(self) -> None:
        bindings = auth._SecurityFrameworkBindings.__new__(
            auth._SecurityFrameworkBindings
        )
        released: list[int] = []
        calls = 0

        def data(_: bytes) -> int:
            nonlocal calls
            calls += 1
            if calls == 2:
                raise auth.SignatureVerificationError("fixture allocation failure")
            return 101

        bindings._data = data
        bindings._release_many = lambda *values: released.extend(
            value for value in values if value
        )
        with self.assertRaises(auth.SignatureVerificationError):
            bindings.verify(b"\x04" + b"\x11" * 64, b"message", b"\x30\x00")
        self.assertEqual(released, [101])

    def test_production_adapter_uses_only_public_security_framework_apis(self) -> None:
        source = inspect.getsource(auth.SecurityFrameworkSignatureVerifier) + inspect.getsource(
            auth._SecurityFrameworkBindings
        )
        self.assertIn(SoftwareP256Fixture.CORE_FOUNDATION, source)
        self.assertIn(SoftwareP256Fixture.SECURITY, source)
        self.assertIn("SecKeyCreateWithData", source)
        self.assertIn("SecKeyVerifySignature", source)
        for forbidden in [
            "SecKeyCreateRandomKey",
            "kSecAttrIsPermanent",
            "kSecAttrApplicationTag",
            "kSecAttrTokenIDSecureEnclave",
            "openssl",
            "subprocess",
        ]:
            self.assertNotIn(forbidden, source)


if __name__ == "__main__":
    unittest.main()
