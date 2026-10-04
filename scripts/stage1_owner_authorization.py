"""Canonical Stage 1 owner-authorization primitives."""

from __future__ import annotations

import base64
import binascii
import ctypes
import dataclasses
import hashlib
import json
import os
import re
import stat
from typing import Any, Mapping, Protocol

OWNER_KEY_TAG = "dev.rohtolos.jarvis.stage1-owner-authorization.v1"
PUBLIC_KEY_SCHEMA = "jarvis.local-provider.stage1-owner-public-key.v1"
BINDING_SCHEMA = "jarvis.local-provider.stage1-owner-binding.v1"
ENVELOPE_SCHEMA = "jarvis.local-provider.stage1-owner-authorization-envelope.v1"
PAYLOAD_SCHEMA = "jarvis.local-provider.stage1-owner-authorization-payload.v1"
SIGNING_DOMAIN = b"JARVIS-STAGE1-OWNER-AUTHORIZATION-V1\0"
AUTHORIZATION_TTL_SECONDS = 300
OWNER_SIGNATURE_ALGORITHM = "ecdsa-p256-sha256-x962"
MAX_ENVELOPE_BYTES = 64 * 1024
MAX_PAYLOAD_BYTES = 48 * 1024
MAX_SIGNATURE_BYTES = 256
MAX_BINDING_BYTES = 1024 * 1024
REVOKED_REVIEW_AGGREGATES = frozenset(
    {
        "f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0",
        "380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0",
        "1c49497d67bc21f3a1a3edf49bca82be180ca517fce3380ead4b99389c9e4bc0",
    }
)
CANDIDATE_BINDING = {
    "candidate_id": "qwen35-4b-q4km-homebrew-llamacpp-0.4.1",
    "candidate_sha256": "08767112eaa0111244e5bb38381f8373615627ddb2af788a8fdd82464aa3a592",
    "review_aggregate_sha256": "c034041a5ea9b5203e5e8385b010079764ff3daf13be00d5163e934cc11d447c",
}
ARTIFACT_BINDINGS = [
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
LIMIT_BINDING = {
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

_MIN_I64 = -(1 << 63)
_MAX_I64 = (1 << 63) - 1


class SignatureVerifier(Protocol):
    def verify(
        self, public_key_x963: bytes, message: bytes, signature_der: bytes
    ) -> bool: ...


class SignatureVerificationError(RuntimeError):
    """Signature verification could not establish authenticity."""


class SecurityFrameworkSignatureVerifier:
    CORE_FOUNDATION_PATH = (
        "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation"
    )
    SECURITY_PATH = "/System/Library/Frameworks/Security.framework/Security"

    def __init__(self) -> None:
        try:
            self._bindings = _SecurityFrameworkBindings(
                self.CORE_FOUNDATION_PATH,
                self.SECURITY_PATH,
            )
        except Exception as error:
            raise SignatureVerificationError(
                "signature verification platform is unavailable"
            ) from error

    def verify(
        self, public_key_x963: bytes, message: bytes, signature_der: bytes
    ) -> bool:
        if (
            len(public_key_x963) != 65
            or public_key_x963[0] != 0x04
            or len(signature_der) < 2
            or len(signature_der) > MAX_SIGNATURE_BYTES
            or signature_der[0] != 0x30
        ):
            return False
        try:
            return self._bindings.verify(public_key_x963, message, signature_der)
        except SignatureVerificationError:
            raise
        except Exception as error:
            raise SignatureVerificationError("signature verification failed") from error


class _SecurityFrameworkBindings:
    def __init__(self, core_foundation_path: str, security_path: str) -> None:
        self.cf = ctypes.CDLL(core_foundation_path)
        self.security = ctypes.CDLL(security_path)
        self._bind_functions()
        self.attr_key_type = self._security_constant("kSecAttrKeyType")
        self.attr_key_class = self._security_constant("kSecAttrKeyClass")
        self.attr_key_size = self._security_constant("kSecAttrKeySizeInBits")
        self.key_type_ec = self._security_constant("kSecAttrKeyTypeECSECPrimeRandom")
        self.key_class_public = self._security_constant("kSecAttrKeyClassPublic")
        self.algorithm = self._security_constant(
            "kSecKeyAlgorithmECDSASignatureMessageX962SHA256"
        )
        self.key_callbacks = self._symbol_address(
            self.cf, "kCFTypeDictionaryKeyCallBacks"
        )
        self.value_callbacks = self._symbol_address(
            self.cf, "kCFTypeDictionaryValueCallBacks"
        )

    def verify(
        self, public_key_x963: bytes, message: bytes, signature_der: bytes
    ) -> bool:
        public_data = 0
        message_data = 0
        signature_data = 0
        number = 0
        attributes = 0
        public_key = 0
        key_error = ctypes.c_void_p()
        verify_error = ctypes.c_void_p()
        try:
            public_data = self._data(public_key_x963)
            message_data = self._data(message)
            signature_data = self._data(signature_der)
            key_size = ctypes.c_int32(256)
            number = self.cf.CFNumberCreate(None, 3, ctypes.byref(key_size))
            if not number:
                raise SignatureVerificationError("signature verification setup failed")
            keys = (ctypes.c_void_p * 3)(
                self.attr_key_type,
                self.attr_key_class,
                self.attr_key_size,
            )
            values = (ctypes.c_void_p * 3)(
                self.key_type_ec,
                self.key_class_public,
                number,
            )
            attributes = self.cf.CFDictionaryCreate(
                None,
                keys,
                values,
                3,
                self.key_callbacks,
                self.value_callbacks,
            )
            if not attributes:
                raise SignatureVerificationError("signature verification setup failed")
            public_key = self.security.SecKeyCreateWithData(
                public_data,
                attributes,
                ctypes.byref(key_error),
            )
            if not public_key:
                raise SignatureVerificationError("signature verification setup failed")
            verified = self.security.SecKeyVerifySignature(
                public_key,
                self.algorithm,
                message_data,
                signature_data,
                ctypes.byref(verify_error),
            )
            return bool(verified)
        finally:
            self._release_error(key_error)
            self._release_error(verify_error)
            self._release_many(
                public_key,
                attributes,
                number,
                public_data,
                message_data,
                signature_data,
            )

    def _bind_functions(self) -> None:
        self.cf.CFDataCreate.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_uint8),
            ctypes.c_long,
        ]
        self.cf.CFDataCreate.restype = ctypes.c_void_p
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
        self.security.SecKeyCreateWithData.argtypes = [
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self.security.SecKeyCreateWithData.restype = ctypes.c_void_p
        self.security.SecKeyVerifySignature.argtypes = [
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self.security.SecKeyVerifySignature.restype = ctypes.c_ubyte

    def _data(self, value: bytes) -> int:
        if not value:
            raise SignatureVerificationError("signature verification input is invalid")
        buffer = (ctypes.c_uint8 * len(value)).from_buffer_copy(value)
        data = self.cf.CFDataCreate(None, buffer, len(value))
        if not data:
            raise SignatureVerificationError("signature verification setup failed")
        return data

    def _security_constant(self, name: str) -> int:
        value = ctypes.c_void_p.in_dll(self.security, name).value
        if value is None:
            raise SignatureVerificationError("signature verification setup failed")
        return value

    @staticmethod
    def _symbol_address(library: ctypes.CDLL, name: str) -> int:
        return ctypes.addressof(ctypes.c_byte.in_dll(library, name))

    def _release_error(self, error: ctypes.c_void_p) -> None:
        if error.value:
            self.cf.CFRelease(error.value)

    def _release_many(self, *values: int) -> None:
        for value in values:
            if value:
                self.cf.CFRelease(value)


@dataclasses.dataclass(frozen=True)
class ParsedEnvelope:
    key_id: str
    payload_bytes: bytes
    payload: dict[str, Any]
    signature_der: bytes
    envelope_sha256: str


@dataclasses.dataclass(frozen=True)
class ParsedFrozenBinding:
    binding_sha256: str
    candidate: dict[str, Any]
    review: dict[str, Any]
    artifacts: list[dict[str, Any]]
    limits: dict[str, Any]
    model_transfer: str
    resource_roster: list[dict[str, Any]]


@dataclasses.dataclass(frozen=True)
class VerifiedOwnerAuthorization:
    payload: dict[str, Any]
    envelope_sha256: str
    generation: str
    receipt_name: str
    nonce_sha256: str
    key_id: str
    binding_sha256: str
    purpose: str


def parse_frozen_binding(binding_bytes: bytes) -> ParsedFrozenBinding:
    binding = decode_json_object(binding_bytes, MAX_BINDING_BYTES)
    if set(binding) != {
        "artifacts",
        "candidate",
        "limits",
        "model_transfer",
        "resource_roster",
        "review",
        "schema",
        "state",
    }:
        raise ValueError("invalid owner binding fields")
    if binding["schema"] != BINDING_SCHEMA or binding["state"] != "frozen":
        raise ValueError("owner binding is not frozen")
    if binding["candidate"] != CANDIDATE_BINDING:
        raise ValueError("owner binding candidate mismatch")
    review = binding["review"]
    if not isinstance(review, dict) or set(review) != {
        "review_aggregate_sha256",
        "review_manifest_sha256",
        "review_sidecar_sha256",
    }:
        raise ValueError("owner binding review mismatch")
    for field in review:
        decode_lower_hex(_require_string(review, field), 32)
    if review["review_aggregate_sha256"] in REVOKED_REVIEW_AGGREGATES:
        raise ValueError("owner binding review is revoked")
    if binding["artifacts"] != ARTIFACT_BINDINGS:
        raise ValueError("owner binding artifact mismatch")
    if binding["limits"] != LIMIT_BINDING:
        raise ValueError("owner binding limit mismatch")
    if binding["model_transfer"] != "blocked":
        raise ValueError("owner binding model policy mismatch")
    resource_roster = binding["resource_roster"]
    if not isinstance(resource_roster, list) or not resource_roster:
        raise ValueError("owner binding resource roster is empty")
    paths: list[str] = []
    for record in resource_roster:
        if not isinstance(record, dict) or set(record) != {
            "bytes",
            "mode",
            "path",
            "sha256",
        }:
            raise ValueError("owner binding resource record mismatch")
        path = _require_string(record, "path")
        if not _safe_resource_path(path) or path in paths:
            raise ValueError("owner binding resource path mismatch")
        if (
            type(record["bytes"]) is not int
            or record["bytes"] < 0
            or type(record["mode"]) is not int
            or not 0 <= record["mode"] <= 0o777
        ):
            raise ValueError("owner binding resource value mismatch")
        decode_lower_hex(_require_string(record, "sha256"), 32)
        paths.append(path)
    if paths != sorted(paths):
        raise ValueError("owner binding resource order mismatch")
    copied = json.loads(canonical_json_bytes(binding))
    return ParsedFrozenBinding(
        binding_sha256=hashlib.sha256(binding_bytes).hexdigest(),
        candidate=copied["candidate"],
        review=copied["review"],
        artifacts=copied["artifacts"],
        limits=copied["limits"],
        model_transfer=copied["model_transfer"],
        resource_roster=copied["resource_roster"],
    )


def _safe_resource_path(value: str) -> bool:
    return (
        bool(value)
        and "\\" not in value
        and "\0" not in value
        and not value.startswith("/")
        and all(part not in ("", ".", "..") for part in value.split("/"))
        and re.fullmatch(r"[\x20-\x7e]+", value) is not None
    )


def verify_owner_authorization(
    envelope_bytes: bytes,
    nonce: bytes,
    anchor_bytes: bytes,
    binding_bytes: bytes,
    directory_stats: Mapping[str, os.stat_result],
    now_unix: int,
    verifier: SignatureVerifier,
) -> VerifiedOwnerAuthorization:
    public_key = _parse_pinned_anchor(anchor_bytes)
    envelope = parse_and_verify_envelope(envelope_bytes, public_key, verifier)
    binding = parse_frozen_binding(binding_bytes)
    payload = envelope.payload
    if set(payload) != {
        "artifacts",
        "binding_sha256",
        "candidate",
        "expires_at_unix",
        "generation",
        "issued_at_unix",
        "ledger",
        "limits",
        "model_transfer",
        "nonce_sha256",
        "purpose",
        "receipt",
        "review",
        "schema",
        "state",
        "workspace_parent",
    }:
        raise ValueError("invalid owner authorization payload fields")
    if (
        payload["schema"] != PAYLOAD_SCHEMA
        or payload["state"] != "granted"
        or payload["purpose"] != "stage1_bottle_transfer"
        or payload["model_transfer"] != "blocked"
    ):
        raise ValueError("invalid owner authorization policy")

    generation = _require_string(payload, "generation")
    decode_lower_hex(generation, 16)
    if type(nonce) is not bytes or len(nonce) != 32:
        raise ValueError("invalid owner authorization nonce")
    nonce_sha256 = _require_string(payload, "nonce_sha256")
    decode_lower_hex(nonce_sha256, 32)
    if nonce_sha256 != hashlib.sha256(nonce).hexdigest():
        raise ValueError("owner authorization nonce mismatch")

    issued_at_unix = _require_i64(payload, "issued_at_unix")
    expires_at_unix = _require_i64(payload, "expires_at_unix")
    if (
        expires_at_unix != issued_at_unix + AUTHORIZATION_TTL_SECONDS
        or type(now_unix) is not int
        or not _MIN_I64 <= now_unix <= _MAX_I64
        or now_unix < issued_at_unix
        or now_unix >= expires_at_unix
    ):
        raise ValueError("owner authorization is outside its validity window")

    binding_sha256 = _require_string(payload, "binding_sha256")
    decode_lower_hex(binding_sha256, 32)
    if binding_sha256 != binding.binding_sha256:
        raise ValueError("owner authorization binding mismatch")
    if payload["candidate"] != binding.candidate:
        raise ValueError("owner authorization candidate mismatch")
    review = payload["review"]
    if not isinstance(review, dict) or set(review) != {
        "review_aggregate_sha256",
        "review_manifest_sha256",
    }:
        raise ValueError("owner authorization review mismatch")
    if review != {
        "review_aggregate_sha256": binding.review["review_aggregate_sha256"],
        "review_manifest_sha256": binding.review["review_manifest_sha256"],
    }:
        raise ValueError("owner authorization review mismatch")
    if review["review_aggregate_sha256"] in REVOKED_REVIEW_AGGREGATES:
        raise ValueError("owner authorization review is revoked")
    if payload["artifacts"] != binding.artifacts:
        raise ValueError("owner authorization artifact mismatch")
    if payload["limits"] != binding.limits:
        raise ValueError("owner authorization limit mismatch")

    receipt = payload["receipt"]
    if not isinstance(receipt, dict) or set(receipt) != {
        "device",
        "inode",
        "name",
        "uid",
    }:
        raise ValueError("owner authorization receipt mismatch")
    receipt_name = _require_string(receipt, "name")
    if receipt_name != f"stage1-{generation}.json":
        raise ValueError("owner authorization receipt name mismatch")
    if set(directory_stats) != {"ledger", "workspace_parent", "receipt_parent"}:
        raise ValueError("owner authorization directory descriptor shape mismatch")
    _verify_directory_identity(payload["ledger"], directory_stats["ledger"])
    _verify_directory_identity(
        payload["workspace_parent"], directory_stats["workspace_parent"]
    )
    _verify_directory_identity(receipt, directory_stats["receipt_parent"], allow_name=True)

    copied_payload = json.loads(canonical_json_bytes(payload))
    return VerifiedOwnerAuthorization(
        payload=copied_payload,
        envelope_sha256=envelope.envelope_sha256,
        generation=generation,
        receipt_name=receipt_name,
        nonce_sha256=nonce_sha256,
        key_id=envelope.key_id,
        binding_sha256=binding_sha256,
        purpose="stage1_bottle_transfer",
    )


def _parse_pinned_anchor(anchor_bytes: bytes) -> bytes:
    anchor = decode_json_object(anchor_bytes, 4096)
    if set(anchor) != {
        "algorithm",
        "key_id",
        "public_key_x963_base64",
        "schema",
        "state",
    }:
        raise ValueError("invalid pinned owner anchor fields")
    if (
        _require_string(anchor, "schema") != PUBLIC_KEY_SCHEMA
        or _require_string(anchor, "state") != "pinned"
        or _require_string(anchor, "algorithm") != OWNER_SIGNATURE_ALGORITHM
    ):
        raise ValueError("owner anchor is not pinned")
    public_key = decode_canonical_base64(
        _require_string(anchor, "public_key_x963_base64"), 65
    )
    if len(public_key) != 65 or public_key[0] != 0x04:
        raise ValueError("invalid pinned owner public key")
    key_id = _require_string(anchor, "key_id")
    decode_lower_hex(key_id, 32)
    if key_id != hashlib.sha256(public_key).hexdigest():
        raise ValueError("pinned owner key identifier mismatch")
    return public_key


def _require_i64(value: Mapping[str, Any], field: str) -> int:
    item = value.get(field)
    if type(item) is not int or not _MIN_I64 <= item <= _MAX_I64:
        raise ValueError(f"{field} must be a signed integer")
    return item


def _verify_directory_identity(
    signed: object,
    observed: os.stat_result,
    *,
    allow_name: bool = False,
) -> None:
    expected_keys = {"device", "inode", "uid"} | ({"name"} if allow_name else set())
    if not isinstance(signed, dict) or set(signed) != expected_keys:
        raise ValueError("owner authorization directory identity mismatch")
    identity = []
    for field in ("device", "inode", "uid"):
        value = signed[field]
        if type(value) is not int or value < 0:
            raise ValueError("owner authorization directory identity mismatch")
        identity.append(value)
    if (
        not stat.S_ISDIR(observed.st_mode)
        or stat.S_IMODE(observed.st_mode) != 0o700
        or observed.st_uid != os.getuid()
        or tuple(identity) != (observed.st_dev, observed.st_ino, observed.st_uid)
    ):
        raise PermissionError("owner authorization directory identity mismatch")


def parse_and_verify_envelope(
    envelope_bytes: bytes,
    public_key_x963: bytes,
    verifier: SignatureVerifier,
) -> ParsedEnvelope:
    if len(public_key_x963) != 65 or public_key_x963[0] != 0x04:
        raise ValueError("invalid pinned public key")
    envelope = decode_json_object(envelope_bytes, MAX_ENVELOPE_BYTES)
    if set(envelope) != {
        "algorithm",
        "key_id",
        "payload_base64",
        "schema",
        "signature_base64",
    }:
        raise ValueError("invalid authorization envelope fields")
    if _require_string(envelope, "schema") != ENVELOPE_SCHEMA:
        raise ValueError("invalid authorization envelope schema")
    if _require_string(envelope, "algorithm") != OWNER_SIGNATURE_ALGORITHM:
        raise ValueError("invalid authorization signature algorithm")

    key_id = _require_string(envelope, "key_id")
    key_id_bytes = decode_lower_hex(key_id, 32)
    if key_id_bytes != hashlib.sha256(public_key_x963).digest():
        raise ValueError("authorization key identifier does not match")

    payload_bytes = decode_canonical_base64(
        _require_string(envelope, "payload_base64"), MAX_PAYLOAD_BYTES
    )
    payload = decode_json_object(payload_bytes, MAX_PAYLOAD_BYTES)
    signature_der = decode_canonical_base64(
        _require_string(envelope, "signature_base64"), MAX_SIGNATURE_BYTES
    )
    if len(signature_der) < 2 or signature_der[0] != 0x30:
        raise ValueError("invalid DER signature")

    try:
        verified = verifier.verify(
            public_key_x963,
            SIGNING_DOMAIN + payload_bytes,
            signature_der,
        )
    except SignatureVerificationError:
        raise
    except Exception as error:
        raise SignatureVerificationError("signature verification failed") from error
    if not verified:
        raise SignatureVerificationError("signature verification failed")

    return ParsedEnvelope(
        key_id=key_id,
        payload_bytes=payload_bytes,
        payload=payload,
        signature_der=signature_der,
        envelope_sha256=hashlib.sha256(envelope_bytes).hexdigest(),
    )


def _require_string(value: dict[str, Any], field: str) -> str:
    item = value.get(field)
    if type(item) is not str:
        raise ValueError(f"{field} must be a string")
    return item


def canonical_json_bytes(value: object) -> bytes:
    _validate_canonical_value(value)
    return json.dumps(
        value,
        ensure_ascii=True,
        allow_nan=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("ascii")


def decode_json_object(payload: bytes, maximum_bytes: int) -> dict[str, Any]:
    if maximum_bytes < 0 or not payload or len(payload) > maximum_bytes:
        raise ValueError("invalid canonical JSON size")
    try:
        text = payload.decode("utf-8")
        value = json.loads(
            text,
            object_pairs_hook=_unique_object,
            parse_constant=_reject_number,
            parse_float=_reject_number,
        )
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as error:
        raise ValueError("invalid canonical JSON") from error
    if not isinstance(value, dict):
        raise ValueError("canonical JSON must be an object")
    if canonical_json_bytes(value) != payload:
        raise ValueError("JSON is not canonical")
    return value


def decode_canonical_base64(value: str, maximum_bytes: int) -> bytes:
    if maximum_bytes < 0 or not isinstance(value, str):
        raise ValueError("invalid base64")
    try:
        encoded = value.encode("ascii")
        decoded = base64.b64decode(encoded, validate=True)
    except (UnicodeEncodeError, binascii.Error, ValueError) as error:
        raise ValueError("invalid base64") from error
    if len(decoded) > maximum_bytes or base64.b64encode(decoded) != encoded:
        raise ValueError("invalid base64")
    return decoded


def decode_lower_hex(value: str, expected_bytes: int) -> bytes:
    if (
        expected_bytes < 0
        or not isinstance(value, str)
        or len(value) != expected_bytes * 2
        or any(character not in "0123456789abcdef" for character in value)
    ):
        raise ValueError("invalid lowercase hex")
    return bytes.fromhex(value)


def _validate_canonical_value(value: object) -> None:
    if value is None or isinstance(value, float):
        raise ValueError("value is outside the canonical JSON domain")
    if isinstance(value, bool):
        return
    if isinstance(value, int):
        if not _MIN_I64 <= value <= _MAX_I64:
            raise ValueError("integer is outside signed 64-bit range")
        return
    if isinstance(value, str):
        if any(ord(character) < 0x20 or ord(character) > 0x7E for character in value):
            raise ValueError("string is not printable ASCII")
        return
    if isinstance(value, list):
        for item in value:
            _validate_canonical_value(item)
        return
    if isinstance(value, dict):
        for key, item in value.items():
            if not isinstance(key, str):
                raise ValueError("object key is not a string")
            _validate_canonical_value(key)
            _validate_canonical_value(item)
        return
    raise ValueError("value is outside the canonical JSON domain")


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate object key")
        value[key] = item
    return value


def _reject_number(_: str) -> None:
    raise ValueError("non-integer number is forbidden")