from __future__ import annotations

import hashlib
import io
import tarfile
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path

from scripts.voice_runtime_sources import (
    SourceCandidate,
    _git_tree_id,
    safe_extract_verified_archive,
    validate_archive_url,
    validate_redirect_url,
)

class VoiceRuntimeSourceTests(unittest.TestCase):
    def test_pinned_candidate_url_and_redirect_allowlist_are_exact(self) -> None:
        candidate = SourceCandidate(
            asset_id="whisper-cpp",
            archive_url="https://codeload.github.com/ggml-org/whisper.cpp/tar.gz/" + "b" * 40,
            archive_root="whisper.cpp-commit",
            archive_bytes=1,
            archive_sha256="a" * 64,
            commit="b" * 40,
            tree="c" * 40,
        )
        validate_archive_url(candidate, candidate.archive_url)
        validate_redirect_url("https://github.com/ggml-org/whisper.cpp/archive/commit.tar.gz")
        for url in (
            "http://codeload.github.com/ggml-org/whisper.cpp/tar.gz/commit",
            "https://user@codeload.github.com/ggml-org/whisper.cpp/tar.gz/commit",
            "https://codeload.github.com.evil.example/ggml-org/whisper.cpp/tar.gz/commit",
            "https://evil.example/ggml-org/whisper.cpp/tar.gz/commit",
        ):
            with self.subTest(url=url), self.assertRaises(ValueError):
                validate_redirect_url(url)
        with self.assertRaises(ValueError):
            validate_archive_url(candidate, candidate.archive_url + "?download=1")

    def test_exact_archive_hash_and_git_tree_are_verified_before_acceptance(self) -> None:
        root = "whisper.cpp-commit"
        archive = make_tar(
            [
                (root, "dir", b"", 0o775),
                (root + "/a", "file", b"hello", 0o664),
            ]
        )
        with tempfile.TemporaryDirectory() as temporary:
            archive_path = Path(temporary) / "source.tar.gz"
            archive_path.write_bytes(archive)
            candidate = candidate_for(archive, root)
            extracted = Path(temporary) / "tree"
            result = safe_extract_verified_archive(archive_path, extracted, candidate)
            self.assertEqual(result.archive_sha256, candidate.archive_sha256)
            self.assertEqual(result.archive_bytes, candidate.archive_bytes)
            self.assertEqual(result.git_tree, "8917ce3833750c0f424a16966ef3391aafe880c1")
            self.assertEqual((extracted / "a").read_bytes(), b"hello")
            self.assertEqual((extracted / "a").stat().st_mode & 0o777, 0o644)


    def test_root_git_metadata_is_excluded_but_invalid_git_entries_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "tree"
            root.mkdir()
            (root / "tracked").write_bytes(b"pinned")
            expected = _git_tree_id(root)
            (root / ".git").mkdir()
            (root / ".git" / "untracked").write_bytes(b"ignored only at source root")
            self.assertEqual(_git_tree_id(root), expected)
            (root / ".git" / "untracked").unlink()
            (root / ".git").rmdir()
            (root / ".git").symlink_to(root / "tracked")
            with self.assertRaises(ValueError):
                _git_tree_id(root)

    def test_traversal_symlink_hardlink_and_special_members_are_rejected(self) -> None:
        cases = (
            [("repo-commit", "dir", b"", 0o775), ("repo-commit/../escape", "file", b"x", 0o664)],
            [("repo-commit", "dir", b"", 0o775), ("repo-commit/link", "symlink", b"../outside", 0o777)],
            [("repo-commit", "dir", b"", 0o775), ("repo-commit/link", "hardlink", b"repo-commit/file", 0o664)],
            [("repo-commit", "dir", b"", 0o775), ("repo-commit/device", "device", b"", 0o600)],
        )
        for members in cases:
            with self.subTest(kind=members[-1][1]):
                archive = make_tar(members)
                candidate = candidate_for(archive, "repo-commit")
                with tempfile.TemporaryDirectory() as temporary:
                    archive_path = Path(temporary) / "source.tar.gz"
                    archive_path.write_bytes(archive)
                    with self.assertRaises(ValueError):
                        safe_extract_verified_archive(archive_path, Path(temporary) / "tree", candidate)

    def test_archive_hash_mismatch_is_rejected_before_extraction(self) -> None:
        archive = make_tar(
            [("repo-commit", "dir", b"", 0o775), ("repo-commit/a", "file", b"hello", 0o664)]
        )
        candidate = replace(candidate_for(archive, "repo-commit"), archive_sha256="0" * 64)
        with tempfile.TemporaryDirectory() as temporary:
            archive_path = Path(temporary) / "source.tar.gz"
            archive_path.write_bytes(archive)
            extracted = Path(temporary) / "tree"
            with self.assertRaises(ValueError):
                safe_extract_verified_archive(archive_path, extracted, candidate)
            self.assertFalse(extracted.exists())

    def test_tree_mismatch_removes_unaccepted_extraction(self) -> None:
        archive = make_tar(
            [("repo-commit", "dir", b"", 0o775), ("repo-commit/a", "file", b"hello", 0o664)]
        )
        candidate = candidate_for(archive, "repo-commit", tree="0" * 40)
        with tempfile.TemporaryDirectory() as temporary:
            archive_path = Path(temporary) / "source.tar.gz"
            archive_path.write_bytes(archive)
            extracted = Path(temporary) / "tree"
            with self.assertRaises(ValueError):
                safe_extract_verified_archive(archive_path, extracted, candidate)
            self.assertFalse(extracted.exists())


def candidate_for(archive: bytes, root: str, tree: str = "8917ce3833750c0f424a16966ef3391aafe880c1") -> SourceCandidate:
    return SourceCandidate(
        asset_id="whisper-cpp",
        archive_url="https://codeload.github.com/ggml-org/whisper.cpp/tar.gz/" + "b" * 40,
        archive_root=root,
        archive_bytes=len(archive),
        archive_sha256=hashlib.sha256(archive).hexdigest(),
        commit="b" * 40,
        tree=tree,
    )


def make_tar(members: list[tuple[str, str, bytes, int]]) -> bytes:
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as archive:
        for name, kind, body, mode in members:
            info = tarfile.TarInfo(name)
            info.mode = mode
            if kind == "dir":
                info.type = tarfile.DIRTYPE
                archive.addfile(info)
            elif kind == "file":
                info.size = len(body)
                archive.addfile(info, io.BytesIO(body))
            elif kind == "symlink":
                info.type = tarfile.SYMTYPE
                info.linkname = body.decode()
                archive.addfile(info)
            elif kind == "hardlink":
                info.type = tarfile.LNKTYPE
                info.linkname = body.decode()
                archive.addfile(info)
            else:
                info.type = tarfile.CHRTYPE
                archive.addfile(info)
    return output.getvalue()


if __name__ == "__main__":
    unittest.main()
