use crate::voice::contract::{
    AssetCatalog, AssetDisclosure, AssetError, AssetInstallOutcome, AssetPurpose,
    AssetRemovalOutcome, AssetState, AssetStatus, AssetStatusCode, CancelToken,
    VoiceAssetAuthority, VoiceConsentPrompt,
};
use crate::voice_authorization::{
    sha256_hex, ApprovedModelHandle, ApprovedRuntimeClosure, TransferOutcome, TransferResult,
    VerifiedModelFile, VoiceAuthorizationError, VoiceAuthorizationService, VoiceTransferLease,
    WorkerRole,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

const PUBLIC_CATALOG_SHA256: &str =
    "ab13f6ff074ea7b9d05abbd86e5220aab615fcaad4ccaafdbda3378e3cafad65";
const PUBLIC_CATALOG_BYTES: &[u8] = include_bytes!("../../resources/voice-asset-catalog.json");
const MAX_MODEL_WEIGHTS_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ASSET_STORAGE_BYTES: u64 = 6 * 1024 * 1024 * 1024;
const MAX_GGUF_METADATA_BYTES: u64 = 16 * 1024 * 1024;
const MAX_GGUF_STRING_BYTES: u64 = 16 * 1024 * 1024;
const MAX_GGUF_TEMPLATE_BYTES: u64 = 1024 * 1024;
const MAX_GGUF_METADATA_ENTRIES: u64 = 16_384;
const MAX_GGUF_KEY_BYTES: u64 = 1024;
const MAX_GGUF_ARRAY_ELEMENTS: u64 = 1_000_000;
const MAX_TRANSFER_SECONDS: u64 = 15 * 60;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddedCatalog {
    schema: String,
    revision: u32,
    assets: Vec<FixedAsset>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedAsset {
    asset_id: String,
    purpose: AssetPurpose,
    filename: String,
    disclosure: AssetDisclosure,
}

impl FixedAsset {
    fn download_url(&self) -> String {
        format!(
            "{}/resolve/{}/{}",
            self.disclosure.source_origin, self.disclosure.source_revision, self.filename
        )
    }

    fn role(&self) -> WorkerRole {
        match self.purpose {
            AssetPurpose::AsrWeights => WorkerRole::Asr,
            AssetPurpose::LlmWeights => WorkerRole::Llm,
            _ => unreachable!("the embedded model catalog only accepts model purposes"),
        }
    }
}

fn fixed_catalog() -> Result<&'static EmbeddedCatalog, AssetError> {
    static CATALOG: LazyLock<Result<EmbeddedCatalog, AssetError>> =
        LazyLock::new(parse_embedded_catalog);
    match &*CATALOG {
        Ok(catalog) => Ok(catalog),
        Err(error) => Err(*error),
    }
}

fn fixed_assets() -> Result<&'static [FixedAsset], AssetError> {
    Ok(&fixed_catalog()?.assets)
}

fn asset_by_id(asset_id: &str) -> Result<&'static FixedAsset, AssetError> {
    fixed_assets()?
        .iter()
        .find(|asset| asset.asset_id == asset_id)
        .ok_or(AssetError::NotFound)
}

fn parse_embedded_catalog() -> Result<EmbeddedCatalog, AssetError> {
    if sha256_hex(PUBLIC_CATALOG_BYTES) != PUBLIC_CATALOG_SHA256 {
        return Err(AssetError::InvalidBinding);
    }
    let catalog: EmbeddedCatalog =
        serde_json::from_slice(PUBLIC_CATALOG_BYTES).map_err(|_| AssetError::InvalidBinding)?;
    if catalog.schema != "saturday.voice.asset-catalog.v1"
        || catalog.revision != 1
        || catalog.assets.len() != 2
    {
        return Err(AssetError::InvalidBinding);
    }

    let asr = &catalog.assets[0];
    let llm = &catalog.assets[1];
    if asr.asset_id != "whisper-base"
        || asr.purpose != AssetPurpose::AsrWeights
        || asr.filename != "ggml-base.bin"
        || asr.disclosure.candidate_name != "Multilingual Whisper base (ggml-base.bin)"
        || asr.disclosure.version != "5359861c739e955e79d9a303bcbc70fb988958b1"
        || asr.disclosure.source_origin != "https://huggingface.co/ggerganov/whisper.cpp"
        || asr.disclosure.source_revision != "5359861c739e955e79d9a303bcbc70fb988958b1"
        || asr.disclosure.purpose != AssetPurpose::AsrWeights
        || asr.disclosure.expected_bytes.0 != 147_951_465
        || asr.disclosure.digest_sha256
            != "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"
        || asr.disclosure.license_ids != ["MIT"]
        || asr.disclosure.max_staging_bytes.0 != 147_951_465
        || asr.disclosure.max_retained_bytes.0 != 147_951_465
        || asr.disclosure.max_disk_bytes.0 != 295_902_930
        || asr.disclosure.retention
            != crate::voice::contract::AssetRetention::AppManagedUntilExplicitRemoval
        || asr.disclosure.build.is_some()
        || llm.asset_id != "qwen3-4b-q4-k-m"
        || llm.purpose != AssetPurpose::LlmWeights
        || llm.filename != "Qwen3-4B-Q4_K_M.gguf"
        || llm.disclosure.candidate_name != "Qwen3-4B-Q4_K_M.gguf"
        || llm.disclosure.version != "bc640142c66e1fdd12af0bd68f40445458f3869b"
        || llm.disclosure.source_origin != "https://huggingface.co/Qwen/Qwen3-4B-GGUF"
        || llm.disclosure.source_revision != "bc640142c66e1fdd12af0bd68f40445458f3869b"
        || llm.disclosure.purpose != AssetPurpose::LlmWeights
        || llm.disclosure.expected_bytes.0 != 2_497_280_256
        || llm.disclosure.digest_sha256
            != "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5"
        || llm.disclosure.license_ids != ["Apache-2.0"]
        || llm.disclosure.max_staging_bytes.0 != 2_497_280_256
        || llm.disclosure.max_retained_bytes.0 != 2_497_280_256
        || llm.disclosure.max_disk_bytes.0 != 4_994_560_512
        || llm.disclosure.retention
            != crate::voice::contract::AssetRetention::AppManagedUntilExplicitRemoval
        || llm.disclosure.build.is_some()
    {
        return Err(AssetError::InvalidBinding);
    }
    let total = asr.disclosure.expected_bytes.0 + llm.disclosure.expected_bytes.0;
    if total > MAX_MODEL_WEIGHTS_BYTES
        || asr.disclosure.max_disk_bytes.0 > MAX_ASSET_STORAGE_BYTES
        || llm.disclosure.max_disk_bytes.0 > MAX_ASSET_STORAGE_BYTES
        || !valid_redirect_origins(&asr.disclosure)
        || !valid_redirect_origins(&llm.disclosure)
    {
        return Err(AssetError::InvalidBinding);
    }
    Ok(catalog)
}

fn valid_redirect_origins(disclosure: &AssetDisclosure) -> bool {
    let expected = [
        "https://huggingface.co",
        "https://cdn-lfs.huggingface.co",
        "https://cas-bridge.xethub.hf.co",
        "https://us.aws.cdn.hf.co",
    ];
    disclosure.redirect_origins.len() == expected.len()
        && disclosure
            .redirect_origins
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual == expected)
}

#[cfg(test)]
fn redirect_origin_allowed(disclosure: &AssetDisclosure, origin: &str) -> bool {
    disclosure
        .redirect_origins
        .iter()
        .any(|allowed| allowed == origin)
}

#[derive(Debug, PartialEq, Eq)]
struct GgufIdentity {
    architecture: String,
    context_length: u64,
    template_sha256: String,
    tokenizer_metadata_sha256: String,
}

struct GgufReader<'a> {
    file: &'a File,
    file_bytes: u64,
    offset: u64,
    metadata_start: u64,
}

impl GgufReader<'_> {
    fn read_exact(&mut self, output: &mut [u8]) -> Result<(), AssetError> {
        let end = self
            .offset
            .checked_add(output.len() as u64)
            .ok_or(AssetError::InvalidBinding)?;
        if end > self.file_bytes || end - self.metadata_start > MAX_GGUF_METADATA_BYTES {
            return Err(AssetError::InvalidBinding);
        }
        let mut read = 0;
        while read < output.len() {
            let amount = self
                .file
                .read_at(&mut output[read..], self.offset + read as u64)
                .map_err(|_| AssetError::StorageFailure)?;
            if amount == 0 {
                return Err(AssetError::InvalidBinding);
            }
            read += amount;
        }
        self.offset = end;
        Ok(())
    }

    fn read_u32(&mut self) -> Result<u32, AssetError> {
        let mut bytes = [0; 4];
        self.read_exact(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_u64(&mut self) -> Result<u64, AssetError> {
        let mut bytes = [0; 8];
        self.read_exact(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn read_hashed_bytes(
        &mut self,
        output: &mut [u8],
        digest: &mut Sha256,
    ) -> Result<(), AssetError> {
        self.read_exact(output)?;
        digest.update(output);
        Ok(())
    }

    fn read_hashed_string(
        &mut self,
        digest: &mut Sha256,
        capture_limit: Option<u64>,
    ) -> Result<Option<Vec<u8>>, AssetError> {
        let mut length_bytes = [0; 8];
        self.read_exact(&mut length_bytes)?;
        let length = u64::from_le_bytes(length_bytes);
        if length == 0 || length > MAX_GGUF_STRING_BYTES {
            return Err(AssetError::InvalidBinding);
        }
        digest.update(length_bytes);
        if let Some(limit) = capture_limit {
            if length > limit {
                return Err(AssetError::InvalidBinding);
            }
            let capacity = usize::try_from(length).map_err(|_| AssetError::InvalidBinding)?;
            let mut value = vec![0; capacity];
            self.read_hashed_bytes(&mut value, digest)?;
            return Ok(Some(value));
        }
        let mut remaining = length;
        let mut buffer = [0; 64 * 1024];
        while remaining > 0 {
            let amount = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| AssetError::InvalidBinding)?;
            self.read_hashed_bytes(&mut buffer[..amount], digest)?;
            remaining -= amount as u64;
        }
        Ok(None)
    }

    fn read_scalar(
        &mut self,
        value_type: u32,
        digest: &mut Sha256,
    ) -> Result<Option<u64>, AssetError> {
        let length = match value_type {
            0..=1 | 7 => 1,
            2..=3 => 2,
            4..=6 => 4,
            10..=12 => 8,
            _ => return Err(AssetError::InvalidBinding),
        };
        let mut bytes = [0; 8];
        self.read_hashed_bytes(&mut bytes[..length], digest)?;
        let numeric = match value_type {
            0 | 1 => Some(u64::from(bytes[0])),
            7 if bytes[0] <= 1 => Some(u64::from(bytes[0])),
            7 => return Err(AssetError::InvalidBinding),
            2 | 3 => Some(u64::from(u16::from_le_bytes([bytes[0], bytes[1]]))),
            4 => Some(u64::from(u32::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ]))),
            10 => Some(u64::from_le_bytes(bytes)),
            _ => None,
        };
        Ok(numeric)
    }
}

fn inspect_qwen_gguf(file: &File, expected_bytes: u64) -> Result<GgufIdentity, AssetError> {
    if expected_bytes == 0
        || file
            .metadata()
            .map_err(|_| AssetError::StorageFailure)?
            .len()
            != expected_bytes
    {
        return Err(AssetError::InvalidBinding);
    }
    let mut reader = GgufReader {
        file,
        file_bytes: expected_bytes,
        offset: 0,
        metadata_start: 0,
    };
    let mut magic = [0; 4];
    reader.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err(AssetError::InvalidBinding);
    }
    let version = reader.read_u32()?;
    let tensor_count = reader.read_u64()?;
    let metadata_count = reader.read_u64()?;
    if !(2..=3).contains(&version)
        || tensor_count == 0
        || tensor_count > 1_000_000
        || metadata_count == 0
        || metadata_count > MAX_GGUF_METADATA_ENTRIES
    {
        return Err(AssetError::InvalidBinding);
    }
    reader.metadata_start = reader.offset;
    let mut keys = std::collections::BTreeSet::new();
    let mut tokenizer_digest = Sha256::new();
    tokenizer_digest.update(b"SATURDAY-QWEN3-TOKENIZER-METADATA-V1\0");
    let mut architecture = None;
    let mut context_length = None;
    let mut template = None;
    let mut has_tokens = false;

    for _ in 0..metadata_count {
        let key_len = reader.read_u64()?;
        if key_len == 0 || key_len > MAX_GGUF_KEY_BYTES {
            return Err(AssetError::InvalidBinding);
        }
        let key_len = usize::try_from(key_len).map_err(|_| AssetError::InvalidBinding)?;
        let mut key_bytes = vec![0; key_len];
        reader.read_exact(&mut key_bytes)?;
        let key = std::str::from_utf8(&key_bytes).map_err(|_| AssetError::InvalidBinding)?;
        if !keys.insert(key.to_owned()) {
            return Err(AssetError::InvalidBinding);
        }
        let value_type = reader.read_u32()?;
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
        let mut numeric = None;
        let mut string = None;
        let mut token_array = false;
        match value_type {
            0..=7 | 10..=12 => {
                numeric = reader.read_scalar(value_type, digest)?;
            }
            8 => {
                let capture_limit = if key == "general.architecture" {
                    Some(MAX_GGUF_KEY_BYTES)
                } else if key == "tokenizer.chat_template" {
                    Some(MAX_GGUF_TEMPLATE_BYTES)
                } else {
                    None
                };
                string = reader.read_hashed_string(digest, capture_limit)?;
            }
            9 => {
                let element_type = reader.read_u32()?;
                if element_type == 9 {
                    return Err(AssetError::InvalidBinding);
                }
                let count_bytes = reader.read_u64()?.to_le_bytes();
                let count = u64::from_le_bytes(count_bytes);
                if count > MAX_GGUF_ARRAY_ELEMENTS {
                    return Err(AssetError::InvalidBinding);
                }
                digest.update(element_type.to_le_bytes());
                digest.update(count_bytes);
                if key == "tokenizer.ggml.tokens" {
                    if element_type != 8 || count == 0 {
                        return Err(AssetError::InvalidBinding);
                    }
                    token_array = true;
                }
                for _ in 0..count {
                    if element_type == 8 {
                        reader.read_hashed_string(digest, None)?;
                    } else {
                        reader.read_scalar(element_type, digest)?;
                    }
                }
            }
            _ => return Err(AssetError::InvalidBinding),
        }
        if selected {
            tokenizer_digest.update(local_digest.finalize());
        }
        if key == "general.architecture" {
            let value = string.ok_or(AssetError::InvalidBinding)?;
            let value = std::str::from_utf8(&value).map_err(|_| AssetError::InvalidBinding)?;
            if value != "qwen3" {
                return Err(AssetError::InvalidBinding);
            }
            architecture = Some(value.to_owned());
        } else if key == "qwen3.context_length" {
            let value = numeric.ok_or(AssetError::InvalidBinding)?;
            let value = u32::try_from(value).map_err(|_| AssetError::InvalidBinding)?;
            if value != 32_768 {
                return Err(AssetError::InvalidBinding);
            }
            context_length = Some(value);
        } else if key == "tokenizer.chat_template" {
            if value_type != 8 {
                return Err(AssetError::InvalidBinding);
            }
            let value = string.ok_or(AssetError::InvalidBinding)?;
            let text = std::str::from_utf8(&value).map_err(|_| AssetError::InvalidBinding)?;
            if text.trim().is_empty() || text.contains('\0') {
                return Err(AssetError::InvalidBinding);
            }
            template = Some(value);
        } else if key == "tokenizer.ggml.tokens" {
            if !token_array {
                return Err(AssetError::InvalidBinding);
            }
            has_tokens = true;
        }
    }
    let architecture = architecture.ok_or(AssetError::InvalidBinding)?;
    let context_length = context_length.ok_or(AssetError::InvalidBinding)?;
    let template = template.ok_or(AssetError::InvalidBinding)?;
    if !has_tokens {
        return Err(AssetError::InvalidBinding);
    }
    Ok(GgufIdentity {
        architecture,
        context_length: u64::from(context_length),
        template_sha256: sha256_hex(&template),
        tokenizer_metadata_sha256: format!("{:x}", tokenizer_digest.finalize()),
    })
}

trait AssetTransfer: Send + Sync {
    fn transfer(
        &self,
        asset: &FixedAsset,
        destination: &File,
        expected_bytes: u64,
        cancellation: &CancelToken,
        deadline: Instant,
    ) -> Result<u64, AssetError>;
}

struct NativeUrlSessionTransfer;

impl AssetTransfer for NativeUrlSessionTransfer {
    fn transfer(
        &self,
        asset: &FixedAsset,
        destination: &File,
        expected_bytes: u64,
        cancellation: &CancelToken,
        deadline: Instant,
    ) -> Result<u64, AssetError> {
        crate::platform::voice_assets::transfer_to_fd(
            &asset.download_url(),
            &asset.disclosure.redirect_origins,
            destination,
            expected_bytes,
            cancellation,
            deadline,
        )
    }
}

/// App-owned lifecycle for the two explicitly reviewed local model files.
///
/// Construction and inventory are inert: they do not prompt, transfer, inspect model contents,
/// initialize voice, or load a worker. Effects begin only from `request_install`.
pub struct RootAssetManager {
    app: tauri::AppHandle,
    app_data_dir: PathBuf,
    authorization: Arc<VoiceAuthorizationService>,
    prompt: Arc<dyn VoiceConsentPrompt>,
    authority: Arc<dyn VoiceAssetAuthority>,
    transfer: Arc<dyn AssetTransfer>,
    operation: Mutex<()>,
}

impl RootAssetManager {
    pub(crate) fn new(
        app: tauri::AppHandle,
        authorization: Arc<VoiceAuthorizationService>,
        prompt: Arc<dyn VoiceConsentPrompt>,
        authority: Arc<dyn VoiceAssetAuthority>,
    ) -> Result<Self, AssetError> {
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|_| AssetError::StorageFailure)?;
        Ok(Self {
            app,
            app_data_dir,
            authorization,
            prompt,
            authority,
            transfer: Arc::new(NativeUrlSessionTransfer),
            operation: Mutex::new(()),
        })
    }

    fn ensure_live_caller(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
    ) -> Result<(), AssetError> {
        if !caller.is_trusted() {
            return Err(AssetError::ApprovalRequired);
        }
        let window = self
            .app
            .get_webview_window(caller.window_label())
            .ok_or(AssetError::ApprovalRequired)?;
        caller
            .ensure_live(&window)
            .map_err(|_| AssetError::ApprovalRequired)
    }

    fn resolve_pending_after_user_cleanup(
        &self,
        asset: &'static FixedAsset,
        root: Option<&AssetRoot>,
    ) -> Result<(), AssetError> {
        let pending = match self.authorization.pending_transfers() {
            Ok(pending) => pending,
            Err(VoiceAuthorizationError::BindingUnfrozen) => return Ok(()),
            Err(error) => return Err(map_authorization_error(error)),
        };
        for transfer in pending
            .into_iter()
            .filter(|transfer| transfer.asset_id() == asset.asset_id)
        {
            if let Some(root) = root {
                remove_stage(root, asset, transfer.generation())?;
                root.staging
                    .as_ref()
                    .unwrap_or(&root.directory)
                    .sync_all()
                    .map_err(|_| AssetError::StorageFailure)?;
            }
            self.authorization
                .resolve_interrupted(&asset.asset_id, transfer.generation())
                .map_err(map_authorization_error)?;
        }
        Ok(())
    }

    fn status_for(&self, asset: &FixedAsset) -> AssetStatus {
        let mut status = AssetStatus {
            asset_id: asset.asset_id.clone(),
            purpose: asset.purpose,
            status: AssetState::Blocked,
            code: AssetStatusCode::NotReviewed,
            asset_disclosure: asset.disclosure.clone(),
        };

        let disclosure = match self.authorization.disclosure(&asset.asset_id) {
            Ok(disclosure) => disclosure,
            Err(error) => {
                let (state, code) = authorization_status(error);
                status.status = state;
                status.code = code;
                return status;
            }
        };
        if disclosure != asset.disclosure {
            status.status = AssetState::RecoveryRequired;
            status.code = AssetStatusCode::RecoveryRequired;
            return status;
        }

        match self.authorization.pending_transfers() {
            Ok(pending)
                if pending
                    .iter()
                    .any(|transfer| transfer.asset_id() == asset.asset_id) =>
            {
                status.status = AssetState::RecoveryRequired;
                status.code = AssetStatusCode::RecoveryRequired;
                return status;
            }
            Ok(_) | Err(VoiceAuthorizationError::BindingUnfrozen) => {}
            Err(error) => {
                let (state, code) = authorization_status(error);
                status.status = state;
                status.code = code;
                return status;
            }
        }

        let receipt = match self.authorization.active_receipt(&asset.asset_id) {
            Ok(receipt) => receipt,
            Err(error) => {
                let (state, code) = authorization_status(error);
                status.status = state;
                status.code = code;
                return status;
            }
        };
        if let Some(receipt) = receipt {
            use crate::voice_authorization::AuthorizedAssetState;
            match receipt.state() {
                AuthorizedAssetState::Disabled => {
                    status.status = AssetState::Disabled;
                    status.code = AssetStatusCode::Disabled;
                    return status;
                }
                AuthorizedAssetState::Revoked => {
                    status.code = AssetStatusCode::AssetMissing;
                    return status;
                }
                AuthorizedAssetState::Enabled => {}
            }
            if !receipt_matches_fixed_asset(&receipt, asset)
                || !payload_metadata_matches(&self.app_data_dir, asset, receipt.version())
            {
                status.status = AssetState::RecoveryRequired;
                status.code = AssetStatusCode::RecoveryRequired;
                return status;
            }
            if let Err(error) = self.authorization.approve_worker_closure(asset.role()) {
                (status.status, status.code) = authorization_status(error);
                return status;
            }
            status.status = AssetState::Ready;
            status.code = AssetStatusCode::Verified;
            return status;
        }

        if let Err(error) = self.authorization.verify_worker_bundle(asset.role()) {
            (status.status, status.code) = authorization_status(error);
            return status;
        }
        status.status = AssetState::AwaitingApproval;
        status.code = AssetStatusCode::ModelApprovalRequired;
        status
    }

    fn install_after_lease(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
        asset: &FixedAsset,
        lease: VoiceTransferLease,
        cancellation: &CancelToken,
        root: AssetRoot,
        store_lock: AssetStoreLock,
    ) -> Result<AssetStatus, AssetError> {
        let generation = lease.generation();
        if cancellation.is_cancelled() {
            self.finish_failed_transfer(
                lease,
                TransferOutcome::Cancelled,
                0,
                sha256_hex(&[]),
                cancellation,
            )?;
            return Err(AssetError::Cancelled);
        }
        if let Err(error) = self.ensure_live_caller(caller) {
            self.finish_failed_transfer(
                lease,
                TransferOutcome::Failed,
                0,
                sha256_hex(&[]),
                cancellation,
            )?;
            return Err(error);
        }
        let mut stage =
            match StagedAsset::create(&self.app_data_dir, asset, generation, root, store_lock) {
                Ok(stage) => stage,
                Err(error) => {
                    self.finish_failed_transfer(
                        lease,
                        TransferOutcome::Failed,
                        0,
                        sha256_hex(&[]),
                        cancellation,
                    )?;
                    return Err(error);
                }
            };
        let deadline = Instant::now() + Duration::from_secs(MAX_TRANSFER_SECONDS);
        let expected_bytes = asset.disclosure.expected_bytes.0;
        let actual_bytes =
            self.transfer
                .transfer(asset, stage.file()?, expected_bytes, cancellation, deadline);
        let actual_bytes = match actual_bytes {
            Ok(bytes) if bytes == expected_bytes => bytes,
            Ok(_) => {
                let partial_bytes = stage.file_len()?;
                let partial_digest = hash_file(stage.file()?)?;
                stage.cleanup()?;
                self.finish_failed_transfer(
                    lease,
                    TransferOutcome::Failed,
                    partial_bytes,
                    partial_digest,
                    cancellation,
                )?;
                return Err(AssetError::DigestMismatch);
            }
            Err(error) => {
                let partial_bytes = stage.file_len().unwrap_or(0);
                let partial_digest = hash_file(stage.file()?).unwrap_or_else(|_| sha256_hex(&[]));
                stage.cleanup()?;
                let outcome = if error == AssetError::Cancelled || cancellation.is_cancelled() {
                    TransferOutcome::Cancelled
                } else {
                    TransferOutcome::Failed
                };
                self.finish_failed_transfer(
                    lease,
                    outcome,
                    partial_bytes,
                    partial_digest,
                    cancellation,
                )?;
                return Err(error);
            }
        };

        let prepared = match stage.verify_and_authorize(
            asset,
            actual_bytes,
            caller,
            cancellation,
            &self.app,
            self.authority.as_ref(),
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                let partial_digest = hash_file(stage.file()?).unwrap_or_else(|_| sha256_hex(&[]));
                stage.cleanup()?;
                let outcome = if error == AssetError::Cancelled || cancellation.is_cancelled() {
                    TransferOutcome::Cancelled
                } else {
                    TransferOutcome::Failed
                };
                self.finish_failed_transfer(
                    lease,
                    outcome,
                    actual_bytes,
                    partial_digest,
                    cancellation,
                )?;
                return Err(error);
            }
        };

        if let Err(error) = self.ensure_live_caller(caller) {
            let digest = hash_file(stage.file()?).unwrap_or_else(|_| sha256_hex(&[]));
            stage.cleanup()?;
            let outcome = if cancellation.is_cancelled() {
                TransferOutcome::Cancelled
            } else {
                TransferOutcome::Failed
            };
            self.finish_failed_transfer(lease, outcome, actual_bytes, digest, cancellation)?;
            return Err(error);
        }
        let promotion_commit = match cancellation.begin_commit() {
            Some(commit) => commit,
            None => {
                stage.cleanup()?;
                self.finish_failed_transfer(
                    lease,
                    TransferOutcome::Cancelled,
                    actual_bytes,
                    prepared.sha256.clone(),
                    cancellation,
                )?;
                return Err(AssetError::Cancelled);
            }
        };
        let promotion_result = stage.promote(asset, &prepared);
        drop(promotion_commit);
        if let Err(error) = promotion_result {
            stage.cleanup()?;
            self.finish_failed_transfer(
                lease,
                TransferOutcome::Failed,
                actual_bytes,
                prepared.sha256.clone(),
                cancellation,
            )?;
            return Err(error);
        }
        if cancellation.is_cancelled() {
            remove_promoted_version(&self.app_data_dir, asset, &prepared.sha256)?;
            self.finish_failed_transfer(
                lease,
                TransferOutcome::Cancelled,
                actual_bytes,
                prepared.sha256.clone(),
                cancellation,
            )?;
            stage.disarm();
            return Err(AssetError::Cancelled);
        }

        let result = TransferResult {
            outcome: TransferOutcome::Promoted,
            bytes_received: actual_bytes,
            sha256: prepared.sha256.clone(),
            promoted_version: Some(prepared.sha256.clone()),
            template_fingerprint_sha256: prepared.template_sha256.clone(),
            tokenizer_metadata_sha256: prepared.tokenizer_metadata_sha256.clone(),
        };
        if let Err(error) = self.authority.finish_transfer(lease, result, cancellation) {
            if remove_promoted_version(&self.app_data_dir, asset, &prepared.sha256).is_err() {
                return Err(AssetError::CleanupFailed);
            }
            return Err(map_authorization_error(error));
        }
        stage.disarm();
        Ok(self.status_for(asset))
    }

    fn finish_failed_transfer(
        &self,
        lease: VoiceTransferLease,
        outcome: TransferOutcome,
        bytes_received: u64,
        sha256: String,
        cancellation: &CancelToken,
    ) -> Result<(), AssetError> {
        self.authority
            .finish_transfer(
                lease,
                TransferResult {
                    outcome,
                    bytes_received,
                    sha256,
                    promoted_version: None,
                    template_fingerprint_sha256: None,
                    tokenizer_metadata_sha256: None,
                },
                cancellation,
            )
            .map_err(map_authorization_error)
    }

    fn open_verified_model(
        &self,
        asset: &'static FixedAsset,
        purpose: AssetPurpose,
    ) -> Result<ApprovedModelHandle, AssetError> {
        if asset.purpose != purpose {
            return Err(AssetError::WrongPurpose);
        }
        let receipt = self
            .authorization
            .active_receipt(&asset.asset_id)
            .map_err(map_authorization_error)?
            .ok_or(AssetError::NotFound)?;
        if !receipt_matches_fixed_asset(&receipt, asset) {
            return Err(AssetError::InvalidBinding);
        }
        if receipt.state() != crate::voice_authorization::AuthorizedAssetState::Enabled {
            return Err(AssetError::Unavailable);
        }
        let file = open_payload_file(&self.app_data_dir, asset, receipt.version())?
            .ok_or(AssetError::NotFound)?;
        validate_readonly_model(&file, asset.disclosure.expected_bytes.0)?;
        let sha256 = hash_file(&file)?;
        if sha256 != asset.disclosure.digest_sha256 || sha256 != receipt.sha256() {
            return Err(AssetError::DigestMismatch);
        }
        let (template_sha256, tokenizer_metadata_sha256) = if purpose == AssetPurpose::LlmWeights {
            let identity = inspect_qwen_gguf(&file, asset.disclosure.expected_bytes.0)?;
            if Some(identity.template_sha256.as_str()) != receipt.template_fingerprint_sha256()
                || Some(identity.tokenizer_metadata_sha256.as_str())
                    != receipt.tokenizer_metadata_sha256()
            {
                return Err(AssetError::InvalidBinding);
            }
            (
                Some(parse_sha256(&identity.template_sha256)?),
                Some(parse_sha256(&identity.tokenizer_metadata_sha256)?),
            )
        } else {
            (None, None)
        };
        let metadata = file.metadata().map_err(|_| AssetError::StorageFailure)?;
        let verified = VerifiedModelFile::from_manager(
            file,
            asset.asset_id.clone(),
            purpose,
            receipt.version().to_owned(),
            asset.disclosure.expected_bytes.0,
            sha256,
            template_sha256,
            tokenizer_metadata_sha256,
            metadata.dev(),
            metadata.ino(),
        )
        .map_err(map_authorization_error)?;
        self.authorization
            .approve_model_file(verified)
            .map_err(map_authorization_error)
    }
}

impl crate::voice::contract::AssetManager for RootAssetManager {
    fn inventory(&self) -> Result<AssetCatalog, AssetError> {
        let assets = fixed_assets()?
            .iter()
            .map(|asset| self.status_for(asset))
            .collect();
        Ok(AssetCatalog {
            revision: crate::voice::contract::DecimalU64(1),
            assets,
        })
    }

    fn request_install(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
        asset_id: &str,
        cancellation: &CancelToken,
    ) -> Result<AssetInstallOutcome, AssetError> {
        let _operation = self.operation.lock().map_err(|_| AssetError::Unavailable)?;
        let asset = asset_by_id(asset_id)?;
        let status = self.status_for(asset);
        if status.status != AssetState::AwaitingApproval {
            if status.status == AssetState::Ready || status.status == AssetState::Disabled {
                return Ok(AssetInstallOutcome {
                    accepted: false,
                    status,
                });
            }
            return Err(match status.code {
                AssetStatusCode::RuntimeBuildRequired
                | AssetStatusCode::SourceReviewRequired
                | AssetStatusCode::NotReviewed => AssetError::ApprovalRequired,
                AssetStatusCode::OwnerTrustSetupRequired => AssetError::ApprovalRequired,
                AssetStatusCode::RecoveryRequired => AssetError::CleanupFailed,
                _ => AssetError::Unavailable,
            });
        }
        if cancellation.is_cancelled() {
            return Err(AssetError::Cancelled);
        }
        self.ensure_live_caller(caller)?;
        let disclosure = self
            .authorization
            .disclosure(asset_id)
            .map_err(map_authorization_error)?;
        if disclosure != asset.disclosure {
            return Err(AssetError::InvalidBinding);
        }
        let consent = self.prompt.confirm(caller, &disclosure, cancellation)?;
        self.ensure_live_caller(caller)?;
        let grant = self
            .authority
            .authorize_install(caller, asset_id, &disclosure, consent, cancellation)
            .map_err(map_authorization_error)?;
        if grant.asset_id() != asset.asset_id
            || grant.purpose() != asset.purpose
            || grant.expected_bytes() != asset.disclosure.expected_bytes.0
            || grant.expected_sha256() != asset.disclosure.digest_sha256
        {
            return Err(AssetError::InvalidBinding);
        }
        self.ensure_live_caller(caller)?;
        let install_root = match open_asset_root(&self.app_data_dir, true)? {
            Some(root) => root,
            None => return Err(AssetError::StorageFailure),
        };
        let store_lock = install_root.lock()?;
        let lease = self
            .authority
            .begin_transfer(grant, cancellation)
            .map_err(map_authorization_error)?;
        if lease.asset_id() != asset.asset_id
            || lease.purpose() != asset.purpose
            || lease.expected_bytes() != asset.disclosure.expected_bytes.0
            || lease.expected_sha256() != asset.disclosure.digest_sha256
        {
            self.finish_failed_transfer(
                lease,
                TransferOutcome::Failed,
                0,
                sha256_hex(&[]),
                cancellation,
            )?;
            return Err(AssetError::InvalidBinding);
        }
        let status =
            self.install_after_lease(caller, asset, lease, cancellation, install_root, store_lock)?;
        Ok(AssetInstallOutcome {
            accepted: status.status == AssetState::Ready,
            status,
        })
    }

    fn disable(&self, asset_id: &str) -> Result<AssetCatalog, AssetError> {
        let _operation = self.operation.lock().map_err(|_| AssetError::Unavailable)?;
        asset_by_id(asset_id)?;
        let root = open_asset_root(&self.app_data_dir, false)?;
        let _store_lock = root.as_ref().map(AssetRoot::lock).transpose()?;
        self.authorization
            .disable_asset(asset_id)
            .map_err(map_authorization_error)?;
        self.inventory()
    }

    fn delete(&self, asset_id: &str) -> Result<AssetRemovalOutcome, AssetError> {
        let _operation = self.operation.lock().map_err(|_| AssetError::Unavailable)?;
        let asset = asset_by_id(asset_id)?;
        let root = open_asset_root(&self.app_data_dir, false)?;
        let _store_lock = root.as_ref().map(AssetRoot::lock).transpose()?;
        self.resolve_pending_after_user_cleanup(asset, root.as_ref())?;
        self.authorization
            .delete_asset(asset_id)
            .map_err(map_authorization_error)?;
        remove_asset_versions(&self.app_data_dir, asset)?;
        let catalog = self.inventory()?;
        Ok(AssetRemovalOutcome {
            removed: true,
            catalog,
        })
    }

    fn rollback(&self, asset_id: &str, version_id: &str) -> Result<AssetCatalog, AssetError> {
        let _operation = self.operation.lock().map_err(|_| AssetError::Unavailable)?;
        let asset = asset_by_id(asset_id)?;
        if !is_sha256(version_id) || version_id != asset.disclosure.digest_sha256 {
            return Err(AssetError::NotFound);
        }
        let root = open_asset_root(&self.app_data_dir, false)?.ok_or(AssetError::NotFound)?;
        let _store_lock = root.lock()?;
        self.resolve_pending_after_user_cleanup(asset, Some(&root))?;
        let candidate = open_payload_file(&self.app_data_dir, asset, version_id)?
            .ok_or(AssetError::NotFound)?;
        validate_readonly_model(&candidate, asset.disclosure.expected_bytes.0)?;
        if hash_file(&candidate)? != asset.disclosure.digest_sha256 {
            return Err(AssetError::DigestMismatch);
        }
        let receipt = self
            .authorization
            .rollback_asset(asset_id, version_id)
            .map_err(map_authorization_error)?;
        if !receipt_matches_fixed_asset(&receipt, asset)
            || receipt.version() != version_id
            || receipt.state() != crate::voice_authorization::AuthorizedAssetState::Enabled
        {
            return Err(AssetError::InvalidBinding);
        }
        let handle = self.open_verified_model(asset, asset.purpose)?;
        drop(handle);
        self.inventory()
    }

    fn open_model(
        &self,
        asset_id: &str,
        purpose: AssetPurpose,
    ) -> Result<ApprovedModelHandle, AssetError> {
        let asset = asset_by_id(asset_id)?;
        self.open_verified_model(asset, purpose)
    }

    fn runtime_closure(&self, asset_id: &str) -> Result<ApprovedRuntimeClosure, AssetError> {
        let asset = asset_by_id(asset_id)?;
        let receipt = self
            .authorization
            .active_receipt(asset_id)
            .map_err(map_authorization_error)?
            .ok_or(AssetError::NotFound)?;
        if !receipt_matches_fixed_asset(&receipt, asset)
            || receipt.state() != crate::voice_authorization::AuthorizedAssetState::Enabled
        {
            return Err(AssetError::Unavailable);
        }
        self.authorization
            .approve_worker_closure(asset.role())
            .map_err(map_authorization_error)
    }
}

fn authorization_status(error: VoiceAuthorizationError) -> (AssetState, AssetStatusCode) {
    match error {
        VoiceAuthorizationError::BindingUnfrozen => {
            (AssetState::Blocked, AssetStatusCode::SourceReviewRequired)
        }
        VoiceAuthorizationError::BindingInvalid
        | VoiceAuthorizationError::RecoveryRequired
        | VoiceAuthorizationError::StorageInvalid
        | VoiceAuthorizationError::StorageFailure
        | VoiceAuthorizationError::ModelInvalid => (
            AssetState::RecoveryRequired,
            AssetStatusCode::RecoveryRequired,
        ),
        VoiceAuthorizationError::KeyUnavailable | VoiceAuthorizationError::OwnerAuthentication => (
            AssetState::Blocked,
            AssetStatusCode::OwnerTrustSetupRequired,
        ),
        VoiceAuthorizationError::WorkerMismatch => {
            (AssetState::Blocked, AssetStatusCode::RuntimeBuildRequired)
        }
        VoiceAuthorizationError::AssetMismatch => {
            (AssetState::Blocked, AssetStatusCode::SourceReviewRequired)
        }
        VoiceAuthorizationError::OwnerCancelled
        | VoiceAuthorizationError::GrantInvalid
        | VoiceAuthorizationError::GrantExpired
        | VoiceAuthorizationError::GrantConsumed
        | VoiceAuthorizationError::ProtocolFailure => {
            (AssetState::Blocked, AssetStatusCode::TransferNotGranted)
        }
    }
}

fn map_authorization_error(error: VoiceAuthorizationError) -> AssetError {
    match error {
        VoiceAuthorizationError::BindingUnfrozen | VoiceAuthorizationError::KeyUnavailable => {
            AssetError::ApprovalRequired
        }
        VoiceAuthorizationError::OwnerCancelled => AssetError::Cancelled,
        VoiceAuthorizationError::GrantExpired => AssetError::Timeout,
        VoiceAuthorizationError::GrantConsumed | VoiceAuthorizationError::RecoveryRequired => {
            AssetError::CleanupFailed
        }
        VoiceAuthorizationError::StorageInvalid | VoiceAuthorizationError::StorageFailure => {
            AssetError::StorageFailure
        }
        VoiceAuthorizationError::ModelInvalid
        | VoiceAuthorizationError::AssetMismatch
        | VoiceAuthorizationError::WorkerMismatch
        | VoiceAuthorizationError::BindingInvalid
        | VoiceAuthorizationError::GrantInvalid
        | VoiceAuthorizationError::OwnerAuthentication
        | VoiceAuthorizationError::ProtocolFailure => AssetError::InvalidBinding,
    }
}

fn receipt_matches_fixed_asset(
    receipt: &crate::voice_authorization::ActiveAssetReceipt,
    asset: &FixedAsset,
) -> bool {
    receipt.asset_id() == asset.asset_id
        && receipt.purpose()
            == match asset.purpose {
                AssetPurpose::AsrWeights => "asr_weights",
                AssetPurpose::LlmWeights => "llm_weights",
                AssetPurpose::Runtime => "runtime",
                AssetPurpose::PortableVoice => "portable_voice",
            }
        && receipt.bytes() == asset.disclosure.expected_bytes.0
        && receipt.sha256() == asset.disclosure.digest_sha256
        && receipt.version() == asset.disclosure.digest_sha256
}

struct AssetRoot {
    directory: File,
    models: Option<File>,
    staging: Option<File>,
}

impl AssetRoot {
    fn open(app_data_dir: &Path, create: bool) -> Result<Option<Self>, AssetError> {
        let Some(app_data) = open_absolute_directory(app_data_dir, create)? else {
            return Ok(None);
        };
        let Some(voice) = open_private_directory(&app_data, "voice", create)? else {
            return Ok(None);
        };
        let Some(directory) = open_private_directory(&voice, "assets", create)? else {
            return Ok(None);
        };
        let models = open_private_directory(&directory, "models", create)?;
        let staging = open_private_directory(&directory, "staging", create)?;
        Ok(Some(Self {
            directory,
            models,
            staging,
        }))
    }

    fn lock(&self) -> Result<AssetStoreLock, AssetError> {
        let file = open_file_at(
            &self.directory,
            "manager.lock",
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )?;
        validate_private_regular(&file, None)?;
        // SAFETY: `file` is a valid descriptor and `flock` only changes its advisory lock state.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(AssetError::Unavailable);
        }
        Ok(AssetStoreLock(file))
    }
}

struct AssetStoreLock(File);

impl Drop for AssetStoreLock {
    fn drop(&mut self) {
        // SAFETY: this descriptor is owned by this guard and remains open until this drop.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn open_asset_root(app_data_dir: &Path, create: bool) -> Result<Option<AssetRoot>, AssetError> {
    AssetRoot::open(app_data_dir, create)
}

fn open_absolute_directory(path: &Path, create: bool) -> Result<Option<File>, AssetError> {
    if !path.is_absolute() {
        return Err(AssetError::StorageFailure);
    }
    let mut current = File::open("/").map_err(|_| AssetError::StorageFailure)?;
    for component in path.components() {
        let name = match component {
            Component::RootDir => continue,
            Component::Normal(name) => name,
            _ => return Err(AssetError::StorageFailure),
        };
        let name = CString::new(name.as_bytes()).map_err(|_| AssetError::StorageFailure)?;
        let Some(next) = open_directory_at(&current, &name, create, false)? else {
            return Ok(None);
        };
        let metadata = next.metadata().map_err(|_| AssetError::StorageFailure)?;
        if !metadata.is_dir() || metadata.mode() & 0o022 != 0 {
            return Err(AssetError::StorageFailure);
        }
        current = next;
    }
    let metadata = current.metadata().map_err(|_| AssetError::StorageFailure)?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(AssetError::StorageFailure);
    }
    Ok(Some(current))
}

fn open_private_directory(
    parent: &File,
    name: &str,
    create: bool,
) -> Result<Option<File>, AssetError> {
    let name = CString::new(name).map_err(|_| AssetError::StorageFailure)?;
    let Some(directory) = open_directory_at(parent, &name, create, true)? else {
        return Ok(None);
    };
    let metadata = directory
        .metadata()
        .map_err(|_| AssetError::StorageFailure)?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.mode() & 0o700 != 0o700
    {
        return Err(AssetError::StorageFailure);
    }
    Ok(Some(directory))
}

fn open_directory_at(
    parent: &File,
    name: &CString,
    create: bool,
    _private: bool,
) -> Result<Option<File>, AssetError> {
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: `parent` is a live directory descriptor and `name` is NUL-terminated.
    let mut fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 && io::Error::last_os_error().kind() == io::ErrorKind::NotFound && create {
        // SAFETY: parent is a directory descriptor; mode restricts the new directory to its owner.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(AssetError::StorageFailure);
            }
        }
        // SAFETY: same checked descriptor and fixed component name as the initial open.
        fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    }
    if fd < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::NotFound && !create {
            return Ok(None);
        }
        return Err(AssetError::StorageFailure);
    }
    // SAFETY: successful `openat` returned a unique owned file descriptor.
    let directory = unsafe { File::from_raw_fd(fd) };
    let metadata = directory
        .metadata()
        .map_err(|_| AssetError::StorageFailure)?;
    if !metadata.is_dir() {
        return Err(AssetError::StorageFailure);
    }
    Ok(Some(directory))
}

fn open_file_at(parent: &File, name: &str, flags: i32, mode: u32) -> Result<File, AssetError> {
    let name = CString::new(name).map_err(|_| AssetError::StorageFailure)?;
    // SAFETY: `parent` is a valid directory descriptor and `name` is a fixed NUL-terminated name.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags, mode) };
    if fd < 0 {
        return Err(AssetError::StorageFailure);
    }
    // SAFETY: successful `openat` returned a unique owned file descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn validate_private_regular(file: &File, expected_bytes: Option<u64>) -> Result<(), AssetError> {
    let metadata = file.metadata().map_err(|_| AssetError::StorageFailure)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
        || metadata.mode() & 0o600 != 0o600
        || expected_bytes.is_some_and(|bytes| metadata.len() != bytes)
    {
        return Err(AssetError::StorageFailure);
    }
    Ok(())
}

fn validate_readonly_model(file: &File, expected_bytes: u64) -> Result<(), AssetError> {
    let metadata = file.metadata().map_err(|_| AssetError::StorageFailure)?;
    // SAFETY: `file` is a valid descriptor; F_GETFL does not mutate it.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.len() != expected_bytes
        || metadata.mode() & 0o777 != 0o400
        || flags < 0
        || flags & libc::O_ACCMODE != libc::O_RDONLY
    {
        return Err(AssetError::StorageFailure);
    }
    Ok(())
}

fn open_payload_file(
    app_data_dir: &Path,
    asset: &FixedAsset,
    version: &str,
) -> Result<Option<File>, AssetError> {
    if !is_sha256(version) || version != asset.disclosure.digest_sha256 {
        return Err(AssetError::InvalidBinding);
    }
    let Some(root) = open_asset_root(app_data_dir, false)? else {
        return Ok(None);
    };
    let Some(models) = root.models.as_ref() else {
        return Ok(None);
    };
    let role = role_directory(asset.purpose)?;
    let Some(role_dir) = open_private_directory(models, role, false)? else {
        return Ok(None);
    };
    let Some(asset_dir) = open_private_directory(&role_dir, &asset.asset_id, false)? else {
        return Ok(None);
    };
    let Some(versions) = open_private_directory(&asset_dir, "versions", false)? else {
        return Ok(None);
    };
    let Some(version_dir) = open_private_directory(&versions, version, false)? else {
        return Ok(None);
    };
    match open_file_at(
        &version_dir,
        "payload",
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        0,
    ) {
        Ok(file) => Ok(Some(file)),
        Err(AssetError::StorageFailure) if entry_is_missing(&version_dir, "payload")? => Ok(None),
        Err(error) => Err(error),
    }
}

fn role_directory(purpose: AssetPurpose) -> Result<&'static str, AssetError> {
    match purpose {
        AssetPurpose::AsrWeights => Ok("asr"),
        AssetPurpose::LlmWeights => Ok("llm"),
        _ => Err(AssetError::WrongPurpose),
    }
}

fn entry_is_missing(parent: &File, name: &str) -> Result<bool, AssetError> {
    let name = CString::new(name).map_err(|_| AssetError::StorageFailure)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: fstatat writes to the output buffer when successful and does not follow the
    // final symlink, so callers never inspect an attacker-selected target.
    let result = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Ok(false);
    }
    Ok(io::Error::last_os_error().kind() == io::ErrorKind::NotFound)
}

fn payload_metadata_matches(app_data_dir: &Path, asset: &FixedAsset, version: &str) -> bool {
    let Ok(Some(file)) = open_payload_file(app_data_dir, asset, version) else {
        return false;
    };
    validate_readonly_model(&file, asset.disclosure.expected_bytes.0).is_ok()
}

fn hash_file(file: &File) -> Result<String, AssetError> {
    let before = file.metadata().map_err(|_| AssetError::StorageFailure)?;
    if !before.is_file() {
        return Err(AssetError::StorageFailure);
    }
    let mut digest = Sha256::new();
    let mut buffer = [0; 128 * 1024];
    let mut offset = 0u64;
    while offset < before.len() {
        let buffer_len = u64::try_from(buffer.len()).map_err(|_| AssetError::StorageFailure)?;
        let amount = usize::try_from((before.len() - offset).min(buffer_len))
            .map_err(|_| AssetError::StorageFailure)?;
        let mut read = 0;
        while read < amount {
            let count = file
                .read_at(&mut buffer[read..amount], offset + read as u64)
                .map_err(|_| AssetError::StorageFailure)?;
            if count == 0 {
                return Err(AssetError::StorageFailure);
            }
            read += count;
        }
        digest.update(&buffer[..amount]);
        offset += amount as u64;
    }
    let after = file.metadata().map_err(|_| AssetError::StorageFailure)?;
    if before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.mode() != after.mode()
        || before.nlink() != after.nlink()
    {
        return Err(AssetError::StorageFailure);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn parse_sha256(value: &str) -> Result<[u8; 32], AssetError> {
    if !is_sha256(value) {
        return Err(AssetError::InvalidBinding);
    }
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| AssetError::InvalidBinding)?;
    }
    Ok(bytes)
}

struct PreparedAsset {
    sha256: String,
    template_sha256: Option<String>,
    tokenizer_metadata_sha256: Option<String>,
}

struct StagedAsset {
    root: AssetRoot,
    _lock: AssetStoreLock,
    name: String,
    directory: File,
    file: Option<File>,
    device: u64,
    inode: u64,
    stage_device: u64,
    stage_inode: u64,
    moved: bool,
    cleaned: bool,
}

impl StagedAsset {
    fn create(
        app_data_dir: &Path,
        asset: &FixedAsset,
        generation: u64,
        root: AssetRoot,
        lock: AssetStoreLock,
    ) -> Result<Self, AssetError> {
        if generation == 0 {
            return Err(AssetError::InvalidBinding);
        }
        ensure_storage_capacity(app_data_dir, &root, asset)?;
        let staging = root.staging.as_ref().ok_or(AssetError::StorageFailure)?;
        let name = format!("{}-{generation}", asset.asset_id);
        let directory = create_private_directory_exclusive(staging, &name)?;
        let file = open_file_at(
            &directory,
            "payload",
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )?;
        validate_private_regular(&file, Some(0))?;
        let stage_metadata = directory
            .metadata()
            .map_err(|_| AssetError::StorageFailure)?;
        let metadata = file.metadata().map_err(|_| AssetError::StorageFailure)?;
        Ok(Self {
            root,
            _lock: lock,
            name,
            directory,
            file: Some(file),
            device: metadata.dev(),
            inode: metadata.ino(),
            stage_device: stage_metadata.dev(),
            stage_inode: stage_metadata.ino(),
            moved: false,
            cleaned: false,
        })
    }

    fn file(&self) -> Result<&File, AssetError> {
        self.file.as_ref().ok_or(AssetError::Unavailable)
    }

    fn file_len(&self) -> Result<u64, AssetError> {
        Ok(self
            .file()?
            .metadata()
            .map_err(|_| AssetError::StorageFailure)?
            .len())
    }

    fn verify_and_authorize(
        &mut self,
        asset: &FixedAsset,
        bytes_received: u64,
        caller: &crate::voice::ipc::NativeCallerContext,
        cancellation: &CancelToken,
        app: &tauri::AppHandle,
        authority: &dyn VoiceAssetAuthority,
    ) -> Result<PreparedAsset, AssetError> {
        if cancellation.is_cancelled() || bytes_received != asset.disclosure.expected_bytes.0 {
            return Err(AssetError::Cancelled);
        }
        let writer = self.file()?;
        writer.sync_all().map_err(|_| AssetError::StorageFailure)?;
        validate_private_regular(writer, Some(bytes_received))?;
        let sha256 = hash_file(writer)?;
        if sha256 != asset.disclosure.digest_sha256 {
            return Err(AssetError::DigestMismatch);
        }
        let identity = if asset.purpose == AssetPurpose::LlmWeights {
            Some(inspect_qwen_gguf(writer, bytes_received)?)
        } else {
            None
        };
        let template_sha256 = identity
            .as_ref()
            .map(|identity| identity.template_sha256.clone());
        let tokenizer_metadata_sha256 = identity
            .as_ref()
            .map(|identity| identity.tokenizer_metadata_sha256.clone());

        // Remove every writable descriptor before purpose authorization. The authority receives
        // an O_RDONLY descriptor to the immutable staged inode, never a caller-supplied path.
        // SAFETY: `self.file` is an owned regular file created with O_EXCL beneath the private
        // staging directory and its identity was recorded immediately after creation.
        if unsafe { libc::fchmod(writer.as_raw_fd(), 0o400) } != 0 {
            return Err(AssetError::StorageFailure);
        }
        writer.sync_all().map_err(|_| AssetError::StorageFailure)?;
        drop(self.file.take());
        let file = open_file_at(
            &self.directory,
            "payload",
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )?;
        validate_readonly_model(&file, bytes_received)?;
        if hash_file(&file)? != sha256 {
            return Err(AssetError::DigestMismatch);
        }
        let metadata = file.metadata().map_err(|_| AssetError::StorageFailure)?;
        if metadata.dev() != self.device || metadata.ino() != self.inode {
            return Err(AssetError::StorageFailure);
        }
        self.file = Some(file.try_clone().map_err(|_| AssetError::StorageFailure)?);
        let candidate = VerifiedModelFile::from_manager(
            file,
            asset.asset_id.clone(),
            asset.purpose,
            sha256.clone(),
            bytes_received,
            sha256.clone(),
            template_sha256.as_deref().map(parse_sha256).transpose()?,
            tokenizer_metadata_sha256
                .as_deref()
                .map(parse_sha256)
                .transpose()?,
            metadata.dev(),
            metadata.ino(),
        )
        .map_err(map_authorization_error)?;
        if cancellation.is_cancelled() {
            return Err(AssetError::Cancelled);
        }
        ensure_live_app_caller(app, caller)?;
        authority
            .authorize_model_purpose(caller, &candidate, cancellation)
            .map_err(map_authorization_error)?;
        if cancellation.is_cancelled() {
            return Err(AssetError::Cancelled);
        }
        ensure_live_app_caller(app, caller)?;
        Ok(PreparedAsset {
            sha256,
            template_sha256,
            tokenizer_metadata_sha256,
        })
    }

    fn promote(&mut self, asset: &FixedAsset, prepared: &PreparedAsset) -> Result<(), AssetError> {
        if self.file.is_none()
            || prepared.sha256 != asset.disclosure.digest_sha256
            || !is_sha256(&prepared.sha256)
        {
            return Err(AssetError::InvalidBinding);
        }
        let versions =
            open_model_versions_dir(&self.root, asset, true)?.ok_or(AssetError::StorageFailure)?;
        if !entry_is_missing(&versions, &prepared.sha256)? {
            return Err(AssetError::CleanupFailed);
        }
        let staging = self
            .root
            .staging
            .as_ref()
            .ok_or(AssetError::StorageFailure)?;
        let stage_name =
            CString::new(self.name.as_str()).map_err(|_| AssetError::StorageFailure)?;
        let final_name =
            CString::new(prepared.sha256.as_str()).map_err(|_| AssetError::StorageFailure)?;
        // SAFETY: both directory descriptors are owned no-follow directories under the app-owned
        // root; source and target names are internally generated, fixed-format components.
        if unsafe {
            libc::renameat(
                staging.as_raw_fd(),
                stage_name.as_ptr(),
                versions.as_raw_fd(),
                final_name.as_ptr(),
            )
        } != 0
        {
            return Err(AssetError::StorageFailure);
        }
        versions
            .sync_all()
            .map_err(|_| AssetError::StorageFailure)?;
        staging.sync_all().map_err(|_| AssetError::StorageFailure)?;
        self.moved = true;
        Ok(())
    }

    fn cleanup(&mut self) -> Result<(), AssetError> {
        if self.cleaned || self.moved {
            return Ok(());
        }
        drop(self.file.take());
        let staging = self
            .root
            .staging
            .as_ref()
            .ok_or(AssetError::StorageFailure)?;
        remove_stage_named(
            staging,
            &self.name,
            Some((self.stage_device, self.stage_inode)),
            Some((self.device, self.inode)),
        )?;
        staging.sync_all().map_err(|_| AssetError::StorageFailure)?;
        self.cleaned = true;
        Ok(())
    }

    fn disarm(&mut self) {
        self.cleaned = true;
    }
}

impl Drop for StagedAsset {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn ensure_live_app_caller(
    app: &tauri::AppHandle,
    caller: &crate::voice::ipc::NativeCallerContext,
) -> Result<(), AssetError> {
    if !caller.is_trusted() {
        return Err(AssetError::ApprovalRequired);
    }
    let window = app
        .get_webview_window(caller.window_label())
        .ok_or(AssetError::ApprovalRequired)?;
    caller
        .ensure_live(&window)
        .map_err(|_| AssetError::ApprovalRequired)
}

fn create_private_directory_exclusive(parent: &File, name: &str) -> Result<File, AssetError> {
    let name = CString::new(name).map_err(|_| AssetError::StorageFailure)?;
    // SAFETY: parent is an app-owned directory descriptor and the fixed generated name cannot
    // traverse; mkdirat does not follow an existing symlink.
    if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
        return Err(AssetError::StorageFailure);
    }
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: successful mkdirat created this single directory component below parent.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(AssetError::StorageFailure);
    }
    // SAFETY: successful openat returned a unique owned descriptor.
    let directory = unsafe { File::from_raw_fd(fd) };
    let metadata = directory
        .metadata()
        .map_err(|_| AssetError::StorageFailure)?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.mode() & 0o700 != 0o700
    {
        return Err(AssetError::StorageFailure);
    }
    Ok(directory)
}

fn open_model_versions_dir(
    root: &AssetRoot,
    asset: &FixedAsset,
    create: bool,
) -> Result<Option<File>, AssetError> {
    let Some(models) = root.models.as_ref() else {
        return Ok(None);
    };
    let Some(role) = open_private_directory(models, role_directory(asset.purpose)?, create)? else {
        return Ok(None);
    };
    let Some(asset_dir) = open_private_directory(&role, &asset.asset_id, create)? else {
        return Ok(None);
    };
    open_private_directory(&asset_dir, "versions", create)
}

fn ensure_storage_capacity(
    app_data_dir: &Path,
    root: &AssetRoot,
    asset: &FixedAsset,
) -> Result<(), AssetError> {
    let retained = known_retained_bytes(app_data_dir)?;
    let incoming = asset.disclosure.expected_bytes.0;
    let total = retained
        .checked_add(incoming)
        .ok_or(AssetError::StorageFailure)?;
    let same_asset = open_payload_file(app_data_dir, asset, &asset.disclosure.digest_sha256)?
        .map(|file| file.metadata().map(|metadata| metadata.len()))
        .transpose()
        .map_err(|_| AssetError::StorageFailure)?
        .unwrap_or(0);
    if incoming > asset.disclosure.max_staging_bytes.0
        || same_asset.saturating_add(incoming) > asset.disclosure.max_disk_bytes.0
        || total > MAX_MODEL_WEIGHTS_BYTES
        || total > MAX_ASSET_STORAGE_BYTES
    {
        return Err(AssetError::StorageFailure);
    }
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: fstatvfs fills a correctly sized output structure for the live asset root descriptor.
    if unsafe { libc::fstatvfs(root.directory.as_raw_fd(), stats.as_mut_ptr()) } != 0 {
        return Err(AssetError::StorageFailure);
    }
    // SAFETY: fstatvfs returned success and initialized `stats`.
    let stats = unsafe { stats.assume_init() };
    let free = (stats.f_bavail as u64)
        .checked_mul(stats.f_frsize)
        .ok_or(AssetError::StorageFailure)?;
    if free < incoming {
        return Err(AssetError::StorageFailure);
    }
    Ok(())
}

fn known_retained_bytes(app_data_dir: &Path) -> Result<u64, AssetError> {
    let mut total = 0u64;
    for asset in fixed_assets()? {
        if let Some(file) = open_payload_file(app_data_dir, asset, &asset.disclosure.digest_sha256)?
        {
            validate_readonly_model(&file, asset.disclosure.expected_bytes.0)?;
            total = total
                .checked_add(
                    file.metadata()
                        .map_err(|_| AssetError::StorageFailure)?
                        .len(),
                )
                .ok_or(AssetError::StorageFailure)?;
        }
    }
    Ok(total)
}

fn remove_stage(root: &AssetRoot, asset: &FixedAsset, generation: u64) -> Result<(), AssetError> {
    let Some(staging) = root.staging.as_ref() else {
        return Ok(());
    };
    let name = format!("{}-{generation}", asset.asset_id);
    remove_stage_named(staging, &name, None, None)
}

fn remove_stage_named(
    staging: &File,
    name: &str,
    expected_dir_identity: Option<(u64, u64)>,
    expected_payload_identity: Option<(u64, u64)>,
) -> Result<(), AssetError> {
    let Some(directory) = open_private_directory(staging, name, false)? else {
        return Ok(());
    };
    let dir_metadata = directory
        .metadata()
        .map_err(|_| AssetError::StorageFailure)?;
    if let Some((device, inode)) = expected_dir_identity {
        if dir_metadata.dev() != device || dir_metadata.ino() != inode {
            return Err(AssetError::CleanupFailed);
        }
    }
    if !entry_is_missing(&directory, "payload")? {
        let payload = open_file_at(
            &directory,
            "payload",
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )?;
        let metadata = payload.metadata().map_err(|_| AssetError::StorageFailure)?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
            || !matches!(metadata.mode() & 0o777, 0o400 | 0o600)
            || expected_payload_identity
                .is_some_and(|(device, inode)| metadata.dev() != device || metadata.ino() != inode)
        {
            return Err(AssetError::CleanupFailed);
        }
        let name = CString::new("payload").expect("fixed component has no NUL");
        // SAFETY: parent is the verified private stage directory and this removes only its fixed
        // payload name; unlinkat never follows the final symlink.
        if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(AssetError::CleanupFailed);
        }
    }
    drop(directory);
    let name = CString::new(name).map_err(|_| AssetError::CleanupFailed)?;
    // SAFETY: this removes the fixed private stage directory only when it is empty.
    if unsafe { libc::unlinkat(staging.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(AssetError::CleanupFailed);
    }
    Ok(())
}
fn remove_promoted_version(
    app_data_dir: &Path,
    asset: &FixedAsset,
    version: &str,
) -> Result<(), AssetError> {
    if version != asset.disclosure.digest_sha256 {
        return Err(AssetError::InvalidBinding);
    }
    let Some(root) = open_asset_root(app_data_dir, false)? else {
        return Ok(());
    };
    remove_version_directory(&root, asset, version)
}

fn remove_asset_versions(app_data_dir: &Path, asset: &FixedAsset) -> Result<(), AssetError> {
    let Some(root) = open_asset_root(app_data_dir, false)? else {
        return Ok(());
    };
    remove_version_directory(&root, asset, &asset.disclosure.digest_sha256)
}

fn remove_version_directory(
    root: &AssetRoot,
    asset: &FixedAsset,
    version: &str,
) -> Result<(), AssetError> {
    if !is_sha256(version) || version != asset.disclosure.digest_sha256 {
        return Err(AssetError::InvalidBinding);
    }
    let Some(versions) = open_model_versions_dir(root, asset, false)? else {
        return Ok(());
    };
    let Some(version_dir) = open_private_directory(&versions, version, false)? else {
        return Ok(());
    };
    if !entry_is_missing(&version_dir, "payload")? {
        let payload = open_file_at(
            &version_dir,
            "payload",
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )?;
        validate_readonly_model(&payload, asset.disclosure.expected_bytes.0)?;
        if hash_file(&payload)? != asset.disclosure.digest_sha256 {
            return Err(AssetError::CleanupFailed);
        }
        let name = CString::new("payload").expect("fixed component has no NUL");
        // SAFETY: this removes only the verified app-owned payload entry without following a link.
        if unsafe { libc::unlinkat(version_dir.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(AssetError::CleanupFailed);
        }
    }
    drop(version_dir);
    let name = CString::new(version).map_err(|_| AssetError::InvalidBinding)?;
    // SAFETY: `version` is the compiled catalog digest and the directory is empty after payload
    // removal; unlinkat cannot recurse or follow a symlink.
    if unsafe { libc::unlinkat(versions.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(AssetError::CleanupFailed);
    }
    versions
        .sync_all()
        .map_err(|_| AssetError::StorageFailure)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    struct TempFile(PathBuf);

    impl TempFile {
        fn with_bytes(bytes: &[u8]) -> (Self, File) {
            let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "saturday-voice-assets-{}-{sequence}.gguf",
                std::process::id()
            ));
            let mut file = OpenOptions::new()
                .write(true)
                .read(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
            drop(file);
            let file = OpenOptions::new().read(true).open(&path).unwrap();
            (Self(path), file)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let path = home.join(format!(
                ".saturday-voice-assets-test-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn gguf_string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }

    fn gguf_fixture(architecture: &str, context: u32, template: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GGUF");
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&5u64.to_le_bytes());

        gguf_string(&mut bytes, "general.architecture");
        bytes.extend_from_slice(&8u32.to_le_bytes());
        gguf_string(&mut bytes, architecture);

        gguf_string(&mut bytes, "qwen3.context_length");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&context.to_le_bytes());

        gguf_string(&mut bytes, "tokenizer.chat_template");
        bytes.extend_from_slice(&8u32.to_le_bytes());
        gguf_string(&mut bytes, template);

        gguf_string(&mut bytes, "tokenizer.ggml.model");
        bytes.extend_from_slice(&8u32.to_le_bytes());
        gguf_string(&mut bytes, "qwen2");
        gguf_string(&mut bytes, "tokenizer.ggml.tokens");
        bytes.extend_from_slice(&9u32.to_le_bytes());
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        gguf_string(&mut bytes, "token");
        bytes
    }

    #[test]
    fn fixed_catalog_discloses_only_the_two_named_and_pinned_models() {
        let assets = fixed_assets().unwrap();
        assert_eq!(assets.len(), 2);

        let asr = asset_by_id("whisper-base").unwrap();
        assert_eq!(asr.purpose, AssetPurpose::AsrWeights);
        assert_eq!(
            asr.disclosure.candidate_name,
            "Multilingual Whisper base (ggml-base.bin)"
        );
        assert_eq!(
            asr.disclosure.source_revision,
            "5359861c739e955e79d9a303bcbc70fb988958b1"
        );
        assert_eq!(asr.disclosure.expected_bytes.0, 147_951_465);
        assert_eq!(
            asr.disclosure.digest_sha256,
            "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"
        );

        let llm = asset_by_id("qwen3-4b-q4-k-m").unwrap();
        assert_eq!(llm.purpose, AssetPurpose::LlmWeights);
        assert_eq!(llm.disclosure.candidate_name, "Qwen3-4B-Q4_K_M.gguf");
        assert_eq!(
            llm.disclosure.source_revision,
            "bc640142c66e1fdd12af0bd68f40445458f3869b"
        );
        assert_eq!(llm.disclosure.expected_bytes.0, 2_497_280_256);
        assert_eq!(
            llm.disclosure.digest_sha256,
            "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5"
        );

        let combined = asr.disclosure.expected_bytes.0 + llm.disclosure.expected_bytes.0;
        assert!(combined <= MAX_MODEL_WEIGHTS_BYTES);
        assert!(asset_by_id("small-model").is_err());
    }

    #[test]
    fn fixed_download_urls_and_redirects_never_accept_caller_origins() {
        let asr = asset_by_id("whisper-base").unwrap();
        assert_eq!(
            asr.download_url(),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.bin"
        );
        let llm = asset_by_id("qwen3-4b-q4-k-m").unwrap();
        assert_eq!(
            llm.download_url(),
            "https://huggingface.co/Qwen/Qwen3-4B-GGUF/resolve/bc640142c66e1fdd12af0bd68f40445458f3869b/Qwen3-4B-Q4_K_M.gguf"
        );
        assert!(!redirect_origin_allowed(
            &llm.disclosure,
            "http://huggingface.co"
        ));
        assert!(!redirect_origin_allowed(
            &llm.disclosure,
            "https://evil.example"
        ));
        assert!(redirect_origin_allowed(
            &llm.disclosure,
            "https://cas-bridge.xethub.hf.co"
        ));
        assert!(redirect_origin_allowed(
            &llm.disclosure,
            "https://us.aws.cdn.hf.co"
        ));
    }

    #[test]
    fn qwen_gguf_inspection_binds_architecture_context_template_and_tokenizer_metadata() {
        let template = "{%- for message in messages %}{{ message['role'] }}{% endfor %}";
        let bytes = gguf_fixture("qwen3", 32_768, template);
        let (_path, file) = TempFile::with_bytes(&bytes);
        let identity = inspect_qwen_gguf(&file, bytes.len() as u64).unwrap();

        assert_eq!(identity.architecture, "qwen3");
        assert_eq!(identity.context_length, 32_768);
        assert_eq!(identity.template_sha256, sha256_hex(template.as_bytes()));
        assert_ne!(
            identity.tokenizer_metadata_sha256,
            sha256_hex(template.as_bytes())
        );
    }

    #[test]
    fn qwen_gguf_inspection_rejects_wrong_architecture_context_and_unbounded_metadata() {
        let template = "{%- for message in messages %}{{ message['role'] }}{% endfor %}";
        for bytes in [
            gguf_fixture("llama", 32_768, template),
            gguf_fixture("qwen3", 16_384, template),
        ] {
            let (_path, file) = TempFile::with_bytes(&bytes);
            assert!(inspect_qwen_gguf(&file, bytes.len() as u64).is_err());
        }

        let mut oversized = b"GGUF".to_vec();
        oversized.extend_from_slice(&3u32.to_le_bytes());
        oversized.extend_from_slice(&0u64.to_le_bytes());
        oversized.extend_from_slice(&u64::MAX.to_le_bytes());
        let (_path, file) = TempFile::with_bytes(&oversized);
        assert!(inspect_qwen_gguf(&file, oversized.len() as u64).is_err());
    }

    #[test]
    fn app_owned_store_rejects_symlinked_asset_path_components() {
        let temp = TempDir::new();
        let app_data = temp.0.join("app-data");
        let outside = temp.0.join("outside");
        fs::create_dir(&app_data).unwrap();
        fs::set_permissions(&app_data, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, app_data.join("voice")).unwrap();

        assert!(matches!(
            open_asset_root(&app_data, true),
            Err(AssetError::StorageFailure)
        ));
        assert!(!outside.join("assets").exists());
    }

    #[test]
    fn model_open_requires_readonly_single_link_identity() {
        let (path, readonly) = TempFile::with_bytes(b"approved");
        let writable = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path.0)
            .unwrap();
        fs::set_permissions(&path.0, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(validate_readonly_model(&writable, 8).is_err());
        assert!(validate_readonly_model(&readonly, 8).is_ok());

        let hardlink = path.0.with_extension("hardlink");
        fs::hard_link(&path.0, &hardlink).unwrap();
        assert!(validate_readonly_model(&readonly, 8).is_err());
        fs::remove_file(hardlink).unwrap();
    }

    #[test]
    fn staging_cleanup_refuses_hardlinks_and_symlinks() {
        let temp = TempDir::new();
        let app_data = temp.0.join("app-data");
        fs::create_dir(&app_data).unwrap();
        fs::set_permissions(&app_data, fs::Permissions::from_mode(0o700)).unwrap();
        let root = open_asset_root(&app_data, true).unwrap().unwrap();
        let staging = root.staging.as_ref().unwrap();

        let hardlink_stage = create_private_directory_exclusive(staging, "whisper-base-1").unwrap();
        let mut payload = open_file_at(
            &hardlink_stage,
            "payload",
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
        .unwrap();
        payload.write_all(b"private").unwrap();
        payload.sync_all().unwrap();
        drop(payload);
        let payload_path = temp
            .0
            .join("app-data/voice/assets/staging/whisper-base-1/payload");
        let outside = temp.0.join("outside-payload");
        fs::hard_link(&payload_path, &outside).unwrap();
        assert!(matches!(
            remove_stage_named(staging, "whisper-base-1", None, None),
            Err(AssetError::CleanupFailed)
        ));
        assert!(outside.exists());
        fs::remove_file(outside).unwrap();
        remove_stage_named(staging, "whisper-base-1", None, None).unwrap();

        let symlink_stage = create_private_directory_exclusive(staging, "whisper-base-2").unwrap();
        let outside_target = temp.0.join("outside-target");
        fs::write(&outside_target, b"do not remove").unwrap();
        std::os::unix::fs::symlink(&outside_target, temp.0.join("outside-target-link")).unwrap();
        std::os::unix::fs::symlink(
            temp.0.join("outside-target-link"),
            temp.0
                .join("app-data/voice/assets/staging/whisper-base-2/payload"),
        )
        .unwrap();
        assert!(remove_stage_named(staging, "whisper-base-2", None, None).is_err());
        assert_eq!(fs::read(&outside_target).unwrap(), b"do not remove");
        drop(symlink_stage);
    }

    #[test]
    fn tokenizer_fingerprint_changes_with_tokens_and_duplicate_keys_are_rejected() {
        let template = "{%- for message in messages %}{{ message['role'] }}{% endfor %}";
        let original = gguf_fixture("qwen3", 32_768, template);
        let (_original_path, original_file) = TempFile::with_bytes(&original);
        let original_identity = inspect_qwen_gguf(&original_file, original.len() as u64).unwrap();

        let mut changed = original.clone();
        let token_offset = changed
            .windows(5)
            .rposition(|window| window == b"token")
            .unwrap();
        changed[token_offset] = b'v';
        let (_changed_path, changed_file) = TempFile::with_bytes(&changed);
        let changed_identity = inspect_qwen_gguf(&changed_file, changed.len() as u64).unwrap();
        assert_ne!(
            original_identity.tokenizer_metadata_sha256,
            changed_identity.tokenizer_metadata_sha256
        );

        let mut duplicate = original;
        duplicate[16..24].copy_from_slice(&6u64.to_le_bytes());
        gguf_string(&mut duplicate, "tokenizer.ggml.model");
        duplicate.extend_from_slice(&8u32.to_le_bytes());
        gguf_string(&mut duplicate, "qwen2");
        let (_duplicate_path, duplicate_file) = TempFile::with_bytes(&duplicate);
        assert!(inspect_qwen_gguf(&duplicate_file, duplicate.len() as u64).is_err());
    }
}
