use super::{
    is_sha256, sha256_hex, AssetPurpose, VoiceAuthorizationError, VoiceBindingReview, WorkerRole,
    VOICE_ASR_MAX_RSS_BYTES, VOICE_BINDING_SCHEMA, VOICE_KEY_APPLICATION_TAG,
    VOICE_LLM_MAX_RSS_BYTES,
};
use crate::voice::contract::AssetDisclosure;
use serde::{Deserialize, Serialize};

pub(super) const MAX_BINDING_BYTES: usize = 256 * 1024;
const WHISPER_ASSET_ID: &str = "whisper-base";
const WHISPER_SOURCE_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
const WHISPER_BYTES: u64 = 147_951_465;
const WHISPER_SHA256: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";
const QWEN_ASSET_ID: &str = "qwen3-4b-q4-k-m";
const QWEN_SOURCE_REVISION: &str = "bc640142c66e1fdd12af0bd68f40445458f3869b";
const QWEN_BYTES: u64 = 2_497_280_256;
const QWEN_SHA256: &str = "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5";

/* Set only from an independently reviewed, final signed model/runtime binding. */
const REVIEWED_CATALOG_SHA256: Option<&str> = option_env!("SATURDAY_VOICE_ASSET_BINDING_SHA256");
const REVIEWER_ROSTER_SHA256: Option<&str> = option_env!("SATURDAY_VOICE_REVIEWER_ROSTER_SHA256");

pub(super) const EMBEDDED_BINDING: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/voice-asset-bindings.json"));

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BindingCatalog {
    schema: String,
    state: String,
    purpose: String,
    owner_key: OwnerKeyBinding,
    review: IndependentReview,
    assets: Vec<AssetBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generation: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnerKeyBinding {
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    application_tag_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    trusted_public_key_x963_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    trusted_public_key_sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IndependentReview {
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    binding_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reviewer_roster_sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AssetBinding {
    asset_id: String,
    purpose: AssetPurpose,
    disclosure: AssetDisclosure,
    generation: u64,
    source_generation_sha256: String,
    source_tree_sha256: String,
    resource_generation_sha256: String,
    template_fingerprint_sha256: Option<String>,
    worker: WorkerBinding,
    code_identity: CodeIdentityBinding,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorkerBinding {
    role: WorkerRole,
    executable_relative_path: String,
    bytes: u64,
    sha256: String,
    source_seal_sha256: String,
    resource_seal_sha256: String,
    max_rss_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CodeIdentityBinding {
    application_team_id_sha256: String,
    application_requirement_sha256: String,
    worker_team_id_sha256: String,
    worker_requirement_sha256: String,
}

#[derive(Debug, Clone)]
pub(super) struct FrozenCatalog {
    pub(super) binding_sha256: String,
    pub(super) key_id_sha256: String,
    pub(super) owner_public_key_x963: Option<Vec<u8>>,
    pub(super) reviewer_roster_sha256: String,
    pub(super) generation: u64,
    pub(super) assets: Vec<FrozenAsset>,
}

#[derive(Debug, Clone)]
pub(super) struct FrozenAsset {
    pub(super) asset_id: String,
    pub(super) purpose: AssetPurpose,
    pub(super) disclosure: AssetDisclosure,
    pub(super) source_generation_sha256: String,
    pub(super) source_tree_sha256: String,
    pub(super) resource_generation_sha256: String,
    pub(super) template_fingerprint_sha256: Option<String>,
    pub(super) worker: FrozenWorker,
    pub(super) code_identity: FrozenCodeIdentity,
}

#[derive(Debug, Clone)]
pub(super) struct FrozenWorker {
    pub(super) role: WorkerRole,
    pub(super) executable_relative_path: String,
    pub(super) bytes: u64,
    pub(super) sha256: String,
    pub(super) source_seal_sha256: String,
    pub(super) resource_seal_sha256: String,
    pub(super) max_rss_bytes: u64,
}

#[derive(Debug, Clone)]
pub(super) struct FrozenCodeIdentity {
    pub(super) application_team_id_sha256: String,
    pub(super) application_requirement_sha256: String,
    pub(super) worker_team_id_sha256: String,
    pub(super) worker_requirement_sha256: String,
}

pub(super) fn is_unprovisioned_bootstrap_profile() -> Result<bool, VoiceAuthorizationError> {
    if EMBEDDED_BINDING.is_empty() || EMBEDDED_BINDING.len() > MAX_BINDING_BYTES {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    let profile: BindingCatalog = serde_json::from_slice(EMBEDDED_BINDING)
        .map_err(|_| VoiceAuthorizationError::BindingInvalid)?;
    if profile.schema != VOICE_BINDING_SCHEMA || profile.purpose != "voice_only" {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    Ok(profile.state == "unprovisioned"
        && profile.owner_key.state == "unprovisioned"
        && profile.owner_key.application_tag_sha256.is_none()
        && profile.owner_key.trusted_public_key_x963_hex.is_none()
        && profile.owner_key.trusted_public_key_sha256.is_none()
        && profile.review.state == "pending_independent_review"
        && profile.review.binding_sha256.is_none()
        && profile.review.reviewer_roster_sha256.is_none()
        && profile.assets.is_empty()
        && profile.generation.is_none())
}

pub(super) fn load_embedded_catalog() -> Result<FrozenCatalog, VoiceAuthorizationError> {
    parse_catalog(
        EMBEDDED_BINDING,
        REVIEWED_CATALOG_SHA256,
        REVIEWER_ROSTER_SHA256,
    )
}

fn parse_catalog(
    bytes: &[u8],
    reviewed_catalog_sha256: Option<&str>,
    reviewer_roster_sha256: Option<&str>,
) -> Result<FrozenCatalog, VoiceAuthorizationError> {
    if bytes.is_empty() || bytes.len() > MAX_BINDING_BYTES {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    let catalog: BindingCatalog =
        serde_json::from_slice(bytes).map_err(|_| VoiceAuthorizationError::BindingInvalid)?;
    if catalog.schema != VOICE_BINDING_SCHEMA || catalog.purpose != "voice_only" {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    if catalog.state != "frozen"
        || !matches!(catalog.owner_key.state.as_str(), "bootstrap" | "key_pinned")
        || catalog.review.state != "approved_independent_review"
        || catalog.assets.len() != 2
    {
        return Err(VoiceAuthorizationError::BindingUnfrozen);
    }
    let (Some(reviewed_catalog_sha256), Some(reviewer_roster_sha256)) =
        (reviewed_catalog_sha256, reviewer_roster_sha256)
    else {
        return Err(VoiceAuthorizationError::BindingUnfrozen);
    };
    if !is_sha256(reviewed_catalog_sha256) || !is_sha256(reviewer_roster_sha256) {
        return Err(VoiceAuthorizationError::BindingUnfrozen);
    }
    let encoded =
        serde_json::to_vec(&catalog).map_err(|_| VoiceAuthorizationError::BindingInvalid)?;
    if encoded != bytes || sha256_hex(bytes) != reviewed_catalog_sha256 {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    let owner_public_key_x963 = match (
        catalog.owner_key.state.as_str(),
        catalog.owner_key.trusted_public_key_x963_hex.as_deref(),
        catalog.owner_key.trusted_public_key_sha256.as_deref(),
    ) {
        ("bootstrap", None, None) => None,
        ("key_pinned", Some(public_hex), Some(public_sha256)) => {
            if public_hex.len() != 130
                || !public_hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || !is_sha256(public_sha256)
            {
                return Err(VoiceAuthorizationError::BindingInvalid);
            }
            let public_key = decode_public_key_hex(public_hex)?;
            if public_key.first() != Some(&0x04) || sha256_hex(&public_key) != public_sha256 {
                return Err(VoiceAuthorizationError::BindingInvalid);
            }
            Some(public_key)
        }
        _ => return Err(VoiceAuthorizationError::BindingInvalid),
    };
    let computed_key_id = sha256_hex(VOICE_KEY_APPLICATION_TAG);
    if catalog.owner_key.application_tag_sha256.as_deref() != Some(computed_key_id.as_str())
        || catalog.review.reviewer_roster_sha256.as_deref() != Some(reviewer_roster_sha256)
        || !is_sha256(catalog.review.binding_sha256.as_deref().unwrap_or_default())
        || catalog.generation.unwrap_or_default() == 0
    {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    let assets_sha256 = sha256_hex(
        &serde_json::to_vec(&catalog.assets)
            .map_err(|_| VoiceAuthorizationError::BindingInvalid)?,
    );
    if catalog.review.binding_sha256.as_deref() != Some(assets_sha256.as_str()) {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }

    let mut frozen_assets = Vec::with_capacity(2);
    for asset in catalog.assets {
        let expected = match asset.asset_id.as_str() {
            WHISPER_ASSET_ID => Some((
                AssetPurpose::AsrWeights,
                WorkerRole::Asr,
                WHISPER_SOURCE_REVISION,
                WHISPER_BYTES,
                WHISPER_SHA256,
                VOICE_ASR_MAX_RSS_BYTES,
                false,
            )),
            QWEN_ASSET_ID => Some((
                AssetPurpose::LlmWeights,
                WorkerRole::Llm,
                QWEN_SOURCE_REVISION,
                QWEN_BYTES,
                QWEN_SHA256,
                VOICE_LLM_MAX_RSS_BYTES,
                true,
            )),
            _ => None,
        }
        .ok_or(VoiceAuthorizationError::BindingInvalid)?;
        let expected_name = if asset.asset_id == WHISPER_ASSET_ID {
            "Multilingual Whisper base (ggml-base.bin)"
        } else {
            "Qwen3-4B-Q4_K_M.gguf"
        };
        let expected_origin = if asset.asset_id == WHISPER_ASSET_ID {
            "https://huggingface.co/ggerganov/whisper.cpp"
        } else {
            "https://huggingface.co/Qwen/Qwen3-4B-GGUF"
        };
        let expected_license = if asset.asset_id == WHISPER_ASSET_ID {
            "MIT"
        } else {
            "Apache-2.0"
        };
        let expected_notice = if asset.asset_id == WHISPER_ASSET_ID {
            "whisper.cpp-MIT"
        } else {
            "qwen3-4b-Apache-2.0"
        };
        let expected_redirects = [
            "https://huggingface.co".to_owned(),
            "https://cdn-lfs.huggingface.co".to_owned(),
            "https://cas-bridge.xethub.hf.co".to_owned(),
            "https://us.aws.cdn.hf.co".to_owned(),
        ];
        if asset.purpose != expected.0
            || asset.disclosure.purpose != expected.0
            || asset.worker.role != expected.1
            || asset.disclosure.source_revision != expected.2
            || asset.disclosure.version != expected.2
            || asset.disclosure.candidate_name != expected_name
            || asset.disclosure.source_origin != expected_origin
            || asset.disclosure.license_ids != [expected_license]
            || asset.disclosure.notice_ids != [expected_notice]
            || asset.disclosure.redirect_origins != expected_redirects
            || asset.disclosure.expected_bytes.0 != expected.3
            || asset.disclosure.max_staging_bytes.0 != expected.3
            || asset.disclosure.max_retained_bytes.0 != expected.3
            || asset.disclosure.max_disk_bytes.0 != expected.3 * 2
            || asset.disclosure.digest_sha256 != expected.4
            || asset.disclosure.retention
                != crate::voice::contract::AssetRetention::AppManagedUntilExplicitRemoval
            || asset.disclosure.build.is_some()
            || asset.generation == 0
            || asset.generation > catalog.generation.unwrap_or_default()
            || (!expected.6 && asset.template_fingerprint_sha256.is_some())
            || asset.worker.max_rss_bytes == 0
            || asset.worker.max_rss_bytes > expected.5
            || asset.worker.bytes == 0
            || asset.worker.executable_relative_path.is_empty()
            || asset.worker.executable_relative_path.starts_with('/')
            || asset.worker.executable_relative_path.contains("..")
            || !is_sha256(&asset.source_generation_sha256)
            || !is_sha256(&asset.source_tree_sha256)
            || !is_sha256(&asset.resource_generation_sha256)
            || !is_sha256(&asset.worker.sha256)
            || !is_sha256(&asset.worker.source_seal_sha256)
            || !is_sha256(&asset.worker.resource_seal_sha256)
            || !is_sha256(&asset.code_identity.application_team_id_sha256)
            || !is_sha256(&asset.code_identity.application_requirement_sha256)
            || !is_sha256(&asset.code_identity.worker_team_id_sha256)
            || !is_sha256(&asset.code_identity.worker_requirement_sha256)
            || asset
                .template_fingerprint_sha256
                .as_deref()
                .is_some_and(|digest| !is_sha256(digest))
        {
            return Err(VoiceAuthorizationError::BindingInvalid);
        }
        frozen_assets.push(FrozenAsset {
            asset_id: asset.asset_id,
            purpose: asset.purpose,
            disclosure: asset.disclosure,
            source_generation_sha256: asset.source_generation_sha256,
            source_tree_sha256: asset.source_tree_sha256,
            resource_generation_sha256: asset.resource_generation_sha256,
            template_fingerprint_sha256: asset.template_fingerprint_sha256,
            worker: FrozenWorker {
                role: asset.worker.role,
                executable_relative_path: asset.worker.executable_relative_path,
                bytes: asset.worker.bytes,
                sha256: asset.worker.sha256,
                source_seal_sha256: asset.worker.source_seal_sha256,
                resource_seal_sha256: asset.worker.resource_seal_sha256,
                max_rss_bytes: asset.worker.max_rss_bytes,
            },
            code_identity: FrozenCodeIdentity {
                application_team_id_sha256: asset.code_identity.application_team_id_sha256,
                application_requirement_sha256: asset.code_identity.application_requirement_sha256,
                worker_team_id_sha256: asset.code_identity.worker_team_id_sha256,
                worker_requirement_sha256: asset.code_identity.worker_requirement_sha256,
            },
        });
    }
    frozen_assets.sort_by(|left, right| left.asset_id.cmp(&right.asset_id));
    if frozen_assets[0].asset_id != WHISPER_ASSET_ID || frozen_assets[1].asset_id != QWEN_ASSET_ID {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    Ok(FrozenCatalog {
        binding_sha256: reviewed_catalog_sha256.to_owned(),
        key_id_sha256: computed_key_id,
        owner_public_key_x963,
        reviewer_roster_sha256: reviewer_roster_sha256.to_owned(),
        generation: catalog.generation.unwrap_or_default(),
        assets: frozen_assets,
    })
}

fn decode_public_key_hex(value: &str) -> Result<Vec<u8>, VoiceAuthorizationError> {
    if value.len() != 130
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    let mut decoded = Vec::with_capacity(65);
    for pair in value.as_bytes().as_chunks::<2>().0 {
        let nibble = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        let high = nibble(pair[0]).ok_or(VoiceAuthorizationError::BindingInvalid)?;
        let low = nibble(pair[1]).ok_or(VoiceAuthorizationError::BindingInvalid)?;
        decoded.push((high << 4) | low);
    }
    if decoded.first() != Some(&0x04) {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    Ok(decoded)
}

impl FrozenCatalog {
    pub(super) fn review(&self) -> VoiceBindingReview {
        VoiceBindingReview {
            binding_sha256: self.binding_sha256.clone(),
            key_id_sha256: self.key_id_sha256.clone(),
            reviewer_roster_sha256: self.reviewer_roster_sha256.clone(),
        }
    }

    pub(super) fn asset(&self, asset_id: &str) -> Result<&FrozenAsset, VoiceAuthorizationError> {
        self.assets
            .iter()
            .find(|asset| asset.asset_id == asset_id)
            .ok_or(VoiceAuthorizationError::AssetMismatch)
    }

    pub(super) fn asset_by_purpose(
        &self,
        purpose: AssetPurpose,
    ) -> Result<&FrozenAsset, VoiceAuthorizationError> {
        self.assets
            .iter()
            .find(|asset| asset.purpose == purpose)
            .ok_or(VoiceAuthorizationError::AssetMismatch)
    }

    pub(super) fn asset_for_disclosure(
        &self,
        disclosure: &AssetDisclosure,
    ) -> Result<&FrozenAsset, VoiceAuthorizationError> {
        self.assets
            .iter()
            .find(|asset| asset.disclosure == *disclosure)
            .ok_or(VoiceAuthorizationError::AssetMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_unprovisioned_template_cannot_authorize_models() {
        assert!(matches!(
            load_embedded_catalog(),
            Err(VoiceAuthorizationError::BindingUnfrozen)
        ));
    }

    #[test]
    fn catalog_rejects_duplicate_and_unknown_members_before_effects() {
        let duplicate = br#"{"schema":"saturday.voice.asset-bindings.v1","schema":"saturday.voice.asset-bindings.v1","state":"unprovisioned","purpose":"voice_only","owner_key":{"state":"unprovisioned"},"review":{"state":"pending_independent_review"},"assets":[]}"#;
        let unknown = br#"{"schema":"saturday.voice.asset-bindings.v1","state":"unprovisioned","purpose":"voice_only","owner_key":{"state":"unprovisioned"},"review":{"state":"pending_independent_review"},"assets":[],"alternate_ledger":"/tmp/ledger"}"#;
        assert!(matches!(
            parse_catalog(duplicate, Some(&"a".repeat(64)), Some(&"b".repeat(64))),
            Err(VoiceAuthorizationError::BindingInvalid)
        ));
        assert!(matches!(
            parse_catalog(unknown, Some(&"a".repeat(64)), Some(&"b".repeat(64))),
            Err(VoiceAuthorizationError::BindingInvalid)
        ));
    }

    #[test]
    fn catalog_source_pins_are_exact_and_canonical() {
        assert_eq!(WHISPER_BYTES, 147_951_465);
        assert_eq!(QWEN_BYTES, 2_497_280_256);
        assert!(is_sha256(WHISPER_SHA256));
        assert!(is_sha256(QWEN_SHA256));
        assert_ne!(WHISPER_ASSET_ID, QWEN_ASSET_ID);
    }
}
