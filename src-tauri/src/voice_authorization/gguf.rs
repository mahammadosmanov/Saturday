use super::VoiceAuthorizationError;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::File;
use std::os::unix::fs::FileExt;

const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;
const MAX_METADATA_ENTRIES: u64 = 16_384;
const MAX_KEY_BYTES: u64 = 256;
const MAX_TEMPLATE_BYTES: u64 = 1024 * 1024;
const MAX_ARRAY_ELEMENTS: u64 = 200_000;
const HASH_CHUNK_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QwenMetadataFingerprint {
    pub(super) chat_template_sha256: [u8; 32],
    pub(super) tokenizer_metadata_sha256: [u8; 32],
    pub(super) architecture: String,
    pub(super) context_length: u32,
}

struct Reader<'a> {
    file: &'a File,
    file_bytes: u64,
    offset: u64,
    metadata_start: u64,
}

impl Reader<'_> {
    fn bytes(&mut self, output: &mut [u8]) -> Result<(), VoiceAuthorizationError> {
        let end = self
            .offset
            .checked_add(output.len() as u64)
            .ok_or(VoiceAuthorizationError::ModelInvalid)?;
        if end > self.file_bytes || end - self.metadata_start > MAX_METADATA_BYTES {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        let mut written = 0;
        while written < output.len() {
            let count = self
                .file
                .read_at(&mut output[written..], self.offset + written as u64)
                .map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
            if count == 0 {
                return Err(VoiceAuthorizationError::ModelInvalid);
            }
            written += count;
        }
        self.offset = end;
        Ok(())
    }

    fn u32(&mut self) -> Result<u32, VoiceAuthorizationError> {
        let mut bytes = [0; 4];
        self.bytes(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, VoiceAuthorizationError> {
        let mut bytes = [0; 8];
        self.bytes(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn stream(
        &mut self,
        mut remaining: u64,
        mut digest: Option<&mut Sha256>,
    ) -> Result<(), VoiceAuthorizationError> {
        let mut buffer = [0u8; HASH_CHUNK_BYTES];
        while remaining > 0 {
            let take = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
            self.bytes(&mut buffer[..take])?;
            if let Some(digest) = digest.as_deref_mut() {
                digest.update(&buffer[..take]);
            }
            remaining -= take as u64;
        }
        Ok(())
    }

    fn string(
        &mut self,
        mut digest: Option<&mut Sha256>,
        capture_limit: Option<u64>,
    ) -> Result<Option<Vec<u8>>, VoiceAuthorizationError> {
        let length_bytes = self.u64()?.to_le_bytes();
        let length = u64::from_le_bytes(length_bytes);
        if length == 0 || length > MAX_METADATA_BYTES {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        if let Some(digest) = digest.as_deref_mut() {
            digest.update(length_bytes);
        }
        let captured = if let Some(limit) = capture_limit {
            if length > limit {
                return Err(VoiceAuthorizationError::ModelInvalid);
            }
            let capacity =
                usize::try_from(length).map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
            let mut value = vec![0; capacity];
            self.bytes(&mut value)?;
            if let Some(digest) = digest {
                digest.update(&value);
            }
            Some(value)
        } else {
            self.stream(length, digest)?;
            None
        };
        Ok(captured)
    }

    fn scalar_bytes(
        &mut self,
        length: usize,
        digest: &mut Sha256,
    ) -> Result<[u8; 8], VoiceAuthorizationError> {
        if length > 8 {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        let mut bytes = [0u8; 8];
        self.bytes(&mut bytes[..length])?;
        digest.update(&bytes[..length]);
        Ok(bytes)
    }
}

/// Independently parses the bounded model metadata from a read-only FD after the caller has
/// verified the entire pinned payload SHA-256. It never executes model code or loads a backend.
pub(super) fn inspect_qwen3(
    file: &File,
    expected_file_bytes: u64,
) -> Result<QwenMetadataFingerprint, VoiceAuthorizationError> {
    let metadata = file
        .metadata()
        .map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
    if !metadata.is_file() || metadata.len() != expected_file_bytes || expected_file_bytes == 0 {
        return Err(VoiceAuthorizationError::ModelInvalid);
    }
    let mut reader = Reader {
        file,
        file_bytes: expected_file_bytes,
        offset: 0,
        metadata_start: 0,
    };
    let mut magic = [0u8; 4];
    reader.bytes(&mut magic)?;
    if &magic != b"GGUF" {
        return Err(VoiceAuthorizationError::ModelInvalid);
    }
    let version = reader.u32()?;
    let tensor_count = reader.u64()?;
    let metadata_count = reader.u64()?;
    if !(2..=3).contains(&version)
        || tensor_count == 0
        || tensor_count > 1_000_000
        || metadata_count == 0
        || metadata_count > MAX_METADATA_ENTRIES
    {
        return Err(VoiceAuthorizationError::ModelInvalid);
    }
    reader.metadata_start = reader.offset;

    let mut keys = BTreeSet::new();
    let mut tokenizer_digest = Sha256::new();
    tokenizer_digest.update(b"SATURDAY-QWEN3-TOKENIZER-METADATA-V1\0");
    let mut architecture = None;
    let mut context_length = None;
    let mut chat_template = None;
    let mut has_tokens = false;

    for _ in 0..metadata_count {
        let key_len = reader.u64()?;
        if key_len == 0 || key_len > MAX_KEY_BYTES {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        let key_len =
            usize::try_from(key_len).map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
        let mut key_bytes = vec![0; key_len];
        reader.bytes(&mut key_bytes)?;
        let key =
            std::str::from_utf8(&key_bytes).map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
        if !keys.insert(key.to_owned()) {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        let value_type = reader.u32()?;
        let selected = key.starts_with("tokenizer.")
            || key == "general.architecture"
            || key == "qwen3.context_length";
        let mut local_digest = Sha256::new();
        if selected {
            local_digest.update((key.len() as u64).to_le_bytes());
            local_digest.update(key.as_bytes());
            local_digest.update(value_type.to_le_bytes());
        }
        let digest = if selected {
            &mut local_digest
        } else {
            &mut tokenizer_digest
        };
        let mut parsed_numeric = None;
        let mut parsed_string = None;

        match value_type {
            0 | 1 | 7 => {
                let bytes = reader.scalar_bytes(1, digest)?;
                if value_type == 7 && bytes[0] > 1 {
                    return Err(VoiceAuthorizationError::ModelInvalid);
                }
                parsed_numeric = Some(u64::from(bytes[0]));
            }
            2..=3 => {
                let bytes = reader.scalar_bytes(2, digest)?;
                parsed_numeric = Some(u64::from(u16::from_le_bytes([bytes[0], bytes[1]])));
            }
            4..=6 => {
                let bytes = reader.scalar_bytes(4, digest)?;
                let bits = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                if value_type == 4 {
                    parsed_numeric = Some(u64::from(bits));
                }
            }
            10..=12 => {
                let bytes = reader.scalar_bytes(8, digest)?;
                if value_type == 10 {
                    parsed_numeric = Some(u64::from_le_bytes(bytes));
                }
            }
            8 => {
                let capture = (key == "general.architecture" || key == "tokenizer.chat_template")
                    .then_some(if key == "tokenizer.chat_template" {
                        MAX_TEMPLATE_BYTES
                    } else {
                        MAX_KEY_BYTES
                    });
                parsed_string = reader.string(Some(digest), capture)?;
            }
            9 => {
                let element_type = reader.u32()?;
                if element_type == 9 {
                    return Err(VoiceAuthorizationError::ModelInvalid);
                }
                let count_bytes = reader.u64()?.to_le_bytes();
                let count = u64::from_le_bytes(count_bytes);
                if count > MAX_ARRAY_ELEMENTS {
                    return Err(VoiceAuthorizationError::ModelInvalid);
                }
                digest.update(element_type.to_le_bytes());
                digest.update(count_bytes);
                if key == "tokenizer.ggml.tokens" {
                    if element_type != 8 || count == 0 {
                        return Err(VoiceAuthorizationError::ModelInvalid);
                    }
                    has_tokens = true;
                }
                for _ in 0..count {
                    match element_type {
                        0 | 1 | 7 => {
                            let bytes = reader.scalar_bytes(1, digest)?;
                            if element_type == 7 && bytes[0] > 1 {
                                return Err(VoiceAuthorizationError::ModelInvalid);
                            }
                        }
                        2..=3 => {
                            reader.scalar_bytes(2, digest)?;
                        }
                        4..=6 => {
                            reader.scalar_bytes(4, digest)?;
                        }
                        10..=12 => {
                            reader.scalar_bytes(8, digest)?;
                        }
                        8 => {
                            reader.string(Some(digest), None)?;
                        }
                        _ => return Err(VoiceAuthorizationError::ModelInvalid),
                    }
                }
            }
            _ => return Err(VoiceAuthorizationError::ModelInvalid),
        }
        if selected {
            tokenizer_digest.update(local_digest.finalize());
        }
        if key == "general.architecture" {
            let bytes = parsed_string.ok_or(VoiceAuthorizationError::ModelInvalid)?;
            let value =
                std::str::from_utf8(&bytes).map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
            if value != "qwen3" {
                return Err(VoiceAuthorizationError::ModelInvalid);
            }
            architecture = Some(value.to_owned());
        } else if key == "qwen3.context_length" {
            let value = parsed_numeric.ok_or(VoiceAuthorizationError::ModelInvalid)?;
            let value = u32::try_from(value).map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
            if value != 32_768 {
                return Err(VoiceAuthorizationError::ModelInvalid);
            }
            context_length = Some(value);
        } else if key == "tokenizer.chat_template" {
            if value_type != 8 {
                return Err(VoiceAuthorizationError::ModelInvalid);
            }
            let bytes = parsed_string.ok_or(VoiceAuthorizationError::ModelInvalid)?;
            let value =
                std::str::from_utf8(&bytes).map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
            if value.trim().is_empty() || value.contains('\0') {
                return Err(VoiceAuthorizationError::ModelInvalid);
            }
            chat_template = Some(Sha256::digest(&bytes).into());
        }
    }
    let architecture = architecture.ok_or(VoiceAuthorizationError::ModelInvalid)?;
    let context_length = context_length.ok_or(VoiceAuthorizationError::ModelInvalid)?;
    if !has_tokens {
        return Err(VoiceAuthorizationError::ModelInvalid);
    }
    let chat_template_sha256 = chat_template.ok_or(VoiceAuthorizationError::ModelInvalid)?;
    let tokenizer_metadata_sha256 = tokenizer_digest.finalize().into();
    Ok(QwenMetadataFingerprint {
        chat_template_sha256,
        tokenizer_metadata_sha256,
        architecture,
        context_length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    fn write_string(target: &mut Vec<u8>, value: &str) {
        target.extend_from_slice(&(value.len() as u64).to_le_bytes());
        target.extend_from_slice(value.as_bytes());
    }

    fn valid_gguf(architecture: &str, context: u32) -> Vec<u8> {
        let mut bytes = b"GGUF".to_vec();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&4u64.to_le_bytes());
        write_string(&mut bytes, "general.architecture");
        bytes.extend_from_slice(&8u32.to_le_bytes());
        write_string(&mut bytes, architecture);
        write_string(&mut bytes, "qwen3.context_length");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&context.to_le_bytes());
        write_string(&mut bytes, "tokenizer.chat_template");
        bytes.extend_from_slice(&8u32.to_le_bytes());
        write_string(&mut bytes, "{{ messages }}");
        write_string(&mut bytes, "tokenizer.ggml.tokens");
        bytes.extend_from_slice(&9u32.to_le_bytes());
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        write_string(&mut bytes, "token");
        bytes
    }

    fn temporary_model(bytes: &[u8]) -> (File, PathBuf) {
        let unique = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "saturday-qwen-metadata-{}-{unique}",
            std::process::id()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(file);
        (File::open(&path).unwrap(), path)
    }

    #[test]
    fn parses_bounded_architecture_template_and_tokenizer_metadata_from_the_real_fd() {
        let bytes = valid_gguf("qwen3", 32_768);
        let (file, path) = temporary_model(&bytes);
        let result = inspect_qwen3(&file, bytes.len() as u64).unwrap();
        assert_eq!(result.architecture, "qwen3");
        assert_eq!(result.context_length, 32_768);
        assert_ne!(result.chat_template_sha256, [0; 32]);
        assert_ne!(result.tokenizer_metadata_sha256, [0; 32]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_wrong_architecture_context_and_truncated_metadata() {
        for bytes in [
            valid_gguf("llama", 32_768),
            valid_gguf("qwen3", 16_384),
            valid_gguf("qwen3", 32_768)[..24].to_vec(),
        ] {
            let (file, path) = temporary_model(&bytes);
            assert!(matches!(
                inspect_qwen3(&file, bytes.len() as u64),
                Err(VoiceAuthorizationError::ModelInvalid)
            ));
            std::fs::remove_file(path).unwrap();
        }
    }
}
