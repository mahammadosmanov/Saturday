from __future__ import annotations

import hashlib
import os
import tempfile
import unittest
from pathlib import Path

from scripts.build_voice_runtimes import (
    _open_directory,
    _open_relative_file,
    _promoted_tree_sha256,
    _tree_content_seal,
    embedded_metal_assembly,
    embedded_metal_source,
)


class VoiceRuntimeBuildTests(unittest.TestCase):
    def test_descriptor_relative_file_open_rejects_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "include").mkdir()
            (root / "include/header.h").write_bytes(b"sealed header")
            (root / "outside").write_bytes(b"outside")
            (root / "include/linked.h").symlink_to("../outside")
            root_fd = _open_directory(root.resolve())
            try:
                with _open_relative_file(root_fd, "include/header.h", 64) as source:
                    self.assertEqual(source.read(), b"sealed header")
                with self.assertRaises(OSError):
                    _open_relative_file(root_fd, "include/linked.h", 64)
            finally:
                os.close(root_fd)

    def test_content_seal_changes_when_framework_input_changes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "Headers").mkdir()
            (root / "Headers/Metal.h").write_bytes(b"framework header")
            first, _ = _tree_content_seal(root)
            (root / "Headers/Metal.h").write_bytes(b"changed header")
            second, _ = _tree_content_seal(root)
            self.assertNotEqual(first, second)

    def test_promoted_tree_digest_binds_path_mode_size_and_raw_content_digest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifact = root / "archive.a"
            artifact.write_bytes(b"compiled bytes")
            artifact.chmod(0o644)
            record = (
                b"archive.a\0"
                + b"0644\0"
                + str(len(b"compiled bytes")).encode("ascii")
                + b"\0"
                + hashlib.sha256(b"compiled bytes").digest()
                + b"\n"
            )
            self.assertEqual(_promoted_tree_sha256(root), hashlib.sha256(record).hexdigest())

    def test_metal_source_inlines_only_declared_headers_and_removes_internal_includes(self) -> None:
        source = (
            b"#pragma once\n#include \"common.h\"\n#include \"dequantize.h\"\n"
            b"#include \"ggml-metal-impl.h\"\n__embed_ggml-common.h__\nfloat kernel;\n"
        )
        result = embedded_metal_source(
            "fa_f16",
            source,
            common=b"COMMON\n",
            dequantize=b"DEQUANTIZE\n",
            quantize=b"QUANTIZE\n",
            fa_common=b"FA_COMMON\n",
            fa_vec_common=b"FA_VEC_COMMON\n",
            ggml_common=b"GGML_COMMON\n",
            metal_impl=b"METAL_IMPL\n",
        )
        self.assertEqual(
            result,
            b"COMMON\nDEQUANTIZE\nMETAL_IMPL\nGGML_COMMON\nfloat kernel;\n",
        )

    def test_metal_source_does_not_inline_unreferenced_optional_headers(self) -> None:
        result = embedded_metal_source(
            "mul_mv",
            b"#include \"common.h\"\nfloat kernel;\n",
            common=b'#include "ggml-metal-impl.h"\nCOMMON\n',
            dequantize=b"DEQUANTIZE\n",
            quantize=b"QUANTIZE\n",
            fa_common=b"FA_COMMON\n",
            fa_vec_common=b"FA_VEC_COMMON\n",
            ggml_common=b"GGML_COMMON\n",
            metal_impl=b"METAL_IMPL\n",
        )
        self.assertEqual(result, b"METAL_IMPL\nCOMMON\nfloat kernel;\n")

    def test_metal_assembly_has_fixed_symbols_and_workspace_relative_blob(self) -> None:
        self.assertEqual(
            embedded_metal_assembly("gated_delta_net", "generated/metal-gated_delta_net.metal"),
            b".section __DATA,__ggml_metallib\n"
            b".globl _ggml_metallib_gated_delta_net_start\n"
            b"_ggml_metallib_gated_delta_net_start:\n"
            b'.incbin "generated/metal-gated_delta_net.metal"\n'
            b".globl _ggml_metallib_gated_delta_net_end\n"
            b"_ggml_metallib_gated_delta_net_end:\n",
        )

    def test_metal_assembly_rejects_paths_that_escape_workspace(self) -> None:
        for path in ("../metal.metal", "/tmp/metal.metal", "generated/a\"b.metal"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                embedded_metal_assembly("fa_f16", path)

    def test_metal_source_rejects_missing_duplicate_or_malformed_embedding_tokens(self) -> None:
        headers = {
            "common": b'',
            "dequantize": b'__embed_ggml-common.h__\n',
            "quantize": b'',
            "fa_common": b'',
            "fa_vec_common": b'',
            "ggml_common": b'GGML_COMMON\n',
            "metal_impl": b'METAL_IMPL\n',
        }
        missing_headers = {**headers, "dequantize": b""}
        with self.assertRaises(ValueError):
            embedded_metal_source(
                "kernel",
                b'#include "dequantize.h"\n#include "ggml-metal-impl.h"\n',
                **missing_headers,
            )
        with self.assertRaises(ValueError):
            embedded_metal_source(
                "kernel",
                b'__embed_ggml-common.h__\n__embed_ggml-common.h__\n#include "ggml-metal-impl.h"\n',
                **headers,
            )
        with self.assertRaises(ValueError):
            embedded_metal_source(
                "kernel",
                b'// #include "ggml-metal-impl.h"\n',
                **headers,
            )


if __name__ == "__main__":
    unittest.main()
