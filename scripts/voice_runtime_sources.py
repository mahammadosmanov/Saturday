#!/usr/bin/env python3
"""Acquire and verify the two pinned voice-runtime source archives."""

from __future__ import annotations

import dataclasses
import hashlib
import json
import os
import re
import shutil
import stat
import tarfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Iterable

_REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
_MANIFEST_PATH = _REPOSITORY_ROOT / "src-tauri/resources/voice-runtime-sources.json"
_SHA256 = re.compile(r"[0-9a-f]{64}\Z")
_GIT_OBJECT = re.compile(r"[0-9a-f]{40}\Z")
_ALLOWED_ARCHIVE_HOST = "codeload.github.com"
_ALLOWED_REDIRECT_HOSTS = frozenset({"github.com", _ALLOWED_ARCHIVE_HOST})
_REDIRECT_LIMIT = 3
_CONNECT_TIMEOUT_SECONDS = 15
_DOWNLOAD_DEADLINE_SECONDS = 300
_CHUNK_BYTES = 1024 * 1024


@dataclasses.dataclass(frozen=True)
class SourceCandidate:
    asset_id: str
    archive_url: str
    archive_root: str
    archive_bytes: int
    archive_sha256: str
    commit: str
    tree: str
    max_archive_bytes: int = 64 * 1024 * 1024
    max_expanded_bytes: int = 256 * 1024 * 1024
    max_file_bytes: int = 16 * 1024 * 1024
    max_files: int = 12_000

    def validate(self) -> None:
        if not self.asset_id or not self.archive_root or "/" in self.archive_root:
            raise ValueError("invalid source candidate identity")
        if not _SHA256.fullmatch(self.archive_sha256):
            raise ValueError("invalid pinned archive digest")
        if not _GIT_OBJECT.fullmatch(self.commit) or not _GIT_OBJECT.fullmatch(self.tree):
            raise ValueError("invalid pinned Git identity")
        if not 0 < self.archive_bytes <= self.max_archive_bytes:
            raise ValueError("pinned archive size exceeds its limit")
        if not 0 < self.max_file_bytes <= self.max_expanded_bytes or self.max_files <= 0:
            raise ValueError("invalid extraction limits")
        validate_archive_url(self, self.archive_url)


def source_archive_filename(candidate: SourceCandidate) -> str:
    names = {"whisper-cpp": "whisper.tar.gz", "llama-cpp": "llama.tar.gz"}
    try:
        return names[candidate.asset_id]
    except KeyError as error:
        raise ValueError("source candidate has no fixed archive filename") from error


def source_tree_directory_name(candidate: SourceCandidate) -> str:
    if candidate.asset_id not in {"whisper-cpp", "llama-cpp"}:
        raise ValueError("source candidate has no fixed extraction directory")
    return f"{candidate.archive_root}-tree"

@dataclasses.dataclass(frozen=True)
class ExtractionRecord:
    archive_bytes: int
    archive_sha256: str
    git_tree: str
    file_count: int
    expanded_bytes: int


@dataclasses.dataclass(frozen=True)
class AcquiredSource:
    candidate: SourceCandidate
    archive_path: Path
    source_root: Path
    effective_url: str
    extraction: ExtractionRecord


def _unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate manifest key")
        result[key] = value
    return result


def load_candidates(manifest_path: Path = _MANIFEST_PATH) -> tuple[SourceCandidate, ...]:
    raw = manifest_path.read_bytes()
    if not raw or len(raw) > 256 * 1024:
        raise ValueError("source manifest size is invalid")
    document = json.loads(raw, object_pairs_hook=_unique_object)
    if not isinstance(document, dict) or document.get("schema") != "saturday.voice-runtime-sources.v1":
        raise ValueError("unsupported voice runtime source manifest")
    if document.get("state") not in {
        "sources_verified_build_recipe_pending_independent_review",
        "sources_verified_build_blocked",
        "independently_reviewed_and_main_approved",
        "runtime_build_accepted",
    }:
        raise ValueError("source manifest is not an approved source candidate set")
    policy = document.get("source_archive_policy")
    runtimes = document.get("runtimes")
    if not isinstance(policy, dict) or not isinstance(runtimes, list) or len(runtimes) != 2:
        raise ValueError("source manifest roster is invalid")
    if (
        policy.get("allowed_origins") != ["https://codeload.github.com"]
        or policy.get("allow_redirect_origins")
        != ["https://github.com", "https://codeload.github.com"]
        or policy.get("max_redirects") != _REDIRECT_LIMIT
        or policy.get("connect_timeout_seconds") != _CONNECT_TIMEOUT_SECONDS
        or policy.get("overall_download_deadline_seconds") != _DOWNLOAD_DEADLINE_SECONDS
        or policy.get("allow_symlinks") is not False
        or policy.get("allow_hardlinks") is not False
        or policy.get("allow_special_files") is not False
        or policy.get("build_network") is not False
    ):
        raise ValueError("source archive policy is not the reviewed policy")
    candidates: list[SourceCandidate] = []
    for item in runtimes:
        if not isinstance(item, dict):
            raise ValueError("source manifest candidate is invalid")
        candidate = SourceCandidate(
            asset_id=_required_string(item, "id"),
            archive_url=_required_string(item, "archive_url"),
            archive_root=f"{_required_string(item, 'repository').split('/', 1)[1]}-{_required_string(item, 'commit')}",
            archive_bytes=_required_int(item, "archive_bytes"),
            archive_sha256=_required_string(item, "archive_sha256"),
            commit=_required_string(item, "commit"),
            tree=_required_string(item, "tree"),
            max_archive_bytes=_required_int(policy, "max_archive_bytes_each"),
            max_expanded_bytes=_required_int(policy, "max_expanded_bytes_each"),
            max_file_bytes=_required_int(policy, "max_file_bytes"),
            max_files=_required_int(policy, "max_files_each"),
        )
        candidate.validate()
        signature = item.get("commit_signature")
        if not isinstance(signature, dict) or signature.get("verified") is not True or signature.get("reason") != "valid":
            raise ValueError("pinned source commit signature is not verified")
        header = item.get("header")
        if not isinstance(header, dict) or not _SHA256.fullmatch(str(header.get("sha256", ""))):
            raise ValueError("pinned native API header identity is missing")
        candidates.append(candidate)
    if {candidate.asset_id for candidate in candidates} != {"whisper-cpp", "llama-cpp"}:
        raise ValueError("unexpected runtime source roster")
    return tuple(candidates)


def _required_string(value: dict[str, object], key: str) -> str:
    result = value.get(key)
    if not isinstance(result, str) or not result:
        raise ValueError(f"source manifest field {key} is invalid")
    return result


def _required_int(value: dict[str, object], key: str) -> int:
    result = value.get(key)
    if isinstance(result, bool) or not isinstance(result, int) or result <= 0:
        raise ValueError(f"source manifest field {key} is invalid")
    return result


def _validate_https_origin(url: str, allowed_hosts: frozenset[str]) -> urllib.parse.SplitResult:
    try:
        parsed = urllib.parse.urlsplit(url)
        port = parsed.port
    except ValueError as error:
        raise ValueError("invalid source URL") from error
    if (
        parsed.scheme != "https"
        or parsed.hostname is None
        or parsed.hostname.lower() not in allowed_hosts
        or parsed.username is not None
        or parsed.password is not None
        or port not in (None, 443)
        or parsed.fragment
        or "\\" in parsed.path
        or "\x00" in parsed.path
    ):
        raise ValueError("source URL is outside the reviewed HTTPS origins")
    return parsed


def validate_archive_url(candidate: SourceCandidate, url: str) -> None:
    if url != candidate.archive_url:
        raise ValueError("archive URL differs from the pinned candidate")
    parsed = _validate_https_origin(url, frozenset({_ALLOWED_ARCHIVE_HOST}))
    expected_suffix = f"/{candidate.commit}"
    if not parsed.path.endswith(expected_suffix) or parsed.query:
        raise ValueError("archive URL is not commit-addressed")


def validate_redirect_url(url: str) -> None:
    _validate_https_origin(url, _ALLOWED_REDIRECT_HOSTS)


def _sha256_file(path: Path, maximum_bytes: int) -> tuple[str, int]:
    digest = hashlib.sha256()
    count = 0
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(path, flags)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_size > maximum_bytes:
            raise ValueError("source archive is not a bounded regular file")
        while True:
            block = os.read(fd, _CHUNK_BYTES)
            if not block:
                break
            count += len(block)
            if count > maximum_bytes:
                raise ValueError("source archive exceeds its transfer limit")
            digest.update(block)
    finally:
        os.close(fd)
    return digest.hexdigest(), count


def _normal_member_path(name: str, archive_root: str) -> tuple[str, ...]:
    if not name or "\\" in name or "\x00" in name or name.startswith("/") or len(name) > 4096:
        raise ValueError("archive member path is invalid")
    normalized = name[:-1] if name.endswith("/") else name
    parts = normalized.split("/")
    if any(part in ("", ".", "..") or len(part.encode("utf-8", "strict")) > 255 for part in parts):
        raise ValueError("archive member path exceeds safe filesystem bounds")
    if parts[0] != archive_root:
        raise ValueError("archive member escapes its pinned root")
    return tuple(parts)


def _preflight_tar(
    archive: tarfile.TarFile, candidate: SourceCandidate
) -> tuple[list[tuple[tarfile.TarInfo, tuple[str, ...], int]], int]:
    members = archive.getmembers()
    if not members or len(members) > candidate.max_files:
        raise ValueError("archive member count is invalid")
    seen: set[tuple[str, ...]] = set()
    checked: list[tuple[tarfile.TarInfo, tuple[str, ...], int]] = []
    expanded = 0
    file_count = 0
    root_is_directory = False
    for member in members:
        parts = _normal_member_path(member.name, candidate.archive_root)
        if parts in seen:
            raise ValueError("archive contains a duplicate member path")
        seen.add(parts)
        if member.mode & 0o7000:
            raise ValueError("archive member has special permission bits")
        if member.isdir():
            if len(parts) == 1:
                if root_is_directory:
                    raise ValueError("archive root is duplicated")
                root_is_directory = True
            mode = 0o755
        elif member.isfile() and not member.issym() and not member.islnk():
            if member.size < 0 or member.size > candidate.max_file_bytes:
                raise ValueError("archive file exceeds its per-file limit")
            expanded += member.size
            file_count += 1
            if expanded > candidate.max_expanded_bytes or file_count > candidate.max_files:
                raise ValueError("archive exceeds its expanded-data limits")
            mode = 0o755 if member.mode & 0o111 else 0o644
        else:
            raise ValueError("archive contains a link or unsupported member type")
        checked.append((member, parts, mode))
    if not root_is_directory:
        raise ValueError("archive does not contain its expected root directory")
    return checked, expanded


def _open_or_create_directory(parent_fd: int, name: str) -> int:
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        return os.open(name, flags, dir_fd=parent_fd)
    except FileNotFoundError:
        os.mkdir(name, mode=0o700, dir_fd=parent_fd)
        return os.open(name, flags, dir_fd=parent_fd)


def _ensure_directory(root_fd: int, parts: Iterable[str]) -> int:
    current = os.dup(root_fd)
    try:
        for part in parts:
            next_fd = _open_or_create_directory(current, part)
            os.close(current)
            current = next_fd
        return current
    except BaseException:
        os.close(current)
        raise


def _extract_members(
    archive: tarfile.TarFile,
    checked: list[tuple[tarfile.TarInfo, tuple[str, ...], int]],
    destination: Path,
) -> None:
    root_fd = os.open(
        destination,
        os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0),
    )
    try:
        for member, parts, mode in checked:
            relative = parts[1:]
            if member.isdir():
                directory_fd = _ensure_directory(root_fd, relative)
                os.close(directory_fd)
                continue
            parent_fd = _ensure_directory(root_fd, relative[:-1])
            try:
                fd = os.open(
                    relative[-1],
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
                    0o600,
                    dir_fd=parent_fd,
                )
                try:
                    source = archive.extractfile(member)
                    if source is None:
                        raise ValueError("archive regular file payload is missing")
                    with source:
                        remaining = member.size
                        while remaining:
                            block = source.read(min(_CHUNK_BYTES, remaining))
                            if not block:
                                raise ValueError("archive file payload is truncated")
                            view = memoryview(block)
                            while view:
                                written = os.write(fd, view)
                                if written <= 0:
                                    raise OSError("short write while extracting source")
                                view = view[written:]
                            remaining -= len(block)
                        if source.read(1):
                            raise ValueError("archive file payload exceeds its declared size")
                    os.fchmod(fd, mode)
                    os.fsync(fd)
                finally:
                    os.close(fd)
            finally:
                os.close(parent_fd)
    finally:
        os.close(root_fd)


def _git_object_id(kind: bytes, payload: bytes) -> bytes:
    return hashlib.sha1(kind + b" " + str(len(payload)).encode("ascii") + b"\0" + payload).digest()


def _git_tree_id(root: Path) -> str:
    def visit(directory: Path) -> bytes:
        entries: list[tuple[bytes, bytes]] = []
        with os.scandir(directory) as scanned:
            disk_entries = list(scanned)
        for entry in disk_entries:
            # Git metadata is outside the source tree; ignore only root-level .git.
            if directory == root and entry.name == ".git":
                if entry.is_symlink() or not entry.is_dir(follow_symlinks=False):
                    raise ValueError("root Git metadata must be a directory")
                continue
            name = entry.name.encode("utf-8", "strict")
            if entry.is_symlink():
                raise ValueError("extracted source unexpectedly contains a symlink")
            if entry.is_dir(follow_symlinks=False):
                mode = b"40000"
                object_id = visit(Path(entry.path))
                sort_key = name + b"/"
            elif entry.is_file(follow_symlinks=False):
                mode_bits = os.stat(entry.path, follow_symlinks=False).st_mode
                if not stat.S_ISREG(mode_bits):
                    raise ValueError("extracted source contains a special file")
                mode = b"100755" if mode_bits & 0o111 else b"100644"
                with open(entry.path, "rb") as file:
                    data = file.read()
                object_id = _git_object_id(b"blob", data)
                sort_key = name
            else:
                raise ValueError("extracted source contains an unsupported member")
            entries.append((sort_key, mode + b" " + name + b"\0" + object_id))
        payload = b"".join(item for _, item in sorted(entries, key=lambda entry: entry[0]))
        return _git_object_id(b"tree", payload)

    return visit(root).hex()


def safe_extract_verified_archive(
    archive_path: Path, destination: Path, candidate: SourceCandidate
) -> ExtractionRecord:
    candidate.validate()
    digest, archive_bytes = _sha256_file(archive_path, candidate.max_archive_bytes)
    if archive_bytes != candidate.archive_bytes or digest != candidate.archive_sha256:
        raise ValueError("source archive size or digest does not match the reviewed candidate")
    if destination.exists() or destination.is_symlink():
        raise ValueError("source extraction destination already exists")
    parent = destination.parent.resolve(strict=True)
    destination = parent / destination.name
    created = False
    try:
        with tarfile.open(archive_path, mode="r:gz") as archive:
            checked, expanded_bytes = _preflight_tar(archive, candidate)
            destination.mkdir(mode=0o700)
            created = True
            _extract_members(archive, checked, destination)
        actual_tree = _git_tree_id(destination)
        if actual_tree != candidate.tree:
            raise ValueError("extracted source tree does not match the signed commit tree")
        file_count = sum(1 for member, _, _ in checked if member.isfile())
        return ExtractionRecord(archive_bytes, digest, actual_tree, file_count, expanded_bytes)
    except (tarfile.TarError, OSError, ValueError, UnicodeError) as error:
        if created and destination.exists() and not destination.is_symlink():
            shutil.rmtree(destination)
        if isinstance(error, ValueError):
            raise
        raise ValueError("source archive is malformed or could not be safely extracted") from error


class _PinnedRedirectHandler(urllib.request.HTTPRedirectHandler):
    def __init__(self) -> None:
        super().__init__()
        self.redirect_count = 0

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        self.redirect_count += 1
        if self.redirect_count > _REDIRECT_LIMIT:
            raise ValueError("source redirect limit exceeded")
        validate_redirect_url(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def _download_candidate(candidate: SourceCandidate, staging_root: Path) -> tuple[Path, str]:
    candidate.validate()
    if staging_root.is_symlink() or not staging_root.is_dir():
        raise ValueError("source staging root is not a private directory")
    if stat.S_IMODE(staging_root.stat().st_mode) & 0o077:
        raise ValueError("source staging root permissions are too broad")
    redirect_handler = _PinnedRedirectHandler()
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), redirect_handler)
    request = urllib.request.Request(
        candidate.archive_url,
        headers={"User-Agent": "Saturday voice source acquisition/1", "Accept-Encoding": "identity"},
        method="GET",
    )
    final_path = staging_root / source_archive_filename(candidate)
    if final_path.exists() or final_path.is_symlink():
        raise ValueError("source archive destination already exists")
    fd, temporary_name = tempfile.mkstemp(prefix=".source-", suffix=".part", dir=staging_root)
    temporary_path = Path(temporary_name)
    started = time.monotonic()
    deadline = started + _DOWNLOAD_DEADLINE_SECONDS
    digest = hashlib.sha256()
    received = 0
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb") as output:
            with opener.open(request, timeout=_CONNECT_TIMEOUT_SECONDS) as response:
                effective_url = response.geturl()
                validate_redirect_url(effective_url)
                content_length = response.headers.get("Content-Length")
                if content_length is not None:
                    try:
                        declared = int(content_length)
                    except ValueError as error:
                        raise ValueError("source server returned an invalid content length") from error
                    if declared < 0 or declared > candidate.max_archive_bytes:
                        raise ValueError("source response exceeds its transfer limit")
                while True:
                    if time.monotonic() > deadline:
                        raise ValueError("source transfer deadline exceeded")
                    block = response.read(_CHUNK_BYTES)
                    if not block:
                        break
                    received += len(block)
                    if received > candidate.max_archive_bytes:
                        raise ValueError("source response exceeds its transfer limit")
                    output.write(block)
                    digest.update(block)
            output.flush()
            os.fsync(output.fileno())
        if received != candidate.archive_bytes or digest.hexdigest() != candidate.archive_sha256:
            raise ValueError("downloaded source size or digest does not match the reviewed candidate")
        if redirect_handler.redirect_count > _REDIRECT_LIMIT:
            raise ValueError("source redirect limit exceeded")
        final_created = False
        os.rename(temporary_path, final_path)
        final_created = True
        directory_fd = os.open(staging_root, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
        return final_path, effective_url
    except (OSError, urllib.error.URLError, ValueError):
        if temporary_path.exists() and not temporary_path.is_symlink():
            temporary_path.unlink()
        if locals().get("final_created") and final_path.exists() and not final_path.is_symlink():
            final_path.unlink()
        raise


def acquire_sources(staging_parent: Path) -> tuple[Path, tuple[AcquiredSource, ...]]:
    """Download and safely extract the fixed, reviewed source roster into a unique private root."""
    parent = staging_parent.resolve(strict=True)
    if staging_parent.is_symlink() or not parent.is_dir():
        raise ValueError("source staging parent is not a real directory")
    if stat.S_IMODE(parent.stat().st_mode) & 0o077:
        raise ValueError("source staging parent permissions are too broad")
    staging_root = Path(tempfile.mkdtemp(prefix="saturday-voice-sources-", dir=parent))
    os.chmod(staging_root, 0o700)
    acquired: list[AcquiredSource] = []
    try:
        for candidate in load_candidates():
            archive_path, effective_url = _download_candidate(candidate, staging_root)
            source_root = staging_root / source_tree_directory_name(candidate)
            extraction = safe_extract_verified_archive(archive_path, source_root, candidate)
            acquired.append(AcquiredSource(candidate, archive_path, source_root, effective_url, extraction))
        return staging_root, tuple(acquired)
    except BaseException:
        shutil.rmtree(staging_root)
        raise


def _main() -> int:
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--staging-parent", type=Path, required=True)
    args = parser.parse_args()
    root, sources = acquire_sources(args.staging_parent)
    print(json.dumps({
        "staging_root": str(root),
        "sources": [
            {
                "asset_id": item.candidate.asset_id,
                "archive_bytes": item.extraction.archive_bytes,
                "archive_sha256": item.extraction.archive_sha256,
                "effective_url": item.effective_url,
                "tree": item.extraction.git_tree,
            }
            for item in sources
        ],
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(_main())
