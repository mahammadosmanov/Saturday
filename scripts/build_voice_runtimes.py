#!/usr/bin/env python3
"""Build the pinned Whisper/llama static worker archives from reviewed source trees."""

from __future__ import annotations

import hashlib
import json
import os
import re
import io
import ctypes
import resource
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Iterable

_REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
if str(_REPOSITORY_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPOSITORY_ROOT))

from scripts.voice_runtime_sources import (  # noqa: E402
    AcquiredSource,
    SourceCandidate,
    load_candidates,
    source_archive_filename,
    source_tree_directory_name,
)

_RECIPE_PATH = _REPOSITORY_ROOT / "src-tauri/resources/voice-runtime-build-recipe.json"
_SOURCE_MANIFEST_PATH = _REPOSITORY_ROOT / "src-tauri/resources/voice-runtime-sources.json"
_SOURCE_LOADER_PATH = _REPOSITORY_ROOT / "scripts/voice_runtime_sources.py"
_TOOLCHAIN_ROOT = Path("/Library/Developer/CommandLineTools")
_CLANG = _TOOLCHAIN_ROOT / "usr/bin/clang"
_LIBTOOL = _TOOLCHAIN_ROOT / "usr/bin/libtool"
_SANDBOX_EXEC = Path("/usr/bin/sandbox-exec")
_SDK = _TOOLCHAIN_ROOT / "SDKs/MacOSX.sdk"
_TARGET = "arm64-apple-macosx12.0.0"
_RESOURCE_DIR = _TOOLCHAIN_ROOT / "usr/lib/clang/21"
_MINIMUM_OS = "12.0"
_CXX_INCLUDE_DIRS = (
    _SDK / "usr/include/c++/v1",
    _SDK / "usr/local/include",
    _RESOURCE_DIR / "include",
    _SDK / "usr/include",
)
_C_INCLUDE_DIRS = (
    _SDK / "usr/local/include",
    _RESOURCE_DIR / "include",
    _SDK / "usr/include",
)
_FRAMEWORK_DIRS = (
    _SDK / "System/Library/Frameworks",
    _SDK / "System/Library/SubFrameworks",
    _SDK / "Library/Frameworks",
)
_MAX_PLAN_BYTES = 16 * 1024 * 1024
_MAX_OBJECT_BYTES = 256 * 1024 * 1024
_MAX_BUILD_WORKSPACE_BYTES = 8 * 1024 * 1024 * 1024
_MAX_OUTPUT_BYTES = 1024 * 1024 * 1024
_MAX_BUILD_DEADLINE_SECONDS = 3600
_MAX_LEAF_DEADLINE_SECONDS = 300
_EXPECTED_TOOL_SHA256 = {
    str(_CLANG): "1590ac950a3d627817d09ade5cb60b2115f17a72182a3141e010b4bcc482a0c9",
    str(_LIBTOOL): "2840879786eb8868f03c3d378eef6c71d8416e15d8b14aebbf0c0cb80ad81c24",
    str(_SANDBOX_EXEC): "58839ef01b4eef8aac0d2aa8f9d1c074ae45aafe3533965b030672450064acc8",
}
_EXPECTED_SDK_SETTINGS_SHA256 = "90a738dbf3fd88e7f6d35646e19d3a8a716923b9773ce1b67260466d9ceed370"
_EXPECTED_GGML_CPU_FP16_FORMAT = (
    "AArch64 __fp16 uses IEEE binary16 by default; pinned Clang defines __ARM_FP16_FORMAT_IEEE; "
    "-mfp16-format=ieee is only needed on 32-bit Arm"
)
_EXPECTED_GNUC_VERSION = "4.2.1"
_COMPILER_INPUT_ROOTS = {
    "macos_sdk": _SDK,
    "clang_resources": _RESOURCE_DIR,
}
_TOOLCHAIN_RUNTIME_FILES = {
    "libLTO": (
        Path("/Library/Developer/CommandLineTools/usr/lib/libLTO.dylib"),
        "08bd424518673ef9371ded084a3d614a65da784f2d898226262eb8e203801732",
    )
}


def _canonical_sdk_path(path: Path) -> Path:
    try:
        relative = path.relative_to(_SDK)
    except ValueError:
        return path
    return _SDK.resolve(strict=True) / relative


_LLAMA_CORE_SOURCES = (
    "llama.cpp", "llama-adapter.cpp", "llama-arch.cpp", "llama-batch.cpp",
    "llama-chat.cpp", "llama-context.cpp", "llama-cparams.cpp", "llama-grammar.cpp",
    "llama-graph.cpp", "llama-hparams.cpp", "llama-impl.cpp", "llama-io.cpp",
    "llama-kv-cache.cpp", "llama-kv-cache-iswa.cpp", "llama-kv-cache-dsa.cpp",
    "llama-kv-cache-dsa-iswa.cpp", "llama-kv-cache-msa.cpp", "llama-kv-cache-dsv4.cpp",
    "llama-memory.cpp", "llama-memory-hybrid.cpp", "llama-memory-hybrid-iswa.cpp",
    "llama-memory-hybrid-idx.cpp", "llama-memory-recurrent.cpp", "llama-mmap.cpp",
    "llama-model-loader.cpp", "llama-model-saver.cpp", "llama-model.cpp", "llama-quant.cpp",
    "llama-sampler.cpp", "llama-vocab.cpp", "unicode-data.cpp", "unicode.cpp",
)
_GGML_BASE_SOURCES = (
    "ggml.c", "ggml.cpp", "ggml-alloc.c", "ggml-backend.cpp", "ggml-backend-meta.cpp",
    "ggml-opt.cpp", "ggml-threading.cpp", "ggml-quants.c", "gguf.cpp",
)
_GGML_AGGREGATE_SOURCES = ("ggml-backend-dl.cpp", "ggml-backend-reg.cpp")
_GGML_CPU_SOURCES = (
    "ggml-cpu/ggml-cpu.c", "ggml-cpu/ggml-cpu.cpp", "ggml-cpu/repack.cpp",
    "ggml-cpu/hbm.cpp", "ggml-cpu/quants.c", "ggml-cpu/traits.cpp",
    "ggml-cpu/amx/amx.cpp", "ggml-cpu/amx/mmq.cpp", "ggml-cpu/tiled/tiled.cpp",
    "ggml-cpu/tiled/tiled-kernel.cpp", "ggml-cpu/binary-ops.cpp", "ggml-cpu/unary-ops.cpp",
    "ggml-cpu/vec.cpp", "ggml-cpu/ops.cpp", "ggml-cpu/arch/arm/quants.c",
    "ggml-cpu/arch/arm/repack.cpp",
)
_GGML_METAL_SOURCES = (
    "ggml-metal.cpp", "ggml-metal-device.m", "ggml-metal-device.cpp", "ggml-metal-common.cpp",
    "ggml-metal-context.m", "ggml-metal-fusion.cpp", "ggml-metal-ops.cpp", "ggml-metal-tuning.cpp",
)
_METAL_KERNELS = (
    "fa_aux", "fa_f16", "fa_f32", "fa_q4_0", "fa_q4_1", "fa_q5_0", "fa_q5_1", "fa_q8_0",
    "fa_vec_f16", "fa_vec_f32", "fa_vec_q4_0", "fa_vec_q4_1", "fa_vec_q5_0", "fa_vec_q5_1",
    "fa_vec_q8_0", "mul_mv", "mul_mv_mma", "mul_mm", "quantize", "softmax", "norm", "unary",
    "binbcast", "reduce", "tri", "ssm", "wkv", "gated_delta_net", "solve_tri", "rope", "conv",
    "upscale", "argsort", "pool", "misc",
)
_INTERNAL_METAL_INCLUDES = (
    b'#include "common.h"', b'#include "dequantize.h"', b'#include "quantize.h"',
    b'#include "fa_common.metal"', b'#include "fa_vec_common.metal"',
)
_EXPECTED_CMAKE_OPTIONS = {
    "BUILD_SHARED_LIBS": False,
    "GGML_BACKEND_DL": False,
    "GGML_NATIVE": False,
    "GGML_CPU_ARM_ARCH": "",
    "GGML_CPU": True,
    "GGML_CPU_ALL_VARIANTS": False,
    "GGML_CPU_REPACK": True,
    "GGML_CPU_KLEIDIAI": False,
    "GGML_ACCELERATE": True,
    "GGML_BLAS": False,
    "GGML_METAL": True,
    "GGML_METAL_EMBED_LIBRARY": True,
    "GGML_METAL_MACOSX_VERSION_MIN": "12.0",
    "GGML_METAL_NDEBUG": False,
    "GGML_METAL_SHADER_DEBUG": False,
    "GGML_OPENMP": False,
    "GGML_OPENMP_FETCH": False,
    "GGML_CCACHE": False,
    "GGML_LLAMAFILE": False,
    "GGML_CUDA": False,
    "GGML_HIP": False,
    "GGML_VULKAN": False,
    "GGML_SYCL": False,
    "GGML_OPENCL": False,
    "GGML_RPC": False,
    "GGML_CPU_HBM": False,
    "GGML_SCHED_NO_REALLOC": False,
    "GGML_ALL_WARNINGS": True,
    "GGML_FATAL_WARNINGS": False,
    "LLAMA_ALL_WARNINGS": True,
    "LLAMA_FATAL_WARNINGS": False,
    "WHISPER_ALL_WARNINGS": True,
    "WHISPER_FATAL_WARNINGS": False,
    "LLAMA_OPENSSL": False,
    "LLAMA_SUBPROCESS": False,
    "LLAMA_LLGUIDANCE": False,
    "LLAMA_BUILD_TESTS": False,
    "LLAMA_BUILD_TOOLS": False,
    "LLAMA_BUILD_EXAMPLES": False,
    "LLAMA_BUILD_SERVER": False,
    "LLAMA_BUILD_APP": False,
    "LLAMA_BUILD_UI": False,
    "LLAMA_USE_PREBUILT_UI": False,
    "LLAMA_BUILD_COMMON": False,
    "LLAMA_TOOLS_INSTALL": False,
    "LLAMA_TESTS_INSTALL": False,
    "WHISPER_BUILD_IS_DEV": False,
    "WHISPER_BUILD_TESTS": False,
    "WHISPER_BUILD_EXAMPLES": False,
    "WHISPER_BUILD_SERVER": False,
    "WHISPER_CURL": False,
    "WHISPER_SDL2": False,
    "WHISPER_COREML": False,
    "WHISPER_OPENVINO": False,
    "WHISPER_VITISAI": False,
    "WHISPER_COMMON_FFMPEG": False,
    "WHISPER_USE_SYSTEM_GGML": False,
    "WHISPER_USE_SYSTEM_LLAMA": False,
}


@dataclass(frozen=True)
class TranslationUnit:
    provider: str
    target: str
    relative_path: str
    language: str
    defines: tuple[str, ...]
    include_dirs: tuple[str, ...]
    object_path: str
    unity_members: tuple[str, ...] = ()


def _json_no_duplicates(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def _read_json(path: Path, maximum_bytes: int = _MAX_PLAN_BYTES) -> dict[str, object]:
    data = path.read_bytes()
    if not data or len(data) > maximum_bytes:
        raise ValueError("manifest size is outside its limit")
    value = json.loads(data, object_pairs_hook=_json_no_duplicates)
    if not isinstance(value, dict):
        raise ValueError("manifest root must be an object")
    return value


def _sha256(path: Path, maximum_bytes: int | None = None) -> tuple[str, int]:
    digest = hashlib.sha256()
    count = 0
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(path, flags)
    try:
        metadata = os.fstat(fd)
        if not stat.S_ISREG(metadata.st_mode):
            raise ValueError("build input is not a regular file")
        while True:
            block = os.read(fd, 1024 * 1024)
            if not block:
                break
            count += len(block)
            if maximum_bytes is not None and count > maximum_bytes:
                raise ValueError("build input exceeds its fixed size limit")
            digest.update(block)
    finally:
        os.close(fd)
    return digest.hexdigest(), count

def _sha256_fd(descriptor: int, maximum_bytes: int | None = None) -> tuple[str, int]:
    digest = hashlib.sha256()
    count = 0
    while True:
        chunk = os.read(descriptor, 1024 * 1024)
        if not chunk:
            break
        count += len(chunk)
        if maximum_bytes is not None and count > maximum_bytes:
            raise ValueError("build input exceeds its fixed size limit")
        digest.update(chunk)
    return digest.hexdigest(), count

def _open_directory(path: Path) -> int:
    """Open an absolute directory through held, no-follow directory descriptors."""
    absolute = Path(os.path.abspath(path))
    if not absolute.is_absolute():
        raise ValueError("directory path must be absolute")
    descriptor = os.open("/", os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0))
    try:
        for component in absolute.parts[1:]:
            child = os.open(
                component,
                os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0),
                dir_fd=descriptor,
            )
            os.close(descriptor)
            descriptor = child
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _open_relative_file(root_fd: int, relative: str, maximum_bytes: int):
    """Open a regular file below a held root, refusing symlinks in every path component."""
    if not _is_safe_relative(relative) or maximum_bytes < 0:
        raise ValueError("source path or size limit is invalid")
    parts = PurePosixPath(relative).parts
    directory_fd = os.dup(root_fd)
    try:
        for component in parts[:-1]:
            child_fd = os.open(
                component,
                os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0),
                dir_fd=directory_fd,
            )
            os.close(directory_fd)
            directory_fd = child_fd
        descriptor = os.open(
            parts[-1],
            os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0),
            dir_fd=directory_fd,
        )
        try:
            metadata = os.fstat(descriptor)
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > maximum_bytes:
                raise ValueError("source input is not a bounded regular file")
            body = bytearray()
            while True:
                chunk = os.read(descriptor, min(1024 * 1024, maximum_bytes + 1 - len(body)))
                if not chunk:
                    break
                body.extend(chunk)
                if len(body) > maximum_bytes:
                    raise ValueError("source input grew beyond its fixed byte limit")
            final = os.fstat(descriptor)
            if (
                len(body) != metadata.st_size
                or (final.st_dev, final.st_ino, final.st_size) != (metadata.st_dev, metadata.st_ino, metadata.st_size)
            ):
                raise ValueError("source input changed while being read")
            return io.BytesIO(body)
        finally:
            os.close(descriptor)
    finally:
        os.close(directory_fd)


def _tree_content_seal(root: Path, *, protected: bool = False) -> tuple[str, int]:
    """Hash an immutable directory tree using descriptor-relative traversal."""
    root_fd = _open_directory(root.resolve(strict=True))
    try:
        root_metadata = os.fstat(root_fd)
        if not stat.S_ISDIR(root_metadata.st_mode):
            raise ValueError("sealed input root is not a directory")
        if protected and (root_metadata.st_uid != 0 or stat.S_IMODE(root_metadata.st_mode) & 0o022):
            raise ValueError("compiler input root is not protected by system ownership")
        total = 0

        def visit(directory_fd: int, is_root: bool = False) -> bytes:
            nonlocal total
            entries: list[tuple[bytes, bytes]] = []
            with os.scandir(directory_fd) as iterator:
                names = sorted((entry.name for entry in iterator), key=lambda name: os.fsencode(name))
            for name in names:
                if is_root and name == ".git":
                    continue
                encoded_name = os.fsencode(name)
                metadata = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
                if stat.S_ISDIR(metadata.st_mode):
                    child_fd = os.open(
                        name,
                        os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0),
                        dir_fd=directory_fd,
                    )
                    try:
                        held = os.fstat(child_fd)
                        if (held.st_dev, held.st_ino) != (metadata.st_dev, metadata.st_ino):
                            raise ValueError("compiler input directory changed during sealing")
                        child_hash = visit(child_fd)
                    finally:
                        os.close(child_fd)
                    record = b"D\0" + encoded_name + b"\0" + oct(stat.S_IMODE(metadata.st_mode)).encode() + b"\0" + child_hash
                    sort_key = encoded_name + b"/"
                elif stat.S_ISREG(metadata.st_mode):
                    file_fd = os.open(
                        name,
                        os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0),
                        dir_fd=directory_fd,
                    )
                    try:
                        held = os.fstat(file_fd)
                        if not stat.S_ISREG(held.st_mode) or (held.st_dev, held.st_ino) != (metadata.st_dev, metadata.st_ino):
                            raise ValueError("compiler input file changed during sealing")
                        content = hashlib.sha256()
                        count = 0
                        while True:
                            chunk = os.read(file_fd, 1024 * 1024)
                            if not chunk:
                                break
                            count += len(chunk)
                            total += len(chunk)
                            content.update(chunk)
                        if count != held.st_size:
                            raise ValueError("compiler input changed while sealing")
                    finally:
                        os.close(file_fd)
                    record = (
                        b"F\0" + encoded_name + b"\0" + oct(stat.S_IMODE(held.st_mode)).encode()
                        + b"\0" + str(count).encode() + b"\0" + content.digest()
                    )
                    sort_key = encoded_name
                elif stat.S_ISLNK(metadata.st_mode):
                    target = os.fsencode(os.readlink(name, dir_fd=directory_fd))
                    record = b"L\0" + encoded_name + b"\0" + target
                    sort_key = encoded_name
                else:
                    raise ValueError("compiler input tree contains an unsupported filesystem entry")
                entries.append((sort_key, record))
            digest = hashlib.sha256()
            for _, record in sorted(entries, key=lambda item: item[0]):
                digest.update(record)
                digest.update(b"\n")
            return digest.digest()

        digest = visit(root_fd, is_root=True).hex()
        return digest, total
    finally:
        os.close(root_fd)


def _is_safe_relative(path: str) -> bool:
    parsed = PurePosixPath(path)
    return (
        bool(path)
        and not parsed.is_absolute()
        and "\\" not in path
        and all(part not in ("", ".", "..") for part in parsed.parts)
        and "\x00" not in path
    )

def _compiler_input_seals() -> dict[str, dict[str, object]]:
    seals = {}
    for name, path in _COMPILER_INPUT_ROOTS.items():
        canonical = path.resolve(strict=True)
        digest, byte_count = _tree_content_seal(canonical, protected=True)
        metadata = os.stat(canonical, follow_symlinks=False)
        if metadata.st_uid != 0 or stat.S_IMODE(metadata.st_mode) & 0o022:
            raise ValueError("compiler input root is not root-owned and protected")
        seals[name] = {
            "path": str(canonical),
            "sha256": digest,
            "bytes": byte_count,
            "device": metadata.st_dev,
            "inode": metadata.st_ino,
            "uid": metadata.st_uid,
            "mode": stat.S_IMODE(metadata.st_mode),
        }
    runtime_files = {}
    for name, (path, expected_sha256) in _TOOLCHAIN_RUNTIME_FILES.items():
        parent_fd = _open_directory(path.parent)
        try:
            parent_metadata = os.fstat(parent_fd)
            if parent_metadata.st_uid != 0 or stat.S_IMODE(parent_metadata.st_mode) & 0o022:
                raise ValueError("compiler runtime parent is not root-owned and protected")
            descriptor = os.open(
                path.name,
                os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0),
                dir_fd=parent_fd,
            )
            try:
                metadata = os.fstat(descriptor)
                digest, byte_count = _sha256_fd(descriptor, 256 * 1024 * 1024)
                after = os.fstat(descriptor)
                if (
                    not stat.S_ISREG(metadata.st_mode)
                    or metadata.st_uid != 0
                    or stat.S_IMODE(metadata.st_mode) & 0o022
                    or digest != expected_sha256
                    or (metadata.st_dev, metadata.st_ino, metadata.st_size, metadata.st_mtime_ns, metadata.st_ctime_ns)
                    != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
                ):
                    raise ValueError("compiler runtime file seal or trusted-root binding mismatch")
                runtime_files[name] = {
                    "path": str(path),
                    "sha256": digest,
                    "bytes": byte_count,
                    "device": metadata.st_dev,
                    "inode": metadata.st_ino,
                    "uid": metadata.st_uid,
                    "mode": stat.S_IMODE(metadata.st_mode),
                }
            finally:
                os.close(descriptor)
        finally:
            os.close(parent_fd)
    seals["tool_runtime_files"] = runtime_files
    return seals

def _promoted_tree_sha256(
    root: Path,
    *,
    max_files: int | None = None,
    max_file_bytes: int | None = None,
    max_total_bytes: int | None = None,
    source_content_only: bool = False,
) -> str:
    root_fd = _open_directory(root.resolve(strict=True))

    try:
        file_count = 0
        total_bytes = 0
        records: list[tuple[bytes, bytes]] = []

        def visit(directory_fd: int, prefix: tuple[str, ...] = (), is_root: bool = False) -> None:
            nonlocal file_count, total_bytes
            before_directory = os.fstat(directory_fd)
            with os.scandir(directory_fd) as iterator:
                names = sorted((entry.name for entry in iterator), key=os.fsencode)
            for name in names:
                info = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
                if source_content_only and is_root and name == ".git":
                    if not stat.S_ISDIR(info.st_mode):
                        raise ValueError("source root .git entry is not a real directory")
                    continue
                relative_parts = prefix + (name,)
                relative = PurePosixPath(*relative_parts).as_posix()
                if stat.S_ISDIR(info.st_mode):
                    child_fd = os.open(
                        name,
                        os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0),
                        dir_fd=directory_fd,
                    )
                    try:
                        held = os.fstat(child_fd)
                        if (held.st_dev, held.st_ino) != (info.st_dev, info.st_ino):
                            raise ValueError("promoted artifact directory changed during hashing")
                        visit(child_fd, relative_parts)
                    finally:
                        os.close(child_fd)
                elif stat.S_ISREG(info.st_mode):
                    file_count += 1
                    if max_files is not None and file_count > max_files:
                        raise ValueError("source tree exceeds its fixed file-count bound")
                    if max_file_bytes is not None and info.st_size > max_file_bytes:
                        raise ValueError("source file exceeds its fixed size bound")
                    file_fd = os.open(
                        name,
                        os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0),
                        dir_fd=directory_fd,
                    )
                    try:
                        held = os.fstat(file_fd)
                        if (
                            not stat.S_ISREG(held.st_mode)
                            or (held.st_dev, held.st_ino) != (info.st_dev, info.st_ino)
                        ):
                            raise ValueError("promoted artifact changed while hashing")
                        content = hashlib.sha256()
                        count = 0
                        while True:
                            chunk = os.read(file_fd, 1024 * 1024)
                            if not chunk:
                                break
                            count += len(chunk)
                            if max_file_bytes is not None and count > max_file_bytes:
                                raise ValueError("source file exceeds its fixed size bound")
                            if max_total_bytes is not None and total_bytes + count > max_total_bytes:
                                raise ValueError("source tree exceeds its fixed expanded-size bound")
                            content.update(chunk)
                        after = os.fstat(file_fd)
                        if (
                            count != held.st_size
                            or (held.st_dev, held.st_ino, held.st_size, held.st_mtime_ns, held.st_ctime_ns)
                            != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
                        ):
                            raise ValueError("promoted artifact changed during hashing")
                        total_bytes += count
                        mode = (
                            b"100755" if held.st_mode & 0o111 else b"100644"
                        ) if source_content_only else format(stat.S_IMODE(held.st_mode), "04o").encode("ascii")
                    finally:
                        os.close(file_fd)
                    record = (
                        relative.encode("utf-8")
                        + b"\0"
                        + mode
                        + b"\0"
                        + str(count).encode("ascii")
                        + b"\0"
                        + content.digest()
                        + b"\n"
                    )
                    records.append((relative.encode("utf-8"), record))
                else:
                    raise ValueError("promoted artifact tree contains a symlink or special file")
            after_directory = os.fstat(directory_fd)
            if (
                (before_directory.st_dev, before_directory.st_ino, before_directory.st_mtime_ns, before_directory.st_ctime_ns)
                != (after_directory.st_dev, after_directory.st_ino, after_directory.st_mtime_ns, after_directory.st_ctime_ns)
            ):
                raise ValueError("promoted artifact directory changed during hashing")

        visit(root_fd, is_root=True)
        digest = hashlib.sha256()
        for _, record in sorted(records, key=lambda item: item[0]):
            digest.update(record)
        return digest.hexdigest()
    finally:
        os.close(root_fd)
def _source_content_seal(root: Path) -> str:
    return _promoted_tree_sha256(
        root,
        max_files=12_000,
        max_file_bytes=16 * 1024 * 1024,
        max_total_bytes=256 * 1024 * 1024,
        source_content_only=True,
    )

def _git_tree_id_fd(
    root_fd: int,
    *,
    max_files: int = 12_000,
    max_file_bytes: int = 16 * 1024 * 1024,
    max_total_bytes: int = 256 * 1024 * 1024,
) -> str:
    """Compute Git's tree ID from one stable root descriptor without following links."""
    file_count = 0
    total_bytes = 0

    def visit(directory_fd: int, is_root: bool = False) -> bytes:
        nonlocal file_count, total_bytes
        entries: list[tuple[bytes, bytes]] = []
        with os.scandir(directory_fd) as iterator:
            names = sorted((entry.name for entry in iterator), key=os.fsencode)
        for name in names:
            if is_root and name == ".git":
                info = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
                if not stat.S_ISDIR(info.st_mode):
                    raise ValueError("root Git metadata must be a directory")
                continue
            encoded_name = os.fsencode(name)
            info = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
            if stat.S_ISDIR(info.st_mode):
                child_fd = os.open(
                    name,
                    os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0),
                    dir_fd=directory_fd,
                )
                try:
                    held = os.fstat(child_fd)
                    if (held.st_dev, held.st_ino) != (info.st_dev, info.st_ino):
                        raise ValueError("source directory changed during tree verification")
                    mode, object_id, sort_key = b"40000", visit(child_fd), encoded_name + b"/"
                finally:
                    os.close(child_fd)
            elif stat.S_ISREG(info.st_mode):
                file_count += 1
                if (
                    file_count > max_files
                    or info.st_size > max_file_bytes
                    or total_bytes + info.st_size > max_total_bytes
                ):
                    raise ValueError("source tree exceeds its fixed file-count or byte budget")
                file_fd = os.open(
                    name,
                    os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0),
                    dir_fd=directory_fd,
                )
                try:
                    held = os.fstat(file_fd)
                    if (
                        (held.st_dev, held.st_ino) != (info.st_dev, info.st_ino)
                        or held.st_size != info.st_size
                    ):
                        raise ValueError("source file changed during tree verification")
                    object_hash = hashlib.sha1()
                    object_hash.update(b"blob " + str(held.st_size).encode("ascii") + b"\0")
                    count = 0
                    while True:
                        chunk = os.read(file_fd, 1024 * 1024)
                        if not chunk:
                            break
                        count += len(chunk)
                        if count > max_file_bytes or total_bytes + count > max_total_bytes:
                            raise ValueError("source file grew beyond its fixed tree budget")
                        object_hash.update(chunk)
                    if count != held.st_size:
                        raise ValueError("source file changed while tree was verified")
                    total_bytes += count
                    object_id = object_hash.digest()
                finally:
                    os.close(file_fd)
                mode = b"100755" if info.st_mode & 0o111 else b"100644"
                sort_key = encoded_name
            else:
                raise ValueError("source tree contains a symlink or special file")
            entries.append((sort_key, mode + b" " + encoded_name + b"\0" + object_id))
        tree_data = b"".join(entry for _, entry in sorted(entries, key=lambda item: item[0]))
        return hashlib.sha1(b"tree " + str(len(tree_data)).encode("ascii") + b"\0" + tree_data).digest()

    return visit(root_fd, is_root=True).hex()

def _source_tree_id(source: AcquiredSource) -> str:
    root_fd = _open_directory(source.source_root)
    try:
        return _git_tree_id_fd(
            root_fd,
            max_files=source.candidate.max_files,
            max_file_bytes=source.candidate.max_file_bytes,
            max_total_bytes=source.candidate.max_expanded_bytes,
        )
    finally:
        os.close(root_fd)


def _write_file(path: Path, data: bytes, mode: int = 0o600) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), mode)
    try:
        view = memoryview(data)
        while view:
            count = os.write(fd, view)
            if count <= 0:
                raise OSError("short write")
            view = view[count:]
        os.fsync(fd)
    finally:
        os.close(fd)


def _read_source_file(root: Path, relative: str, max_bytes: int = 16 * 1024 * 1024) -> bytes:
    if not _is_safe_relative(relative):
        raise ValueError("source path is not a safe relative path")
    path = root.joinpath(*PurePosixPath(relative).parts)
    if path.is_symlink():
        raise ValueError("source input is a symlink")
    digest, size = _sha256(path, max_bytes)
    del digest
    if size > max_bytes:
        raise ValueError("source input exceeds its size limit")
    return path.read_bytes()


def embedded_metal_source(
    kind: str,
    source: bytes,
    *,
    common: bytes,
    dequantize: bytes,
    quantize: bytes,
    fa_common: bytes,
    fa_vec_common: bytes,
    ggml_common: bytes,
    metal_impl: bytes,
) -> bytes:
    if not re.fullmatch(r"[A-Za-z0-9_]+", kind):
        raise ValueError("Metal kernel symbol is invalid")
    inputs = (source, common, dequantize, quantize, fa_common, fa_vec_common, ggml_common, metal_impl)
    if any(b"\x00" in item for item in inputs):
        raise ValueError("Metal source input contains NUL")
    headers = [common]
    optional = (
        (b'#include "dequantize.h"', dequantize),
        (b'#include "quantize.h"', quantize),
        (b'#include "fa_common.metal"', fa_common),
        (b'#include "fa_vec_common.metal"', fa_vec_common),
    )
    for needle, header in optional:
        if needle in source:
            headers.append(header)
    lines = b"".join(headers + [source]).splitlines(keepends=True)
    tokens = {
        b'__embed_ggml-common.h__': 0,
        b'#include "ggml-metal-impl.h"': 0,
    }
    transformed: list[bytes] = []
    for line in lines:
        stripped = line.strip()
        if any(token in line and stripped != token for token in (*_INTERNAL_METAL_INCLUDES, *tokens)):
            raise ValueError("Metal transform found a malformed internal directive")
        if stripped in _INTERNAL_METAL_INCLUDES or stripped == b"#pragma once":
            continue
        if stripped in tokens:
            tokens[stripped] += 1
            if stripped == b'__embed_ggml-common.h__':
                transformed.append(ggml_common)
            else:
                transformed.append(metal_impl)
            continue
        if any(token in line for token in tokens):
            raise ValueError("Metal transform token is not an exact directive line")
        transformed.append(line)
    expected_sentinel = 1 if b'#include "dequantize.h"' in source else 0
    if tokens[b'__embed_ggml-common.h__'] != expected_sentinel or tokens[b'#include "ggml-metal-impl.h"'] != 1:
        raise ValueError("Metal transform sentinel/include count differs from pinned CMake closure")
    result = b"".join(transformed)
    if len(result) > 16 * 1024 * 1024:
        raise ValueError("embedded Metal source exceeds its fixed per-kernel limit")
    return result


def embedded_metal_assembly(kind: str, relative_blob_path: str) -> bytes:
    if not re.fullmatch(r"[A-Za-z0-9_]+", kind) or not _is_safe_relative(relative_blob_path):
        raise ValueError("Metal assembly symbol or path is invalid")
    if '"' in relative_blob_path or "\n" in relative_blob_path or "\r" in relative_blob_path:
        raise ValueError("Metal assembly path contains an unsafe character")
    symbol = kind.replace("-", "_")
    text = (
        ".section __DATA,__ggml_metallib\n"
        f".globl _ggml_metallib_{symbol}_start\n"
        f"_ggml_metallib_{symbol}_start:\n"
        f'.incbin "{relative_blob_path}"\n'
        f".globl _ggml_metallib_{symbol}_end\n"
        f"_ggml_metallib_{symbol}_end:\n"
    )
    return text.encode("ascii")


def _kernel_inputs(ggml_root: Path, kind: str) -> tuple[bytes, bytes, bytes, bytes, bytes, bytes, bytes, bytes]:
    metal_root = ggml_root / "src/ggml-metal"
    return (
        _read_source_file(metal_root, "kernels/" + kind + ".metal"),
        _read_source_file(metal_root, "kernels/common.h"),
        _read_source_file(metal_root, "kernels/dequantize.h"),
        _read_source_file(metal_root, "kernels/quantize.h"),
        _read_source_file(metal_root, "kernels/fa_common.metal"),
        _read_source_file(metal_root, "kernels/fa_vec_common.metal"),
        _read_source_file(ggml_root / "src", "ggml-common.h"),
        _read_source_file(metal_root, "ggml-metal-impl.h"),
    )


def _provider_sources(source: AcquiredSource) -> dict[str, tuple[str, ...]]:
    root = source.source_root
    if source.candidate.asset_id == "llama-cpp":
        model_dir = root / "src/models"
        if model_dir.is_symlink() or not model_dir.is_dir():
            raise ValueError("pinned llama model source directory is missing")
        models = tuple(sorted("models/" + path.name for path in model_dir.iterdir() if path.suffix == ".cpp"))
        if not models:
            raise ValueError("pinned llama model source roster is empty")
        return {
            "llama": tuple("src/" + path for path in _LLAMA_CORE_SOURCES) + tuple("src/" + path for path in models),
            "ggml": tuple("ggml/src/" + path for path in _GGML_AGGREGATE_SOURCES),
            "ggml-metal": tuple("ggml/src/ggml-metal/" + path for path in _GGML_METAL_SOURCES),
            "ggml-cpu": tuple("ggml/src/" + path for path in _GGML_CPU_SOURCES),
            "ggml-base": tuple("ggml/src/" + path for path in _GGML_BASE_SOURCES),
        }
    if source.candidate.asset_id == "whisper-cpp":
        return {
            "whisper": ("src/whisper.cpp", "src/aneforge/whisper-aneforge.cpp"),
            "ggml": tuple("ggml/src/" + path for path in _GGML_AGGREGATE_SOURCES),
            "ggml-metal": tuple("ggml/src/ggml-metal/" + path for path in _GGML_METAL_SOURCES),
            "ggml-cpu": tuple("ggml/src/" + path for path in _GGML_CPU_SOURCES),
            "ggml-base": tuple("ggml/src/" + path for path in _GGML_BASE_SOURCES),
        }
    raise ValueError("source candidate has no approved static provider graph")


def _target_include_dirs(source: AcquiredSource, target: str) -> tuple[str, ...]:
    root = source.source_root
    ggml = root / "ggml"
    directories = {
        "llama": (root / "src", root / "include", ggml / "include"),
        "whisper": (root / "src", root / "include", ggml / "include"),
        "ggml": (ggml / "src", ggml / "src/ggml-cpu", ggml / "src/ggml-metal", ggml / "include"),
        "ggml-base": (ggml / "src", ggml / "include"),
        "ggml-cpu": (ggml / "src", ggml / "src/ggml-cpu", ggml, ggml / "include"),
        "ggml-metal": (ggml / "src", ggml / "src/ggml-metal", ggml / "include"),
    }
    try:
        return tuple(str(path) for path in directories[target])
    except KeyError as error:
        raise ValueError("unknown target include closure") from error


def _target_defines(provider: str, target: str) -> tuple[str, ...]:
    ggml_common = ("_XOPEN_SOURCE=600", "_DARWIN_C_SOURCE", "GGML_SCHED_MAX_COPIES=4")
    if target in {"ggml-base", "ggml-cpu", "ggml-metal", "ggml"}:
        if target == "ggml":
            return ggml_common + ("GGML_USE_CPU", "GGML_USE_METAL")
        if target == "ggml-cpu":
            return ggml_common + ("GGML_USE_CPU_REPACK", "GGML_USE_ACCELERATE", "ACCELERATE_NEW_LAPACK", "ACCELERATE_LAPACK_ILP64")
        if target == "ggml-metal":
            return ggml_common + ("GGML_METAL_EMBED_LIBRARY",)
        return ggml_common
    if target in {"whisper", "llama"} and provider in {"whisper-cpp", "llama-cpp"}:
        if target == "whisper":
            return ("GGML_USE_CPU", "GGML_USE_METAL", 'WHISPER_VERSION="1.9.5"')
        return ("GGML_USE_CPU", "GGML_USE_METAL")
    raise ValueError("unknown provider target compile definitions")




def _header_generated_files(source: AcquiredSource) -> dict[str, bytes]:
    commit = source.candidate.commit
    ggml = (
        "#pragma once\n\n"
        '#define GGML_VERSION "0.26.0"\n'
        f'#define GGML_COMMIT  "{commit}"\n'
    ).encode()
    if source.candidate.asset_id == "llama-cpp":
        return {
            "generated/llama/llama-version.h": (
                "#pragma once\n\n"
                '#define LLAMA_VERSION "0.6.0"\n'
                f'#define LLAMA_COMMIT  "{commit}"\n'
            ).encode(),
            "generated/llama/ggml-version.h": ggml,
        }
    if source.candidate.asset_id == "whisper-cpp":
        return {"generated/whisper/ggml-version.h": ggml}
    raise ValueError("source candidate has no generated version-header recipe")




def _generated_compile_includes_for_unit(unit: TranslationUnit, workspace: Path) -> tuple[str, ...]:
    if unit.target == "llama":
        return (str(workspace / "generated/llama"),)
    if unit.target == "ggml-base":
        prefix = "llama" if unit.provider == "llama-cpp" else "whisper"
        return (str(workspace / f"generated/{prefix}"),)
    return ()


def _workspace_bytes(root: Path) -> int:
    total = 0
    for base, directories, files in os.walk(root, followlinks=False):
        for name in directories:
            path = Path(base) / name
            if path.is_symlink():
                raise ValueError("build workspace contains an unexpected symlink")
        for name in files:
            path = Path(base) / name
            metadata = path.lstat()
            if not stat.S_ISREG(metadata.st_mode):
                raise ValueError("build workspace contains an unsupported file")
            total += metadata.st_size
    return total


def _sandbox_profile(workspace: Path, sources: tuple[AcquiredSource, ...]) -> str:
    roots = (
        workspace.resolve(strict=True),
        _SDK.resolve(strict=True),
        _RESOURCE_DIR.resolve(strict=True),
        Path("/System/Library").resolve(strict=True),
        Path("/usr/lib").resolve(strict=True),
        *(source.source_root.resolve(strict=True) for source in sources),
    )
    paths = tuple(str(path) for path in roots)
    read_files = tuple(str(path) for path in (_CLANG, _LIBTOOL, *[item[0] for item in _TOOLCHAIN_RUNTIME_FILES.values()]))
    write_paths = tuple(str((workspace / name).resolve(strict=True)) for name in ("generated", "obj", "output", "logs", "tmp"))
    if any(any(character in path for character in ('"', "\\", "\n", "\r")) for path in paths + read_files + write_paths):
        raise ValueError("sandbox root cannot be represented safely")
    reads = "".join(f'(allow file-read* (subpath "{path}"))\n' for path in paths)
    reads += '(allow file-read-data (literal "/"))\n'
    reads += "".join(f'(allow file-read* (literal "{path}"))\n' for path in read_files)
    writes = "".join(f'(allow file-write* (subpath "{path}"))\n' for path in write_paths)
    return (
        "(version 1)\n"
        "(deny default)\n"
        + reads
        + writes
        + f'(allow process-exec (literal "{_CLANG}") (literal "{_LIBTOOL}"))\n'
        + "(deny process-fork)\n"
        + "(allow sysctl-read)\n"
    )


def _child_resource_limits() -> dict[str, tuple[int, int, int]]:
    requested = {
        "cpu_seconds": (resource.RLIMIT_CPU, 300),
        "file_size_bytes": (resource.RLIMIT_FSIZE, 1024 * 1024 * 1024),
    }
    limits = {}
    for name, (kind, desired_soft) in requested.items():
        _, inherited_hard = resource.getrlimit(kind)
        hard = desired_soft if inherited_hard == resource.RLIM_INFINITY else min(desired_soft, inherited_hard)
        soft = hard
        if soft <= 0 or hard <= 0:
            raise OSError(f"inherited {name} limit cannot support a build leaf")
        limits[name] = (kind, soft, hard)
    return limits


def _limit_child_resources(limits: dict[str, tuple[int, int, int]]) -> None:
    for kind, soft, inherited_hard in limits.values():
        resource.setrlimit(kind, (soft, inherited_hard))


class _DarwinRusagePrefix(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16), ("values", ctypes.c_uint64 * 8)]


def _physical_footprint_bytes(pid: int) -> int:
    libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    proc_pid_rusage = libproc.proc_pid_rusage
    proc_pid_rusage.argtypes = (ctypes.c_int, ctypes.c_int, ctypes.c_void_p)
    proc_pid_rusage.restype = ctypes.c_int
    buffer = ctypes.create_string_buffer(512)
    if proc_pid_rusage(pid, 4, buffer) != 0:
        error = ctypes.get_errno()
        raise OSError(error, "proc_pid_rusage(RUSAGE_INFO_V4) failed")
    return _DarwinRusagePrefix.from_buffer(buffer).values[7]

def _stop_leaf(process: subprocess.Popen[bytes]) -> int:
    try:
        process.send_signal(signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        return process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        process.kill()
        return process.wait()

def _redact_failure_text(text: str, path_prefixes: tuple[tuple[str, str], ...]) -> str:
    for prefix, replacement in path_prefixes:
        if prefix:
            text = text.replace(prefix, replacement)
    return text


def _failure_tail(path: Path, maximum_bytes: int, path_prefixes: tuple[tuple[str, str], ...]) -> str:
    try:
        with path.open("rb") as stream:
            stream.seek(0, os.SEEK_END)
            length = min(stream.tell(), maximum_bytes)
            stream.seek(-length, os.SEEK_END)
            content = stream.read(length).decode("utf-8", errors="replace").strip()
    except OSError as error:
        content = f"<unavailable: {error}>"
    return _redact_failure_text(content, path_prefixes) or "<empty>"


def _write_failure_artifact(
    directory: Path,
    leaf: dict[str, object],
    argv: tuple[str, ...],
    return_code: int,
    stdout_tail: str,
    stderr_tail: str,
    path_prefixes: tuple[tuple[str, str], ...],
) -> str:
    command = [_redact_failure_text(argument, path_prefixes) for argument in argv]
    artifact = {
        "schema": "saturday.voice-runtime-build-failure.v1",
        "leaf": leaf,
        "exit_status": return_code,
        "command": command,
        "stdout_tail": stdout_tail,
        "stderr_tail": stderr_tail,
    }
    data = (json.dumps(artifact, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()
    if len(data) > 64 * 1024:
        raise ValueError("bounded build failure artifact exceeds its fixed limit")
    path = directory / f"voice-runtime-build-failure-{time.time_ns()}.json"
    descriptor = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
        0o600,
    )
    try:
        with os.fdopen(descriptor, "wb", closefd=True) as stream:
            stream.write(data)
    except BaseException:
        path.unlink(missing_ok=True)
        raise
    return path.name


def _run_leaf(
    workspace: Path,
    profile_path: Path,
    argv: tuple[str, ...],
    timeout_seconds: int,
    workspace_limit_bytes: int,
    child_limits: dict[str, tuple[int, int, int]],
    build_deadline: float,
    failure_artifact_dir: Path,
    leaf_identity: dict[str, object],
    private_path_prefixes: tuple[tuple[str, str], ...],
) -> tuple[int, int]:
    if not argv or argv[0] not in {str(_CLANG), str(_LIBTOOL)}:
        raise ValueError("build graph attempted an unapproved executable")
    if argv[0] == str(_CLANG) and (len(argv) < 2 or argv[1] not in {"-cc1", "-cc1as"}):
        raise ValueError("compiler invocation is not a fixed direct leaf")
    if argv[0] == str(_LIBTOOL) and (len(argv) < 2 or argv[1] != "-static"):
        raise ValueError("archive invocation is not fixed static libtool")
    remaining = build_deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("fixed source build exceeded its overall deadline")
    if _workspace_bytes(workspace) > workspace_limit_bytes:
        raise ValueError("build workspace already exceeds its fixed byte budget")
    stdout_path = workspace / "logs/leaf.stdout"
    stderr_path = workspace / "logs/leaf.stderr"
    for path in (stdout_path, stderr_path):
        if path.exists() or path.is_symlink():
            path.unlink()
    env = {
        "HOME": str(workspace / "tmp"),
        "TMPDIR": str(workspace / "tmp"),
        "LANG": "C",
        "LC_ALL": "C",
        "ZERO_AR_DATE": "1",
    }
    stdout_fd = os.open(stdout_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
    stderr_fd = os.open(stderr_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
    process = None
    try:
        process = subprocess.Popen(
            [str(_SANDBOX_EXEC), "-f", str(profile_path), "--", *argv],
            cwd=workspace,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=stdout_fd,
            stderr=stderr_fd,
            close_fds=True,
            preexec_fn=lambda: _limit_child_resources(child_limits),
        )
        peak_footprint = 0
        sample_count = 0
        return_code = None
        deadline = min(time.monotonic() + timeout_seconds, build_deadline)
        while return_code is None:
            if process.poll() is not None:
                return_code = process.returncode
                break
            footprint = _physical_footprint_bytes(process.pid)
            peak_footprint = max(peak_footprint, footprint)
            sample_count += 1
            if footprint > 8 * 1024 * 1024 * 1024:
                _stop_leaf(process)
                raise MemoryError("build leaf exceeded the sampled 8 GiB physical-footprint threshold")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return_code = _stop_leaf(process)
                raise TimeoutError(f"build leaf exceeded its deadline (exit {return_code})")
            try:
                return_code = process.wait(timeout=min(0.025, remaining))
            except subprocess.TimeoutExpired:
                continue
        if return_code != 0:
            stdout_tail = _failure_tail(stdout_path, 8_192, private_path_prefixes)
            stderr_tail = _failure_tail(stderr_path, 8_192, private_path_prefixes)
            try:
                artifact_name = _write_failure_artifact(
                    failure_artifact_dir,
                    leaf_identity,
                    argv,
                    return_code,
                    stdout_tail,
                    stderr_tail,
                    private_path_prefixes,
                )
            except (OSError, ValueError) as error:
                artifact_name = f"<write failed: {type(error).__name__}: {_redact_failure_text(str(error), private_path_prefixes)}>"
            raise RuntimeError(
                f"fixed build leaf exited with status {return_code}; leaf={json.dumps(leaf_identity, sort_keys=True)}; "
                f"failure_artifact={artifact_name}\nstdout tail:\n{stdout_tail}\nstderr tail:\n{stderr_tail}"
            )
    finally:
        os.close(stdout_fd)
        os.close(stderr_fd)
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
    if _workspace_bytes(workspace) > workspace_limit_bytes:
        raise ValueError("build leaf exceeded the fixed workspace byte budget")
    return peak_footprint, sample_count


def _verify_tool(path: Path, expected_sha256: str) -> None:
    parent_fd = _open_directory(path.parent)
    try:
        descriptor = os.open(
            path.name,
            os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0),
            dir_fd=parent_fd,
        )
        try:
            metadata = os.fstat(descriptor)
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_uid != 0
                or stat.S_IMODE(metadata.st_mode) & 0o022
                or _sha256_fd(descriptor, 256 * 1024 * 1024)[0] != expected_sha256
            ):
                raise ValueError(f"tool identity or trusted-root binding mismatch: {path}")
        finally:
            os.close(descriptor)
    finally:
        os.close(parent_fd)


def _candidate_for(source_id: str) -> SourceCandidate:
    candidates = {candidate.asset_id: candidate for candidate in load_candidates()}
    if source_id not in candidates:
        raise ValueError("source candidate is not pinned in the reviewed manifest")
    return candidates[source_id]
def _approved_source_seal(source_id: str) -> str:
    manifest = _read_json(_SOURCE_MANIFEST_PATH)
    runtimes = manifest.get("runtimes")
    if not isinstance(runtimes, list):
        raise ValueError("source manifest runtime roster is malformed")
    for item in runtimes:
        if isinstance(item, dict) and item.get("id") == source_id:
            seal = item.get("source_seal_sha256")
            if not isinstance(seal, str) or not re.fullmatch(r"[0-9a-f]{64}", seal):
                raise ValueError("approved source content seal is missing")
            return seal
    raise ValueError("source content seal is not pinned for this provider")


def _verify_source_stage(source_stage: Path) -> tuple[AcquiredSource, ...]:
    stage = Path(os.path.abspath(source_stage))
    stage_fd = _open_directory(stage)
    try:
        stage_metadata = os.fstat(stage_fd)
        if stat.S_IMODE(stage_metadata.st_mode) & 0o077 or stage_metadata.st_uid != os.getuid():
            raise ValueError("source stage must be owned by the current user and private mode-0700")
        result = []
        for candidate in load_candidates():
            archive_name = source_archive_filename(candidate)
            source_name = source_tree_directory_name(candidate)
            archive_fd = os.open(
                archive_name,
                os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0),
                dir_fd=stage_fd,
            )
            source_fd = os.open(
                source_name,
                os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0),
                dir_fd=stage_fd,
            )
            try:
                archive_metadata = os.fstat(archive_fd)
                if not stat.S_ISREG(archive_metadata.st_mode):
                    raise ValueError("reviewed source archive is not a regular file")
                archive_hash, archive_size = _sha256_fd(archive_fd, candidate.max_archive_bytes)
                if archive_hash != candidate.archive_sha256 or archive_size != candidate.archive_bytes:
                    raise ValueError("source archive differs from the reviewed digest")
                os.lseek(archive_fd, 0, os.SEEK_SET)
                if _git_tree_id_fd(
                    source_fd,
                    max_files=candidate.max_files,
                    max_file_bytes=candidate.max_file_bytes,
                    max_total_bytes=candidate.max_expanded_bytes,
                ) != candidate.tree:
                    raise ValueError("source extraction differs from the signed Git tree")
            finally:
                os.close(archive_fd)
                os.close(source_fd)
            source_seal = _source_content_seal(stage / source_name)
            if source_seal != _approved_source_seal(candidate.asset_id):
                raise ValueError("source archive stage differs from the frozen source content seal")
            result.append(
                AcquiredSource(
                    candidate,
                    stage / archive_name,
                    stage / source_name,
                    candidate.archive_url,
                    None,
                )
            )
    finally:
        os.close(stage_fd)
    sources = tuple(result)
    source_manifest = _read_json(_SOURCE_MANIFEST_PATH)
    recipe = _read_json(_RECIPE_PATH)
    manifest_runtimes = source_manifest.get("runtimes")
    recipe_sources = recipe.get("source_candidates")
    if not isinstance(manifest_runtimes, list) or not isinstance(recipe_sources, dict):
        raise ValueError("source lock and build recipe candidate lists are malformed")
    source_items = {
        item.get("id"): item
        for item in manifest_runtimes
        if isinstance(item, dict) and isinstance(item.get("id"), str)
    }
    recipe_items = recipe_sources
    target = recipe.get("target")
    compiler = recipe.get("compiler_recipe")
    options = recipe.get("cmake_equivalent_options")
    archives = compiler.get("provider_archives") if isinstance(compiler, dict) else None
    metal = recipe.get("embedded_metal")
    if (
        not isinstance(target, dict)
        or target.get("triple") != _TARGET
        or target.get("deployment_minimum") != _MINIMUM_OS
        or target.get("sdk_version") != "27.0"
        or not isinstance(compiler, dict)
        or compiler.get("cc1_target") != _TARGET
        or compiler.get("cc1_cpu") != 'none; matches GGML_NATIVE=false and GGML_CPU_ARM_ARCH=""'
        or compiler.get("release_flags") != ["-O3", "-DNDEBUG"]
        or compiler.get("cc1_gnuc_version") != _EXPECTED_GNUC_VERSION
        or compiler.get("cc1_stack_protector") != ["-stack-protector", "1"]
        or compiler.get("cxx_standard") != {"llama": "c++17", "whisper": "c++11", "ggml": "c++17"}
        or compiler.get("c_warning_flags") != [
            "-Wall", "-Wextra", "-Wpedantic", "-Wcast-qual", "-Wno-unused-function", "-Wshadow",
            "-Wstrict-prototypes", "-Wpointer-arith", "-Wmissing-prototypes", "-Werror=implicit-int",
            "-Werror=implicit-function-declaration", "-Wunreachable-code-break",
            "-Wunreachable-code-return", "-Wdouble-promotion"
        ]
        or compiler.get("cxx_warning_flags") != [
            "-Wall", "-Wextra", "-Wpedantic", "-Wcast-qual", "-Wno-unused-function",
            "-Wmissing-declarations", "-Wmissing-noreturn", "-Wmissing-prototypes",
            "-Wunreachable-code-break", "-Wunreachable-code-return", "-Wdouble-promotion", "-Wextra-semi"
        ]
        or compiler.get("ggml_cpu_compile_flags") != []
        or compiler.get("ggml_cpu_fp16_format") != _EXPECTED_GGML_CPU_FP16_FORMAT
        or compiler.get("whisper_cxx_warning_flags") != [
            "-Wall", "-Wextra", "-Wpedantic", "-Wcast-qual", "-Wno-unused-function",
            "-Wmissing-declarations", "-Wmissing-noreturn"
        ]
        or options != _EXPECTED_CMAKE_OPTIONS
        or not isinstance(archives, dict)
        or archives.get("llama_archive_member_target_order") != ["llama", "ggml", "ggml-metal", "ggml-cpu", "ggml-base"]
        or archives.get("whisper_archive_member_target_order") != ["whisper", "ggml", "ggml-metal", "ggml-cpu", "ggml-base"]
        or archives.get("provider_roles_are_separate") is not True
        or archives.get("ggml_source_snapshots_are_never_shared") is not True
        or archives.get("llama_worker") != "libSaturdayLlamaRuntime.a"
        or archives.get("whisper_worker") != "libSaturdayWhisperRuntime.a"
        or archives.get("native_worker_link_flags") != [
            "-framework", "Accelerate", "-framework", "Foundation",
            "-framework", "Metal", "-framework", "MetalKit", "-lc++"
        ]
        or not isinstance(metal, dict)
        or metal.get("kernel_sources") != list(_METAL_KERNELS)
        or metal.get("offline_metal_compiler") is not False
    ):
        raise ValueError("source/build recipe does not match the producer's fixed target graph")
    stage_fd = _open_directory(stage)
    try:
        for source in sources:
            manifest_item = source_items.get(source.candidate.asset_id)
            recipe_item = recipe_items.get(source.candidate.asset_id)
            if not isinstance(manifest_item, dict) or not isinstance(recipe_item, dict):
                raise ValueError("source candidate is absent from source lock or build recipe")
            source_header = manifest_item.get("header")
            recipe_header = {"path": recipe_item.get("header"), "sha256": recipe_item.get("header_sha256")}
            if not isinstance(source_header, dict) or source_header != recipe_header:
                raise ValueError("source lock and build recipe header seals disagree")
            source_fd = os.open(
                source_tree_directory_name(source.candidate),
                os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0),
                dir_fd=stage_fd,
            )
            try:
                with _open_relative_file(source_fd, str(source_header["path"]), 16 * 1024 * 1024) as header_file:
                    header_hash = hashlib.sha256(header_file.read()).hexdigest()
            finally:
                os.close(source_fd)
            if header_hash != source_header["sha256"]:
                raise ValueError("pinned public API header digest mismatch")
            if (
                manifest_item.get("commit") != source.candidate.commit
                or manifest_item.get("tree") != source.candidate.tree
                or manifest_item.get("archive_sha256") != source.candidate.archive_sha256
                or recipe_item.get("commit") != source.candidate.commit
                or recipe_item.get("tree") != source.candidate.tree
                or recipe_item.get("archive_sha256") != source.candidate.archive_sha256
            ):
                raise ValueError("source lock and build recipe source identities disagree")
    finally:
        os.close(stage_fd)
    return sources


def _copy_source_snapshot(source: AcquiredSource, workspace: Path) -> AcquiredSource:
    destination = workspace / "sources" / source.candidate.asset_id
    if destination.exists() or destination.is_symlink():
        raise ValueError("private source snapshot destination already exists")
    source_fd = _open_directory(source.source_root)
    stage_source_seal = _source_content_seal(source.source_root)
    try:
        if _git_tree_id_fd(
            source_fd,
            max_files=source.candidate.max_files,
            max_file_bytes=source.candidate.max_file_bytes,
            max_total_bytes=source.candidate.max_expanded_bytes,
        ) != source.candidate.tree:
            raise ValueError("source tree changed before immutable snapshot")
        expanded_bytes = 0

        def copy_directory(origin_fd: int, output: Path, is_root: bool = False) -> None:
            nonlocal expanded_bytes
            output.mkdir(mode=0o700)
            with os.scandir(origin_fd) as iterator:
                names = sorted((entry.name for entry in iterator), key=os.fsencode)
            for name in names:
                info = os.stat(name, dir_fd=origin_fd, follow_symlinks=False)
                if is_root and name == ".git":
                    if not stat.S_ISDIR(info.st_mode):
                        raise ValueError("root Git metadata must be a directory")
                    continue
                target_path = output / name
                if stat.S_ISDIR(info.st_mode):
                    child_fd = os.open(
                        name,
                        os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0),
                        dir_fd=origin_fd,
                    )
                    try:
                        held = os.fstat(child_fd)
                        if (held.st_dev, held.st_ino) != (info.st_dev, info.st_ino):
                            raise ValueError("source directory changed while snapshotting")
                        copy_directory(child_fd, target_path)
                    finally:
                        os.close(child_fd)
                    os.chmod(target_path, 0o555)
                elif stat.S_ISREG(info.st_mode):
                    if info.st_size > source.candidate.max_file_bytes:
                        raise ValueError("source snapshot file exceeds its fixed size cap")
                    expanded_bytes += info.st_size
                    if expanded_bytes > source.candidate.max_expanded_bytes:
                        raise ValueError("source snapshot exceeds its fixed expanded-size cap")
                    file_fd = os.open(
                        name,
                        os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0),
                        dir_fd=origin_fd,
                    )
                    try:
                        held = os.fstat(file_fd)
                        if (held.st_dev, held.st_ino) != (info.st_dev, info.st_ino) or held.st_size != info.st_size:
                            raise ValueError("source file changed while snapshotting")
                        body = bytearray()
                        while True:
                            chunk = os.read(file_fd, 1024 * 1024)
                            if not chunk:
                                break
                            body.extend(chunk)
                            if len(body) > source.candidate.max_file_bytes:
                                raise ValueError("source snapshot file exceeded its read cap")
                        if len(body) != info.st_size:
                            raise ValueError("source file changed during snapshot copy")
                    finally:
                        os.close(file_fd)
                    _write_file(target_path, bytes(body), 0o700 if info.st_mode & 0o111 else 0o600)
                    os.chmod(target_path, 0o555 if info.st_mode & 0o111 else 0o444)
                else:
                    raise ValueError("source snapshot contains a symlink or special file")
            os.chmod(output, 0o555)

        destination.parent.mkdir(mode=0o700, exist_ok=True)
        copy_directory(source_fd, destination, is_root=True)
        if _git_tree_id_fd(
            source_fd,
            max_files=source.candidate.max_files,
            max_file_bytes=source.candidate.max_file_bytes,
            max_total_bytes=source.candidate.max_expanded_bytes,
        ) != source.candidate.tree:
            raise ValueError("verified source root changed during snapshot copy")
    finally:
        os.close(source_fd)
    stage_after_seal = _source_content_seal(source.source_root)
    copied_fd = _open_directory(destination)
    try:
        copied_tree = _git_tree_id_fd(
            copied_fd,
            max_files=source.candidate.max_files,
            max_file_bytes=source.candidate.max_file_bytes,
            max_total_bytes=source.candidate.max_expanded_bytes,
        )
    finally:
        os.close(copied_fd)
    copied_source_seal = _source_content_seal(destination)
    if (
        copied_tree != source.candidate.tree
        or stage_after_seal != stage_source_seal
        or copied_source_seal != stage_source_seal
        or stage_source_seal != _approved_source_seal(source.candidate.asset_id)
    ):
        raise ValueError("private source snapshot differs from the verified source content seal")
    return AcquiredSource(
        source.candidate, source.archive_path, destination, source.effective_url, source.extraction
    )


def _metal_kernel_record(
    source: AcquiredSource,
    workspace: Path,
    kind: str,
) -> tuple[dict[str, object], bytes]:
    ggml_root = source.source_root / "ggml"
    inputs = _kernel_inputs(ggml_root, kind)
    source_bytes, common, dequantize, quantize, fa_common, fa_vec_common, ggml_common, metal_impl = inputs
    embedded = embedded_metal_source(
        kind, source_bytes, common=common, dequantize=dequantize, quantize=quantize,
        fa_common=fa_common, fa_vec_common=fa_vec_common, ggml_common=ggml_common,
        metal_impl=metal_impl,
    )
    input_names = (
        f"ggml/src/ggml-metal/kernels/{kind}.metal",
        "ggml/src/ggml-metal/kernels/common.h",
        "ggml/src/ggml-metal/kernels/dequantize.h",
        "ggml/src/ggml-metal/kernels/quantize.h",
        "ggml/src/ggml-metal/kernels/fa_common.metal",
        "ggml/src/ggml-metal/kernels/fa_vec_common.metal",
        "ggml/src/ggml-common.h",
        "ggml/src/ggml-metal/ggml-metal-impl.h",
    )
    blob_relative = f"generated/metal/{source.candidate.asset_id}/{kind}.metal"
    assembly_relative = f"generated/metal/{source.candidate.asset_id}/{kind}.s"
    object_relative = f"obj/{source.candidate.asset_id}/ggml-metal/generated-{kind}.o"
    assembly = embedded_metal_assembly(kind, blob_relative)
    return {
        "kind": kind,
        "inputs": [
            {"path": path, "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
            for path, data in zip(input_names, inputs)
        ],
        "embedded_source": {
            "path": blob_relative, "sha256": hashlib.sha256(embedded).hexdigest(), "bytes": len(embedded),
        },
        "assembly": {
            "path": assembly_relative, "sha256": hashlib.sha256(assembly).hexdigest(), "bytes": len(assembly),
            "argv": [
                str(_CLANG), "-cc1as", "-triple", _TARGET, "-filetype", "obj",
                "-o", str(workspace / object_relative), str(workspace / assembly_relative),
            ],
            "object": object_relative,
        },
    }, embedded


def _metal_resource_records(source: AcquiredSource, workspace: Path) -> list[dict[str, object]]:
    return [_metal_kernel_record(source, workspace, kind)[0] for kind in _METAL_KERNELS]


def _generate_metal_workspace(
    source: AcquiredSource,
    workspace: Path,
) -> tuple[tuple[str, ...], list[dict[str, object]]]:
    generated = workspace / "generated/metal" / source.candidate.asset_id
    generated.mkdir(parents=True, mode=0o700)
    assembly_paths = []
    records = []
    for kind in _METAL_KERNELS:
        record, embedded = _metal_kernel_record(source, workspace, kind)
        blob_relative = str(record["embedded_source"]["path"])
        asm_relative = str(record["assembly"]["path"])
        _write_file(workspace / blob_relative, embedded, 0o600)
        _write_file(workspace / asm_relative, embedded_metal_assembly(kind, blob_relative), 0o600)
        assembly_paths.append(asm_relative)
        records.append(record)
    return tuple(assembly_paths), records


def _unit_input_record(unit: TranslationUnit, source_root: Path, workspace: Path) -> dict[str, object]:
    path = (workspace if unit.unity_members else source_root).joinpath(*PurePosixPath(unit.relative_path).parts)
    sha, size = _sha256(path, 16 * 1024 * 1024)
    argv = _compile_argv(unit, source_root, workspace)
    return {
        "target": unit.target,
        "source": unit.relative_path,
        "source_sha256": sha,
        "source_bytes": size,
        "unity_members": [
            {
                "path": member,
                "sha256": _sha256(source_root.joinpath(*PurePosixPath(member).parts), 16 * 1024 * 1024)[0],
            }
            for member in unit.unity_members
        ],
        "language": unit.language,
        "defines": list(unit.defines),
        "include_directories": list(unit.include_dirs) + list(_generated_compile_includes_for_unit(unit, workspace)),
        "argv": list(argv),
        "output": unit.object_path,
    }


def _compile_argv(unit: TranslationUnit, source_root: Path, workspace: Path) -> tuple[str, ...]:
    args = [
        str(_CLANG), "-cc1", "-triple", _TARGET, "-fgnuc-version=" + _EXPECTED_GNUC_VERSION,
        "-isysroot", str(_canonical_sdk_path(_SDK)), "-resource-dir", str(_RESOURCE_DIR), "-target-sdk-version=27.0",
        "-mrelocation-model", "pic", "-pic-level", "2", "-mframe-pointer=non-leaf",
        "-O3", "-DNDEBUG", "-fno-strict-return", "-ffp-contract=on", "-fno-rounding-math",
        "-funwind-tables=1", "-fblocks", "-stack-protector", "1", "-fstack-check",
        "-mdarwin-stkchk-strong-link", "-emit-obj",
    ]
    if unit.language == "c++":
        standard = "c++11" if unit.target == "whisper" else "c++17"
        args.extend(["-std=" + standard, "-fcxx-exceptions", "-fexceptions"])
        for path in _CXX_INCLUDE_DIRS:
            args.extend(["-internal-isystem", str(_canonical_sdk_path(path))])
        warnings = (
            ("-Wall", "-Wextra", "-Wpedantic", "-Wcast-qual", "-Wno-unused-function",
             "-Wmissing-declarations", "-Wmissing-noreturn")
            if unit.target == "whisper"
            else ("-Wall", "-Wextra", "-Wpedantic", "-Wcast-qual", "-Wno-unused-function",
                  "-Wmissing-declarations", "-Wmissing-noreturn", "-Wmissing-prototypes",
                  "-Wunreachable-code-break", "-Wunreachable-code-return", "-Wdouble-promotion", "-Wextra-semi")
        )
        args.extend(warnings)
    elif unit.language == "objective-c":
        args.extend(["-fobjc-runtime=macosx-" + _MINIMUM_OS + ".0", "-fobjc-exceptions", "-fobjc-msgsend-selector-stubs", "-fexceptions"])
        for path in _C_INCLUDE_DIRS:
            args.extend(["-internal-isystem", str(_canonical_sdk_path(path))])
    else:
        args.extend(["-std=c11", "-Wall", "-Wextra", "-Wpedantic", "-Wcast-qual",
                     "-Wno-unused-function", "-Wshadow", "-Wstrict-prototypes", "-Wpointer-arith",
                     "-Wmissing-prototypes", "-Werror=implicit-int",
                     "-Werror=implicit-function-declaration", "-Wunreachable-code-break",
                     "-Wunreachable-code-return", "-Wdouble-promotion"])
        for path in _C_INCLUDE_DIRS:
            args.extend(["-internal-isystem", str(_canonical_sdk_path(path))])
    for path in _FRAMEWORK_DIRS:
        args.extend(["-internal-iframework", str(_canonical_sdk_path(path))])
    for define in unit.defines:
        args.extend(["-D", define])
    for path in unit.include_dirs + _generated_compile_includes_for_unit(unit, workspace):
        args.extend(["-I", path])
    source_path = (workspace if unit.unity_members else source_root).joinpath(*PurePosixPath(unit.relative_path).parts)
    args.extend(["-o", str(workspace / unit.object_path), "-x", unit.language, str(source_path)])
    return tuple(args)


def _component_units(source: AcquiredSource, workspace: Path) -> dict[str, tuple[TranslationUnit, ...]]:
    graph = _provider_sources(source)
    units: dict[str, tuple[TranslationUnit, ...]] = {}
    for target, paths in graph.items():
        entries: list[tuple[str, tuple[str, ...]]] = [(path, ()) for path in paths]
        if source.candidate.asset_id == "llama-cpp" and target == "llama":
            core = tuple("src/" + item for item in _LLAMA_CORE_SOURCES)
            if paths[:len(core)] != core:
                raise ValueError("pinned llama core source ordering differs from CMake")
            models = paths[len(core):]
            if not models or models != tuple(sorted(models)):
                raise ValueError("pinned llama model GLOB resolution is empty or unordered")
            entries = [(path, ()) for path in core]
            for batch, start in enumerate(range(0, len(models), 16)):
                members = models[start:start + 16]
                generated_relative = f"generated/llama/Unity/unity_{batch}_cxx.cxx"
                include_lines = []
                for member in members:
                    included = source.source_root / member
                    if '"' in str(included) or "\n" in str(included) or "\r" in str(included):
                        raise ValueError("unity source path cannot be represented safely")
                    include_lines.append(f'#include "{included}"\n'.encode())
                generated_bytes = b"".join(include_lines)
                generated_path = workspace / generated_relative
                generated_path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
                _write_file(generated_path, generated_bytes, 0o600)
                entries.append((generated_relative, members))
        target_units = []
        for relative, unity_members in entries:
            suffix = Path(relative).suffix
            language = {".c": "c", ".cpp": "c++", ".cxx": "c++", ".m": "objective-c"}.get(suffix)
            if language is None:
                raise ValueError("fixed build graph contains an unsupported source language")
            source_path = (workspace if unity_members else source.source_root).joinpath(*PurePosixPath(relative).parts)
            _sha256(source_path, 16 * 1024 * 1024)
            for member in unity_members:
                _sha256(source.source_root.joinpath(*PurePosixPath(member).parts), 16 * 1024 * 1024)
            safe_name = re.sub(r"[^A-Za-z0-9_.-]", "_", relative)
            target_units.append(TranslationUnit(
                provider=source.candidate.asset_id,
                target=target,
                relative_path=relative,
                language=language,
                defines=_target_defines(source.candidate.asset_id, target),
                include_dirs=_target_include_dirs(source, target),
                object_path=f"obj/{source.candidate.asset_id}/{target}/{safe_name}.o",
                unity_members=unity_members,
            ))
        units[target] = tuple(target_units)
    return units


def _tree_record(source: AcquiredSource, workspace: Path) -> dict[str, object]:
    generated = _header_generated_files(source)
    for relative, data in generated.items():
        destination = workspace / relative
        destination.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        _write_file(destination, data)
    graph = _component_units(source, workspace)
    metal_resources = _metal_resource_records(source, workspace)
    target_records = []
    member_order: list[str] = []
    for target, units in graph.items():
        records = [_unit_input_record(unit, source.source_root, workspace) for unit in units]
        target_records.append({"target": target, "translation_units": records})
        member_order.extend(unit.object_path for unit in units)
        if target == "ggml-metal":
            member_order.extend(str(record["assembly"]["object"]) for record in metal_resources)
    archive_name = "libSaturdayLlamaRuntime.a" if source.candidate.asset_id == "llama-cpp" else "libSaturdayWhisperRuntime.a"
    role = "llm" if source.candidate.asset_id == "llama-cpp" else "asr"
    archive_relative = f"output/{role}/archives/{archive_name}"
    archive_argv = _archive_workspace_objects(tuple(member_order), workspace, workspace / archive_relative)
    source_seal = _source_content_seal(source.source_root)
    if source_seal != _approved_source_seal(source.candidate.asset_id):
        raise ValueError("source plan differs from the frozen source content seal")
    return {
        "provider": source.candidate.asset_id,
        "source_seal": source_seal,
        "source_tree": source.candidate.tree,
        "archive_sha256": source.candidate.archive_sha256,
        "target_triple": _TARGET,
        "compiler_sha256": _sha256(_CLANG, 256 * 1024 * 1024)[0],
        "libtool_sha256": _sha256(_LIBTOOL, 256 * 1024 * 1024)[0],
        "sdk_settings_sha256": _sha256(_SDK / "SDKSettings.plist", 1024 * 1024)[0],
        "generated_headers": {
            name: {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
            for name, data in generated.items()
        },
        "targets": target_records,
        "llama_model_resolution": (
            {
                "glob": "src/models/*.cpp",
                "sorted_sources": [
                    member
                    for unit in graph["llama"][len(_LLAMA_CORE_SOURCES):]
                    for member in unit.unity_members
                ],
                "unity_units": [
                    {"source": unit.relative_path, "members": list(unit.unity_members)}
                    for unit in graph["llama"][len(_LLAMA_CORE_SOURCES):]
                ],
                "unity": "CMake UNITY_BUILD ON, BATCH mode, batch size 16; sorted GLOB sources are grouped consecutively"
            } if source.candidate.asset_id == "llama-cpp" else None
        ),
        "metal_resources": metal_resources,
        "archive": {
            "path": archive_relative,
            "member_order": member_order,
            "argv": list(archive_argv),
            "ordered_target_closure": list(graph),
        },
        "framework_link_order": ["Accelerate", "Foundation", "Metal", "MetalKit", "c++"],
        "framework_link_flags": ["-framework", "Accelerate", "-framework", "Foundation", "-framework", "Metal", "-framework", "MetalKit", "-lc++"],
    }


def _plan_record(sources: tuple[AcquiredSource, ...], workspace: Path) -> dict[str, object]:
    recipe_bytes = _RECIPE_PATH.read_bytes()
    source_manifest_bytes = _SOURCE_MANIFEST_PATH.read_bytes()
    sandbox_profile = _sandbox_profile(workspace, sources)
    return {
        "schema": "saturday.voice-runtime-build-plan.v1",
        "state": "source_graph_only_no_vendor_compilation",
        "recipe_sha256": hashlib.sha256(recipe_bytes).hexdigest(),
        "source_manifest_sha256": hashlib.sha256(source_manifest_bytes).hexdigest(),
        "runner_sha256": _runner_sha256(),
        "source_candidate_loader_sha256": _source_loader_sha256(),
        "compiler_input_seals": _compiler_input_seals(),
        "compiler_sdk_paths": {
            "cc1_sysroot": str(_canonical_sdk_path(_SDK)),
            "cxx_include_roots": [str(_canonical_sdk_path(path)) for path in _CXX_INCLUDE_DIRS],
            "c_and_objective_c_include_roots": [str(_canonical_sdk_path(path)) for path in _C_INCLUDE_DIRS],
            "framework_roots": [str(_canonical_sdk_path(path)) for path in _FRAMEWORK_DIRS],
        },
        "sandbox_profile": {
            "sha256": hashlib.sha256(sandbox_profile.encode("utf-8")).hexdigest(),
            "read_roots": [
                str(workspace.resolve(strict=True)), str(_SDK.resolve(strict=True)),
                str(_RESOURCE_DIR.resolve(strict=True)), "/System/Library", "/usr/lib",
                *(str(source.source_root.resolve(strict=True)) for source in sources),
            ],
            "read_directory_literals": ["/"],
            "read_directory_identity": {
                "device": os.stat("/").st_dev,
                "inode": os.stat("/").st_ino,
                "uid": os.stat("/").st_uid,
                "mode": stat.S_IMODE(os.stat("/").st_mode),
            },
            "read_files": [str(_CLANG), str(_LIBTOOL)],
            "exec_allowlist": [str(_CLANG), str(_LIBTOOL)],
            "fork": "denied",
            "network": "denied by default",
            "write_root": str(workspace.resolve(strict=True)),
        },
        "sdk_path": str(_SDK),
        "sdk_settings_sha256": _sha256(_SDK / "SDKSettings.plist", 1024 * 1024)[0],
        "clang_path": str(_CLANG),
        "clang_sha256": _sha256(_CLANG, 256 * 1024 * 1024)[0],
        "libtool_path": str(_LIBTOOL),
        "libtool_sha256": _sha256(_LIBTOOL, 256 * 1024 * 1024)[0],
        "sandbox_exec_path": str(_SANDBOX_EXEC),
        "sandbox_exec_sha256": _sha256(_SANDBOX_EXEC, 256 * 1024 * 1024)[0],
        "providers": [_tree_record(source, workspace) for source in sources],
    }


def _atomic_json(path: Path, value: dict[str, object]) -> None:
    data = (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()
    if len(data) > _MAX_PLAN_BYTES:
        raise ValueError("build plan exceeds its fixed metadata size limit")
    if path.exists() or path.is_symlink():
        raise ValueError("build plan output already exists")
    temp = path.with_name("." + path.name + ".tmp")
    _write_file(temp, data)
    os.rename(temp, path)


def _runner_sha256() -> str:
    return _sha256(Path(__file__).resolve(), 16 * 1024 * 1024)[0]


def _source_loader_sha256() -> str:
    return _sha256(_SOURCE_LOADER_PATH, 16 * 1024 * 1024)[0]



def _verify_compiler_input_seals(recipe: dict[str, object]) -> dict[str, dict[str, object]]:
    boundary = recipe.get("build_boundary")
    expected = boundary.get("compiler_input_seals") if isinstance(boundary, dict) else None
    actual = _compiler_input_seals()
    if expected != actual:
        raise ValueError("compiler input content or protected-root bindings differ from frozen recipe")
    return actual

def _verify_review_gate() -> dict[str, object]:
    recipe = _read_json(_RECIPE_PATH)
    manifest = _read_json(_SOURCE_MANIFEST_PATH)
    gates = recipe.get("independent_review_gates")
    if (
        recipe.get("state") != "independently_reviewed_and_main_approved"
        or manifest.get("state") != "independently_reviewed_and_main_approved"
        or not isinstance(gates, dict)
        or gates.get("vendor_compilation_authorized") is not True
        or gates.get("main_greenlight_required") is not False
    ):
        raise PermissionError("vendor compilation remains blocked pending independent review and Main greenlight")
    target = recipe.get("target")
    compiler = recipe.get("compiler_recipe")
    boundary = recipe.get("build_boundary")
    options = recipe.get("cmake_equivalent_options")
    if (
        not isinstance(target, dict)
        or target.get("triple") != _TARGET
        or target.get("deployment_minimum") != _MINIMUM_OS
        or target.get("sdk_version") != "27.0"
        or not isinstance(compiler, dict)
        or compiler.get("cc1_target") != _TARGET
        or compiler.get("cc1_cpu") != 'none; matches GGML_NATIVE=false and GGML_CPU_ARM_ARCH=""'
        or compiler.get("cc1_sdk_path") != str(_canonical_sdk_path(_SDK))
        or compiler.get("ggml_cpu_compile_flags") != []
        or compiler.get("ggml_cpu_fp16_format") != _EXPECTED_GGML_CPU_FP16_FORMAT
        or compiler.get("cc1_gnuc_version") != _EXPECTED_GNUC_VERSION
        or compiler.get("cc1_stack_protector") != ["-stack-protector", "1"]
        or not isinstance(boundary, dict)
        or boundary.get("startup_read_exception") != "exact file-read-data permission on literal / for root-directory enumeration only; no descendant data access; root-directory device, inode, owner and mode recorded in each source plan"
        or options != _EXPECTED_CMAKE_OPTIONS
    ):
        raise ValueError("build recipe target/CPU/SDK settings differ from the fixed compiler graph")
    source_loader = boundary.get("source_candidate_loader")
    if (
        not isinstance(source_loader, dict)
        or source_loader.get("path") != str(_SOURCE_LOADER_PATH)
        or source_loader.get("sha256") != _source_loader_sha256()
    ):
        raise ValueError("source candidate loader differs from the reviewed build input")
    tools = boundary.get("leaf_tools")
    if not isinstance(tools, list) or len(tools) != len(_EXPECTED_TOOL_SHA256) or any(not isinstance(item, dict) for item in tools):
        raise ValueError("build recipe toolchain pins are missing or malformed")
    recipe_tools = {item.get("path"): item.get("sha256") for item in tools}
    if recipe_tools != _EXPECTED_TOOL_SHA256:
        raise ValueError("build recipe tool identities differ from fixed host tool pins")
    sdk = boundary.get("sdk")
    if (
        not isinstance(sdk, dict)
        or sdk.get("path") != str(_SDK)
        or sdk.get("version") != "27.0"
        or sdk.get("settings_sha256") != _EXPECTED_SDK_SETTINGS_SHA256
    ):
        raise ValueError("build recipe SDK seal differs from the fixed SDK")
    limits = boundary.get("resource_limits")
    if (
        not isinstance(limits, dict)
        or limits.get("max_archive_bytes_each") != 64 * 1024 * 1024
        or limits.get("max_expanded_source_bytes_each") != 256 * 1024 * 1024
        or limits.get("max_build_workspace_bytes_each") != _MAX_BUILD_WORKSPACE_BYTES
        or limits.get("max_output_bytes_each") != _MAX_OUTPUT_BYTES
        or limits.get("max_cpu_processes") != 1
        or limits.get("max_leaf_rlimit_cpu_hard_seconds") != 300
        or limits.get("max_leaf_rlimit_fsize_hard_bytes") != 1024 * 1024 * 1024
        or limits.get("max_memory_bytes") != 8 * 1024 * 1024 * 1024
        or limits.get("memory_enforcement") != "sample Darwin proc_pid_rusage RUSAGE_INFO_V4 ri_phys_footprint every 25 ms; cancel direct leaf on observed value above 8 GiB; not a kernel hard memory ceiling"
        or limits.get("max_build_deadline_seconds") != _MAX_BUILD_DEADLINE_SECONDS
        or limits.get("max_leaf_deadline_seconds") != _MAX_LEAF_DEADLINE_SECONDS
        or limits.get("network") != "none"
        or limits.get("write_scope") != "fixed-capacity APFS image/generated,obj,output,logs,tmp; source snapshots are read-only"
    ):
        raise ValueError("build recipe resource limits differ from enforced limits")
    return recipe


def _archive_workspace_objects(objects: tuple[str, ...], workspace: Path, archive_path: Path) -> tuple[str, ...]:
    member_paths = tuple(str(workspace / object_path) for object_path in objects)
    if not member_paths:
        raise ValueError("static archive cannot be empty")
    argv = (str(_LIBTOOL), "-static", "-o", str(archive_path), *member_paths)
    return argv


def _headers_for(source: AcquiredSource) -> tuple[str, ...]:
    root = source.source_root
    candidates = ("include/llama.h", "include/whisper.h", "ggml/include/ggml.h", "ggml/include/ggml-alloc.h", "ggml/include/ggml-backend.h", "ggml/include/ggml-cpp.h", "ggml/include/ggml-opt.h", "ggml/include/gguf.h")
    return tuple(path for path in candidates if (root / path).is_file())


def _notices_for(source: AcquiredSource) -> tuple[tuple[str, bytes], ...]:
    manifest = _read_json(_SOURCE_MANIFEST_PATH)
    runtimes = manifest.get("runtimes")
    if not isinstance(runtimes, list):
        raise ValueError("runtime source manifest has no license roster")
    item = next((entry for entry in runtimes if isinstance(entry, dict) and entry.get("id") == source.candidate.asset_id), None)
    if not isinstance(item, dict) or not isinstance(item.get("license_files"), list):
        raise ValueError("source license closure is incomplete")
    notices = []
    for relative in item["license_files"]:
        if not isinstance(relative, str) or not _is_safe_relative(relative):
            raise ValueError("license notice path is unsafe")
        notices.append((relative, _read_source_file(source.source_root, relative)))
    if not notices:
        raise ValueError("source license closure is empty")
    return tuple(notices)


def _build_provider(
    source: AcquiredSource,
    workspace: Path,
    recipe: dict[str, object],
    profile_path: Path,
    child_limits: dict[str, tuple[int, int, int]],
    build_deadline: float,
    failure_artifact_dir: Path,
    private_path_prefixes: tuple[tuple[str, str], ...],
) -> dict[str, object]:
    before_tree = _source_tree_id(source)
    if before_tree != source.candidate.tree:
        raise ValueError("source tree changed after review")
    source_seal = _source_content_seal(source.source_root)
    if source_seal != _approved_source_seal(source.candidate.asset_id):
        raise ValueError("private source snapshot differs from the frozen source content seal")
    generated = _header_generated_files(source)
    for relative, data in generated.items():
        destination = workspace / relative
        destination.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        _write_file(destination, data)
    if source.candidate.asset_id not in {"llama-cpp", "whisper-cpp"}:
        raise ValueError("provider role has no reviewed static build graph")
    metal_assembly, metal_records = _generate_metal_workspace(source, workspace)
    units = _component_units(source, workspace)
    object_paths: list[str] = []
    source_inventory: list[dict[str, object]] = []
    observed_peak_footprint = 0
    observed_footprint_samples = 0
    metal_object_records = []
    for target, target_units in units.items():
        for unit in target_units:
            output = workspace / unit.object_path
            output.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
            argv = _compile_argv(unit, source.source_root, workspace)
            leaf_peak, leaf_samples = _run_leaf(
                workspace, profile_path, argv, _MAX_LEAF_DEADLINE_SECONDS, _MAX_BUILD_WORKSPACE_BYTES,
                child_limits, build_deadline, failure_artifact_dir,
                {"provider": source.candidate.asset_id, "target": target, "translation_unit": unit.relative_path,
                 "unity_members": list(unit.unity_members), "object": unit.object_path},
                private_path_prefixes,
            )
            observed_peak_footprint = max(observed_peak_footprint, leaf_peak)
            observed_footprint_samples += leaf_samples
            if output.is_symlink() or not output.is_file() or output.stat().st_size > _MAX_OBJECT_BYTES:
                raise ValueError("compiler output is missing or exceeds its per-object limit")
            object_paths.append(unit.object_path)
            source_inventory.append(_unit_input_record(unit, source.source_root, workspace))
        if target == "ggml-metal":
            for record in metal_records:
                output_relative = str(record["assembly"]["object"])
                output = workspace / output_relative
                output.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
                argv = tuple(str(value) for value in record["assembly"]["argv"])
                leaf_peak, leaf_samples = _run_leaf(
                    workspace, profile_path, argv, _MAX_LEAF_DEADLINE_SECONDS, _MAX_BUILD_WORKSPACE_BYTES,
                    child_limits, build_deadline, failure_artifact_dir,
                    {"provider": source.candidate.asset_id, "target": "ggml-metal",
                     "translation_unit": str(record["kind"]), "object": output_relative},
                    private_path_prefixes,
                )
                observed_peak_footprint = max(observed_peak_footprint, leaf_peak)
                observed_footprint_samples += leaf_samples
                if output.is_symlink() or not output.is_file() or output.stat().st_size > _MAX_OBJECT_BYTES:
                    raise ValueError("Metal resource object is missing or exceeds its per-object limit")
                object_paths.append(output_relative)
                metal_object_records.append({
                    "kind": record["kind"],
                    "path": output_relative,
                    "sha256": _sha256(output, _MAX_OBJECT_BYTES)[0],
                    "bytes": output.stat().st_size,
                    "argv": list(argv),
                })
    archive_name = "libSaturdayLlamaRuntime.a" if source.candidate.asset_id == "llama-cpp" else "libSaturdayWhisperRuntime.a"
    role = "llm" if source.candidate.asset_id == "llama-cpp" else "asr"
    archive_relative = f"output/{role}/archives/{archive_name}"
    archive_path = workspace / archive_relative
    archive_path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
    archive_argv = _archive_workspace_objects(tuple(object_paths), workspace, archive_path)
    leaf_peak, leaf_samples = _run_leaf(
        workspace, profile_path, archive_argv, _MAX_LEAF_DEADLINE_SECONDS, _MAX_BUILD_WORKSPACE_BYTES,
        child_limits, build_deadline, failure_artifact_dir,
        {"provider": source.candidate.asset_id, "target": "archive",
         "translation_unit": archive_name, "object_count": len(object_paths), "archive": archive_relative},
        private_path_prefixes,
    )
    observed_peak_footprint = max(observed_peak_footprint, leaf_peak)
    observed_footprint_samples += leaf_samples
    if archive_path.is_symlink() or not archive_path.is_file() or archive_path.stat().st_size > _MAX_OUTPUT_BYTES:
        raise ValueError("provider archive is missing or exceeds its output limit")
    output_root = workspace / "output" / role
    headers_root = output_root / "include"
    headers_root.mkdir(parents=True, mode=0o700)
    header_records = []
    for relative in _headers_for(source):
        destination = headers_root / Path(relative).name
        if destination.exists():
            raise ValueError("provider header name collision")
        _write_file(destination, _read_source_file(source.source_root, relative), 0o644)
        header_records.append({"path": destination.relative_to(output_root).as_posix(), "sha256": _sha256(destination)[0]})
    notice_root = output_root / "notices"
    notice_root.mkdir(mode=0o700)
    notice_records = []
    for relative, data in _notices_for(source):
        destination = notice_root / relative
        destination.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        _write_file(destination, data, 0o644)
        notice_records.append({"path": destination.relative_to(output_root).as_posix(), "sha256": hashlib.sha256(data).hexdigest()})
    if (
        _source_tree_id(source) != before_tree
        or _source_content_seal(source.source_root) != source_seal
    ):
        raise ValueError("vendor source tree changed during provider build")
    archive_sha, archive_bytes = _sha256(archive_path, 1024 * 1024 * 1024)

    return {
        "provider": source.candidate.asset_id,
        "role": role,
        "source_commit": source.candidate.commit,
        "source_archive_url": source.candidate.archive_url,
        "source_tree": before_tree,
        "source_seal": source_seal,
        "source_archive_sha256": source.candidate.archive_sha256,
        "source_archive_bytes": source.candidate.archive_bytes,
        "archive": archive_relative,
        "archive_sha256": archive_sha,
        "archive_bytes": archive_bytes,
        "archive_command": list(archive_argv),
        "archive_member_order": object_paths,
        "object_count": len(object_paths),
        "translation_units": source_inventory,
        "metal_resources": metal_records,
        "metal_objects": metal_object_records,
        "generated_headers": {
            name: {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
            for name, data in generated.items()
        },
        "headers": header_records,
        "notices": notice_records,
        "framework_dependencies": ["Accelerate", "Foundation", "Metal", "MetalKit"],
        "framework_link_order": ["Accelerate", "Foundation", "Metal", "MetalKit", "c++"],
        "framework_link_flags": ["-framework", "Accelerate", "-framework", "Foundation", "-framework", "Metal", "-framework", "MetalKit", "-lc++"],
        "metal_kernel_objects": len(metal_assembly),
        "promoted_resource_root": f"src-tauri/resources/voice-runtime-build/{role}",
        "sdk_tree_sha256": recipe["build_boundary"]["compiler_input_seals"]["macos_sdk"]["sha256"],
        "clang_resources_tree_sha256": recipe["build_boundary"]["compiler_input_seals"]["clang_resources"]["sha256"],
        "promoted_tree_sha256": _promoted_tree_sha256(output_root),
        "observed_peak_phys_footprint_bytes": observed_peak_footprint,
        "observed_phys_footprint_sample_count": observed_footprint_samples,
    }




def _run_hdiutil(arguments: tuple[str, ...], timeout: int) -> None:
    result = subprocess.run(
        ["/usr/bin/hdiutil", *arguments],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        close_fds=True,
        timeout=timeout,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError("bounded APFS workspace operation failed")


def _create_bounded_workspace(parent: Path) -> tuple[Path, Path, Path, tuple[int, int]]:
    run_root = Path(tempfile.mkdtemp(prefix="saturday-voice-build-", dir=parent))
    os.chmod(run_root, 0o700)
    metadata = os.lstat(run_root)
    mountpoint = run_root / "mounted-workspace"
    mountpoint.mkdir(mode=0o700)
    image = run_root / "workspace.sparseimage"
    attached = False
    try:
        _run_hdiutil(
            (
                "create", "-quiet", "-size", "8g", "-fs", "APFS", "-type", "SPARSE",
                "-volname", "SaturdayVoiceBuild", str(image),
            ),
            300,
        )
        os.chmod(image, 0o600)
        _run_hdiutil(("attach", "-quiet", "-nobrowse", "-owners", "off", "-mountpoint", str(mountpoint), str(image)), 120)
        attached = True
        root_stat = os.lstat(run_root)
        if (metadata.st_dev, metadata.st_ino) != (root_stat.st_dev, root_stat.st_ino):
            raise ValueError("private workspace owner directory changed while creating image")
        workspace_stat = os.statvfs(mountpoint)
        capacity = workspace_stat.f_blocks * workspace_stat.f_frsize
        if capacity <= 0 or capacity > _MAX_BUILD_WORKSPACE_BYTES:
            raise ValueError("APFS workspace capacity does not match the fixed aggregate limit")
        os.chmod(mountpoint, 0o700)
        return run_root, image, mountpoint, (metadata.st_dev, metadata.st_ino)
    except BaseException:
        try:
            mounted = os.stat(mountpoint).st_dev != metadata.st_dev
        except OSError:
            mounted = False
        if attached or mounted:
            try:
                _run_hdiutil(("detach", "-quiet", str(mountpoint)), 30)
            except (OSError, RuntimeError, subprocess.TimeoutExpired):
                raise RuntimeError(f"failed workspace remains for owner recovery: {run_root}")
        shutil.rmtree(run_root)
        raise


def _discard_failed_workspace(run_root: Path, mountpoint: Path, identity: tuple[int, int]) -> None:
    metadata = os.lstat(run_root)
    if (metadata.st_dev, metadata.st_ino) != identity or run_root.is_symlink():
        raise RuntimeError(f"failed workspace ownership changed; retained for recovery: {run_root}")
    _run_hdiutil(("detach", "-quiet", str(mountpoint)), 30)
    shutil.rmtree(run_root)

def _build_all(source_stage: Path, build_parent: Path) -> tuple[Path, dict[str, object]]:
    runner_sha256 = _runner_sha256()
    source_loader_sha256 = _source_loader_sha256()
    recipe = _verify_review_gate()
    if _source_loader_sha256() != source_loader_sha256:
        raise ValueError("source candidate loader changed during the review gate")
    boundary = recipe["build_boundary"]
    tools = {str(item["path"]): str(item["sha256"]) for item in boundary["leaf_tools"]}
    for path, expected in _EXPECTED_TOOL_SHA256.items():
        if tools[path] != expected:
            raise ValueError("build recipe tool pin changed after gate validation")
        _verify_tool(Path(path), expected)
    sdk_settings_hash, _ = _sha256(_SDK / "SDKSettings.plist", 1024 * 1024)
    if sdk_settings_hash != _EXPECTED_SDK_SETTINGS_SHA256:
        raise ValueError("active SDK settings digest differs from the frozen recipe")
    compiler_input_seals = _verify_compiler_input_seals(recipe)
    limits = boundary["resource_limits"]
    child_limits = _child_resource_limits()
    sources = _verify_source_stage(source_stage)
    parent = Path(os.path.abspath(build_parent))
    parent_fd = _open_directory(parent)
    try:
        parent_metadata = os.fstat(parent_fd)
        if (
            parent_metadata.st_uid != os.getuid()
            or stat.S_IMODE(parent_metadata.st_mode) & 0o077
            or parent.is_symlink()
        ):
            raise ValueError("build parent must be owned by the current user and private mode-0700")
    finally:
        os.close(parent_fd)
    build_deadline = time.monotonic() + int(limits["max_build_deadline_seconds"])
    run_root, image, workspace, identity = _create_bounded_workspace(parent)
    success = False
    try:
        os.chmod(workspace, 0o700)
        workspace_stat = os.statvfs(workspace)
        workspace_capacity_bytes = workspace_stat.f_blocks * workspace_stat.f_frsize
        if workspace_capacity_bytes <= 0 or workspace_capacity_bytes > _MAX_BUILD_WORKSPACE_BYTES:
            raise ValueError("mounted APFS workspace capacity differs from the fixed workspace budget")
        (workspace / "logs").mkdir(mode=0o700)
        for name in ("generated", "obj", "output", "tmp", "sources"):
            (workspace / name).mkdir(mode=0o700)
        build_sources = tuple(_copy_source_snapshot(source, workspace) for source in sources)
        profile_path = workspace / "sandbox.sb"
        profile = _sandbox_profile(workspace, build_sources)
        _write_file(profile_path, profile.encode("utf-8"), 0o600)
        private_path_prefixes = (
            (str(workspace), "<workspace>"),
            (str(Path(os.path.abspath(source_stage))), "<source-stage>"),
            (str(Path(__file__).resolve().parent.parent), "<project>"),
            (str(Path.home()), "<user-home>"),
        )
        providers = [
            _build_provider(
                source, workspace, recipe, profile_path, child_limits, build_deadline,
                parent, private_path_prefixes,
            )
            for source in build_sources
        ]
        if _verify_source_stage(source_stage) != sources:
            raise ValueError("original verified source inputs changed during isolated build")
        if _compiler_input_seals() != compiler_input_seals:
            raise ValueError("sealed compiler or SDK inputs changed during the build")
        if _source_loader_sha256() != source_loader_sha256:
            raise ValueError("source candidate loader changed during the isolated build")
        workspace_bytes = _workspace_bytes(workspace)
        if workspace_bytes > int(limits["max_build_workspace_bytes_each"]):
            raise ValueError("bounded APFS image contents exceed the fixed workspace budget")
        receipt = {
            "schema": "saturday.voice-runtime-artifacts.v1",
            "state": "built_unreviewed_not_ready_for_product_use",
            "recipe_sha256": hashlib.sha256(_RECIPE_PATH.read_bytes()).hexdigest(),
            "source_manifest_sha256": hashlib.sha256(_SOURCE_MANIFEST_PATH.read_bytes()).hexdigest(),
            "runner_sha256": runner_sha256,
            "sandbox_profile_sha256": hashlib.sha256(profile.encode("utf-8")).hexdigest(),
            "source_candidate_loader_sha256": source_loader_sha256,
            "compiler_input_seals": compiler_input_seals,
            "effective_limits": {
                "workspace_filesystem": "APFS fixed-capacity private sparse disk image",
                "workspace_capacity_bytes": workspace_capacity_bytes,
                "measured_workspace_bytes": workspace_bytes,
                "leaf_rlimit_cpu_seconds": child_limits["cpu_seconds"][1],
                "leaf_rlimit_fsize_bytes": child_limits["file_size_bytes"][1],
                "leaf_resource_limit_hard_ceiling": {
                    name: ("unlimited" if hard == resource.RLIM_INFINITY else hard)
                    for name, (_, _, hard) in child_limits.items()
                },
                "leaf_memory_cancellation": {
                    "measurement": "Darwin proc_pid_rusage RUSAGE_INFO_V4 ri_phys_footprint",
                    "interval_ms": 25,
                    "cancel_threshold_bytes": 8 * 1024 * 1024 * 1024,
                    "semantics": "sampled direct-child cancellation, not a kernel hard memory ceiling",
                },
                "build_deadline_seconds": int(limits["max_build_deadline_seconds"]),
                "leaf_deadline_seconds": int(limits["max_leaf_deadline_seconds"]),
            },
            "workspace_image": image.name,
            "providers": providers,
            "product_readiness": "requires independent artifact review and native purpose binding",
        }
        receipt_path = workspace / "output/build-artifact-manifest.json"
        receipt_path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        _atomic_json(receipt_path, receipt)
        if time.monotonic() > build_deadline:
            raise TimeoutError("fixed source build exceeded its overall deadline")
        _run_hdiutil(("detach", "-quiet", str(workspace)), 30)
        success = True
        return image, receipt
    finally:
        if not success:
            _discard_failed_workspace(run_root, workspace, identity)


def _main() -> int:
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-stage", type=Path, required=True)
    parser.add_argument("--build-parent", type=Path, required=True)
    parser.add_argument("--plan-only", action="store_true")
    parser.add_argument("--plan-output", type=Path)
    args = parser.parse_args()
    sources = _verify_source_stage(args.source_stage)
    if args.plan_only:
        if args.plan_output is None:
            parser.error("--plan-only requires --plan-output")
        parent = args.plan_output.parent.resolve(strict=True)
        if stat.S_IMODE(parent.stat().st_mode) & 0o077:
            raise ValueError("plan output parent must be private")
        scratch = Path(tempfile.mkdtemp(prefix="saturday-voice-plan-", dir=parent))
        os.chmod(scratch, 0o700)
        try:
            (scratch / "generated/llama").mkdir(parents=True, mode=0o700)
            for name in ("obj", "output", "logs", "tmp"):
                (scratch / name).mkdir(mode=0o700)
            plan = _plan_record(sources, scratch)
            _atomic_json(args.plan_output, plan)
        finally:
            shutil.rmtree(scratch)
        print(json.dumps({"plan": str(args.plan_output), "sha256": _sha256(args.plan_output)[0]}, sort_keys=True))
        return 0
    image, receipt = _build_all(args.source_stage, args.build_parent)
    print(json.dumps({"artifact_image": str(image), "providers": len(receipt["providers"]), "state": receipt["state"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(_main())
