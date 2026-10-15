#!/usr/bin/env python3
"""Emit the fixed private voice binding resource for bootstrap or signed-runtime builds."""

from __future__ import annotations

import pwd


import hashlib
import json
import os
import stat
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
APP_IDENTIFIER = "dev.rohtolos.saturday"
APPLICATION_TAG = f"{APP_IDENTIFIER}.voice.owner-key.v1"
MAX_ANCHOR_BYTES = 32 * 1024
MAX_BINDING_BYTES = 256 * 1024

def strict_json(data: bytes, limit: int, *, canonical: str | None = "sorted") -> Any:
    if not data or len(data) > limit:
        raise ValueError("voice binding input size is invalid")

    def unique_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON member")
            result[key] = value
        return result

    value = json.loads(data, object_pairs_hook=unique_pairs)
    if canonical == "sorted" and canonical_json(value, sort_keys=True) != data:
        raise ValueError("voice binding input is not canonical JSON")
    if canonical == "ordered" and canonical_json(value, sort_keys=False) != data:
        raise ValueError("voice binding input is not canonical JSON")
    return value


def canonical_json(value: Any, *, sort_keys: bool) -> bytes:
    return json.dumps(
        value, sort_keys=sort_keys, separators=(",", ":"), ensure_ascii=False
    ).encode()


def app_data_root() -> Path:
    return (
        Path(pwd.getpwuid(os.getuid()).pw_dir)
        / "Library"
        / "Application Support"
        / APP_IDENTIFIER
    )


def read_private_file(name: str, limit: int) -> bytes | None:
    path = app_data_root() / "voice" / "authorization" / name
    directories = (app_data_root(), app_data_root() / "voice", path.parent)
    try:
        for directory in directories:
            info = directory.lstat()
            if (
                stat.S_ISLNK(info.st_mode)
                or not stat.S_ISDIR(info.st_mode)
                or info.st_uid != os.getuid()
                or (
                    directory != app_data_root()
                    and stat.S_IMODE(info.st_mode) != 0o700
                )
            ):
                raise ValueError("private owner bootstrap directory is unsafe")
        info = path.lstat()
    except FileNotFoundError:
        return None
    if (
        stat.S_ISLNK(info.st_mode)
        or not stat.S_ISREG(info.st_mode)
        or stat.S_IMODE(info.st_mode) != 0o600
        or info.st_uid != os.getuid()
        or info.st_nlink != 1
        or info.st_size > limit
    ):
        raise ValueError("private owner bootstrap file permissions are unsafe")
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    try:
        opened = os.fstat(fd)
        if (opened.st_dev, opened.st_ino) != (info.st_dev, info.st_ino):
            raise ValueError("private owner bootstrap file changed while opening")
        data = bytearray()
        while len(data) <= limit:
            chunk = os.read(fd, min(4096, limit + 1 - len(data)))
            if not chunk:
                break
            data.extend(chunk)
    finally:
        os.close(fd)
    return bytes(data)


def read_bootstrap_material() -> dict[str, bytes] | None:
    record_bytes = read_private_file("owner-key-bootstrap.json", MAX_ANCHOR_BYTES)
    witness_bytes = read_private_file(
        "owner-key-bootstrap-witness.json", MAX_ANCHOR_BYTES
    )
    if record_bytes is None or witness_bytes is None:
        if record_bytes is not None or witness_bytes is not None:
            raise ValueError("owner bootstrap record and witness must exist together")
        return None
    record = strict_json(record_bytes, MAX_ANCHOR_BYTES)
    if not isinstance(record, dict) or set(record) != {"payload", "signature_der"}:
        raise ValueError("private owner bootstrap record schema is invalid")
    payload = record["payload"]
    required = {
        "schema",
        "application_tag_sha256",
        "owner_public_key_x963",
        "owner_public_key_sha256",
        "application_bundle_identifier",
        "application_team_id_sha256",
        "application_requirement_sha256",
        "issued_at_unix",
        "nonce_sha256",
    }
    if not isinstance(payload, dict) or set(payload) != required:
        raise ValueError("private owner bootstrap payload schema is invalid")
    if (
        payload["schema"] != "saturday.voice.owner-key-bootstrap.v1"
        or payload["application_bundle_identifier"] != APP_IDENTIFIER
        or payload["application_tag_sha256"]
        != hashlib.sha256(APPLICATION_TAG.encode()).hexdigest()
    ):
        raise ValueError("private owner bootstrap record belongs to another application")
    public_key = payload["owner_public_key_x963"]
    signature = record["signature_der"]
    if (
        not isinstance(public_key, list)
        or len(public_key) != 65
        or any(type(byte) is not int or not 0 <= byte <= 255 for byte in public_key)
        or public_key[0] != 4
        or payload["owner_public_key_sha256"]
        != hashlib.sha256(bytes(public_key)).hexdigest()
        or not isinstance(signature, list)
        or not 8 <= len(signature) <= 256
        or signature[0] != 48
        or any(type(byte) is not int or not 0 <= byte <= 255 for byte in signature)
        or type(payload["issued_at_unix"]) is not int
        or payload["issued_at_unix"] <= 0
        or any(
            not isinstance(payload[field], str)
            or len(payload[field]) != 64
            or any(char not in "0123456789abcdef" for char in payload[field])
            for field in (
                "application_team_id_sha256",
                "application_requirement_sha256",
                "nonce_sha256",
            )
        )
    ):
        raise ValueError("private owner bootstrap key, identity, or signature is invalid")
    witness = strict_json(witness_bytes, MAX_ANCHOR_BYTES)
    if (
        not isinstance(witness, dict)
        or set(witness) != {"payload", "signature_der"}
        or not isinstance(witness["payload"], dict)
        or witness["payload"].get("schema")
        != "saturday.voice.owner-key-bootstrap-witness.v1"
        or witness["payload"].get("owner_public_key_sha256")
        != hashlib.sha256(bytes(public_key)).hexdigest()
        or witness["payload"].get("bootstrap_record_sha256")
        != hashlib.sha256(record_bytes).hexdigest()
    ):
        raise ValueError("private owner bootstrap witness does not match its record")
    return {
        "public_key": bytes(public_key),
        "record": record_bytes,
        "witness": witness_bytes,
    }


def write_private_output(path: Path, data: bytes) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    parent = path.parent.lstat()
    if stat.S_ISLNK(parent.st_mode) or not stat.S_ISDIR(parent.st_mode):
        raise ValueError("private build directory is not a real directory")
    if parent.st_uid != os.getuid() or stat.S_IMODE(parent.st_mode) & 0o077:
        raise ValueError("private build directory ownership or permissions are unsafe")
    fd, temporary = tempfile.mkstemp(prefix=".voice-binding-", dir=path.parent)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb", closefd=True) as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory_fd = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    except BaseException:
        try:
            os.close(fd)
        except OSError:
            pass
        Path(temporary).unlink(missing_ok=True)
        raise


def bootstrap_catalog() -> bytes:
    source = ROOT / "src-tauri" / "resources" / "voice-asset-bindings.json"
    catalog = strict_json(source.read_bytes(), MAX_BINDING_BYTES, canonical=None)
    if not isinstance(catalog, dict) or not isinstance(catalog.get("owner_key"), dict):
        raise ValueError("bootstrap voice binding schema is invalid")
    catalog["owner_key"] = {
        "state": "bootstrap",
        "application_tag_sha256": hashlib.sha256(APPLICATION_TAG.encode()).hexdigest(),
    }
    return canonical_json(catalog, sort_keys=False)


def main() -> int:
    target = ROOT / "src-tauri" / "target" / "voice-owner-build"
    binding_output = target / "voice-asset-bindings.json"
    pin_output = target / "voice-owner-key-pin.json"
    bootstrap = read_bootstrap_material()
    if bootstrap is None:
        if pin_output.exists() or pin_output.is_symlink():
            raise ValueError("stale owner pin exists without signed bootstrap material")
        write_private_output(binding_output, bootstrap_catalog())
        return 0

    public_key = bootstrap["public_key"]
    record = bootstrap["record"]
    witness = bootstrap["witness"]
    pin = {
        "schema": "saturday.voice.owner-key-pin.v1",
        "application_tag_sha256": hashlib.sha256(APPLICATION_TAG.encode()).hexdigest(),
        "owner_public_key_x963_hex": public_key.hex(),
        "owner_public_key_sha256": hashlib.sha256(public_key).hexdigest(),
        "bootstrap_record_sha256": hashlib.sha256(record).hexdigest(),
        "bootstrap_record": base64.urlsafe_b64encode(record).rstrip(b"=").decode("ascii"),
        "bootstrap_witness_sha256": hashlib.sha256(witness).hexdigest(),
        "bootstrap_witness": base64.urlsafe_b64encode(witness).rstrip(b"=").decode("ascii"),
    }
    encoded_pin = canonical_json(pin, sort_keys=True) + b"\n"
    write_private_output(pin_output, encoded_pin)
    write_private_output(binding_output, bootstrap_catalog())
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"voice binding generation failed: {error}")
