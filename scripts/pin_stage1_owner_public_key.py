"""Pin the Stage 1 public trust anchor from app-owned bootstrap state."""

from __future__ import annotations

import base64
import hashlib
import os
import pwd
import stat
import sys
from pathlib import Path

from scripts import stage1_owner_authorization as auth

OWNER_SIGNATURE_ALGORITHM = "ecdsa-p256-sha256-x962"
PUBLIC_KEY_RESOURCE = (
    Path(__file__).resolve().parents[1]
    / "src-tauri"
    / "resources"
    / "stage1-owner-public-key.json"
)
_BOOTSTRAP_COMPONENTS = (
    "Library",
    "Application Support",
    "dev.rohtolos.saturday",
    "Stage1OwnerAuthorization",
    "bootstrap",
)
_BOOTSTRAP_NAME = "owner-public-key.json"
_MAX_RECORD_BYTES = 4096
_UNPROVISIONED = auth.canonical_json_bytes(
    {"schema": auth.PUBLIC_KEY_SCHEMA, "state": "unprovisioned"}
)


class PinError(ValueError):
    """Bootstrap or tracked public-key state is invalid."""


def pin_from_canonical_bootstrap() -> dict[str, object]:
    bootstrap = _read_account_bootstrap()
    pinned, key_id = _validate_bootstrap_record(bootstrap)
    current = _read_tracked_resource(PUBLIC_KEY_RESOURCE)
    if current != pinned:
        if current != _UNPROVISIONED:
            raise PinError("tracked public-key anchor does not match")
        _atomic_replace(PUBLIC_KEY_RESOURCE, pinned)
    return {"key_id": key_id, "pinned": True}


def _effective_account_home() -> Path:
    try:
        home = Path(pwd.getpwuid(os.geteuid()).pw_dir)
    except (KeyError, OSError) as error:
        raise PinError("effective account home is unavailable") from error
    if not home.is_absolute():
        raise PinError("effective account home is invalid")
    return home


def _read_account_bootstrap() -> bytes:
    expected_uid = os.geteuid()
    flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    descriptors: list[int] = []
    file_descriptor = -1
    try:
        current = os.open(_effective_account_home(), flags)
        descriptors.append(current)
        _validate_directory(current, expected_uid, private=False)
        for index, component in enumerate(_BOOTSTRAP_COMPONENTS):
            current = os.open(component, flags, dir_fd=current)
            descriptors.append(current)
            _validate_directory(current, expected_uid, private=index >= 2)
        file_flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0)
        file_flags |= getattr(os, "O_NOFOLLOW", 0)
        file_descriptor = os.open(_BOOTSTRAP_NAME, file_flags, dir_fd=current)
        metadata = os.fstat(file_descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_nlink != 1
            or metadata.st_uid != expected_uid
            or metadata.st_mode & 0o777 != 0o600
            or metadata.st_size <= 0
            or metadata.st_size > _MAX_RECORD_BYTES
        ):
            raise PinError("bootstrap file metadata is invalid")
        return _read_bounded_descriptor(file_descriptor, _MAX_RECORD_BYTES)
    except OSError as error:
        raise PinError("bootstrap path is invalid") from error
    finally:
        if file_descriptor >= 0:
            os.close(file_descriptor)
        for descriptor in reversed(descriptors):
            os.close(descriptor)


def _validate_directory(descriptor: int, expected_uid: int, private: bool) -> None:
    metadata = os.fstat(descriptor)
    mode = metadata.st_mode & 0o777
    invalid_mode = mode != 0o700 if private else mode & 0o022 != 0
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or metadata.st_uid != expected_uid
        or invalid_mode
    ):
        raise PinError("bootstrap directory metadata is invalid")


def _read_bounded_descriptor(descriptor: int, maximum_bytes: int) -> bytes:
    chunks: list[bytes] = []
    remaining = maximum_bytes + 1
    while remaining > 0:
        chunk = os.read(descriptor, remaining)
        if not chunk:
            break
        chunks.append(chunk)
        remaining -= len(chunk)
    payload = b"".join(chunks)
    if not payload or len(payload) > maximum_bytes:
        raise PinError("record size is invalid")
    return payload


def _validate_bootstrap_record(payload: bytes) -> tuple[bytes, str]:
    try:
        record = auth.decode_json_object(payload, _MAX_RECORD_BYTES)
        if set(record) != {
            "algorithm",
            "application_tag",
            "key_id",
            "public_key_x963_base64",
            "schema",
        }:
            raise PinError("bootstrap fields are invalid")
        if _require_string(record, "schema") != auth.PUBLIC_KEY_SCHEMA:
            raise PinError("bootstrap schema is invalid")
        if _require_string(record, "algorithm") != OWNER_SIGNATURE_ALGORITHM:
            raise PinError("bootstrap algorithm is invalid")
        if _require_string(record, "application_tag") != auth.OWNER_KEY_TAG:
            raise PinError("bootstrap application tag is invalid")
        public_key = auth.decode_canonical_base64(
            _require_string(record, "public_key_x963_base64"), 65
        )
        if len(public_key) != 65 or public_key[0] != 0x04:
            raise PinError("bootstrap public key is invalid")
        key_id_bytes = auth.decode_lower_hex(_require_string(record, "key_id"), 32)
        expected_key_id = hashlib.sha256(public_key).digest()
        if key_id_bytes != expected_key_id:
            raise PinError("bootstrap key identifier is invalid")
    except (KeyError, TypeError, ValueError) as error:
        if isinstance(error, PinError):
            raise
        raise PinError("bootstrap record is invalid") from error

    key_id = expected_key_id.hex()
    pinned = auth.canonical_json_bytes(
        {
            "algorithm": OWNER_SIGNATURE_ALGORITHM,
            "key_id": key_id,
            "public_key_x963_base64": base64.b64encode(public_key).decode("ascii"),
            "schema": auth.PUBLIC_KEY_SCHEMA,
            "state": "pinned",
        }
    )
    return pinned, key_id


def _read_tracked_resource(path: Path) -> bytes:
    try:
        metadata = path.lstat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
            raise PinError("tracked public-key resource is unsafe")
        with path.open("rb") as handle:
            payload = handle.read(_MAX_RECORD_BYTES + 1)
    except OSError as error:
        raise PinError("tracked public-key resource is unavailable") from error
    if not payload or len(payload) > _MAX_RECORD_BYTES:
        raise PinError("tracked public-key resource size is invalid")
    return payload


def _atomic_replace(path: Path, payload: bytes) -> None:
    directory_flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_CLOEXEC", 0)
    directory_flags |= getattr(os, "O_NOFOLLOW", 0)
    directory_descriptor = os.open(path.parent, directory_flags)
    temporary_name = f".{path.name}.tmp"
    file_descriptor = -1
    created = False
    try:
        file_flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
        file_flags |= getattr(os, "O_CLOEXEC", 0)
        file_flags |= getattr(os, "O_NOFOLLOW", 0)
        file_descriptor = os.open(
            temporary_name,
            file_flags,
            0o644,
            dir_fd=directory_descriptor,
        )
        created = True
        os.fchmod(file_descriptor, 0o644)
        view = memoryview(payload)
        while view:
            written = os.write(file_descriptor, view)
            if written <= 0:
                raise OSError("short write")
            view = view[written:]
        os.fsync(file_descriptor)
        os.close(file_descriptor)
        file_descriptor = -1
        os.replace(
            temporary_name,
            path.name,
            src_dir_fd=directory_descriptor,
            dst_dir_fd=directory_descriptor,
        )
        created = False
        os.fsync(directory_descriptor)
    finally:
        if file_descriptor >= 0:
            os.close(file_descriptor)
        if created:
            try:
                os.unlink(temporary_name, dir_fd=directory_descriptor)
            except FileNotFoundError:
                pass
        os.close(directory_descriptor)


def _require_string(record: dict[str, object], field: str) -> str:
    value = record[field]
    if type(value) is not str:
        raise PinError(f"{field} is not a string")
    return value


def main(arguments: list[str] | None = None) -> int:
    arguments = sys.argv[1:] if arguments is None else arguments
    if arguments:
        print("SATURDAY_STAGE1_OWNER_PUBLIC_KEY_PIN_ERROR:arguments", file=sys.stderr)
        return 64
    try:
        metadata = pin_from_canonical_bootstrap()
    except (OSError, PinError):
        print("SATURDAY_STAGE1_OWNER_PUBLIC_KEY_PIN_ERROR:invalid", file=sys.stderr)
        return 78
    print(auth.canonical_json_bytes(metadata).decode("ascii"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
