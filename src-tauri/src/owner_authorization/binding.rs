use crate::owner_authorization::canonical::{
    canonical_json_bytes, decode_canonical_base64, decode_lower_hex,
};
use crate::owner_authorization::{BINDING_SCHEMA, OWNER_KEY_TAG, PUBLIC_KEY_SCHEMA};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const OWNER_SIGNATURE_ALGORITHM: &str = "ecdsa-p256-sha256-x962";
const MAX_PUBLIC_KEY_RECORD_BYTES: usize = 4096;
const MAX_BINDING_BYTES: usize = 1024 * 1024;
const CANDIDATE_ID: &str = "qwen35-4b-q4km-homebrew-llamacpp-0.4.1";
const CANDIDATE_SHA256: &str = "b2672f1a00121c09077bc447da7fff720238012daf2ecd645add58c48124a5bb";
const CANDIDATE_REVIEW_AGGREGATE: &str =
    "9178a4708e4e5a834c936fe8f8ff7a76ee3c45e8aee7fcb90bfe9101989e5a8c";
const REVOKED_REVIEW_AGGREGATES: [&str; 3] = [
    "f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0",
    "380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0",
    "1c49497d67bc21f3a1a3edf49bca82be180ca517fce3380ead4b99389c9e4bc0",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicKeyAnchor {
    Unprovisioned,
    Pinned {
        key_id: [u8; 32],
        public_key_x963: [u8; 65],
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateBinding {
    pub candidate_id: String,
    pub candidate_sha256: [u8; 32],
    pub review_aggregate_sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewBinding {
    pub review_aggregate_sha256: [u8; 32],
    pub review_manifest_sha256: [u8; 32],
    pub review_sidecar_sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactBinding {
    pub formula: String,
    pub version: String,
    pub repository: String,
    pub layer_sha256: [u8; 32],
    pub transfer_bytes: u64,
    pub installed_bytes: Option<u64>,
    pub maximum_extracted_bytes: Option<u64>,
    pub post_install_execution: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginPolicyBinding {
    pub registry: String,
    pub token_realm: String,
    pub service: String,
    pub allowed_blob_redirect_origin: String,
    pub authorization_forwarding: String,
    pub strip_authorization_on_cross_origin: bool,
    pub redirect_limit: u64,
    pub reject_redirect_from_blob_origin: bool,
    pub content_length_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitBinding {
    pub artifact_count: u64,
    pub verified_layer_total_bytes: u64,
    pub maximum_transfer_bytes: u64,
    pub maximum_total_extracted_bytes: u64,
    pub maximum_archive_entries: u64,
    pub minimum_free_bytes_before_each_body_read: u64,
    pub formula_execution_allowed: bool,
    pub post_install_execution_allowed: bool,
    pub origin_policy: OriginPolicyBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTransferState {
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRecord {
    pub path: String,
    pub bytes: u64,
    pub mode: u32,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenBinding {
    pub binding_sha256: [u8; 32],
    pub candidate: CandidateBinding,
    pub review: ReviewBinding,
    pub artifacts: [ArtifactBinding; 5],
    pub limits: LimitBinding,
    pub model_transfer: ModelTransferState,
    pub resource_roster: Vec<ResourceRecord>,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum BindingError {
    #[error("public key record is invalid")]
    InvalidRecord,
    #[error("public key identity is invalid")]
    InvalidPublicKey,
}

pub fn parse_frozen_binding(bytes: &[u8]) -> Result<FrozenBinding, BindingError> {
    let object = parse_canonical_object_bounded(bytes, MAX_BINDING_BYTES)?;
    require_exact_keys(
        &object,
        &[
            "artifacts",
            "candidate",
            "limits",
            "model_transfer",
            "resource_roster",
            "review",
            "schema",
            "state",
        ],
    )?;
    if string_field(&object, "schema")? != BINDING_SCHEMA
        || string_field(&object, "state")? != "frozen"
        || string_field(&object, "model_transfer")? != "blocked"
    {
        return Err(BindingError::InvalidRecord);
    }

    let candidate_object = object_field(&object, "candidate")?;
    require_exact_keys(
        candidate_object,
        &[
            "candidate_id",
            "candidate_sha256",
            "review_aggregate_sha256",
        ],
    )?;
    if string_field(candidate_object, "candidate_id")? != CANDIDATE_ID
        || string_field(candidate_object, "candidate_sha256")? != CANDIDATE_SHA256
        || string_field(candidate_object, "review_aggregate_sha256")? != CANDIDATE_REVIEW_AGGREGATE
    {
        return Err(BindingError::InvalidRecord);
    }
    let candidate = CandidateBinding {
        candidate_id: CANDIDATE_ID.to_owned(),
        candidate_sha256: decode_lower_hex(CANDIDATE_SHA256)
            .map_err(|_| BindingError::InvalidRecord)?,
        review_aggregate_sha256: decode_lower_hex(CANDIDATE_REVIEW_AGGREGATE)
            .map_err(|_| BindingError::InvalidRecord)?,
    };

    let review_object = object_field(&object, "review")?;
    require_exact_keys(
        review_object,
        &[
            "review_aggregate_sha256",
            "review_manifest_sha256",
            "review_sidecar_sha256",
        ],
    )?;
    let review_aggregate = string_field(review_object, "review_aggregate_sha256")?;
    if REVOKED_REVIEW_AGGREGATES.contains(&review_aggregate) {
        return Err(BindingError::InvalidRecord);
    }
    let review = ReviewBinding {
        review_aggregate_sha256: decode_lower_hex(review_aggregate)
            .map_err(|_| BindingError::InvalidRecord)?,
        review_manifest_sha256: decode_lower_hex(string_field(
            review_object,
            "review_manifest_sha256",
        )?)
        .map_err(|_| BindingError::InvalidRecord)?,
        review_sidecar_sha256: decode_lower_hex(string_field(
            review_object,
            "review_sidecar_sha256",
        )?)
        .map_err(|_| BindingError::InvalidRecord)?,
    };

    if object.get("artifacts") != Some(&expected_artifacts_value())
        || object.get("limits") != Some(&expected_limits_value())
    {
        return Err(BindingError::InvalidRecord);
    }
    let resource_roster = parse_resource_roster(
        object
            .get("resource_roster")
            .and_then(Value::as_array)
            .ok_or(BindingError::InvalidRecord)?,
    )?;

    Ok(FrozenBinding {
        binding_sha256: Sha256::digest(bytes).into(),
        candidate,
        review,
        artifacts: expected_artifacts()?,
        limits: expected_limits(),
        model_transfer: ModelTransferState::Blocked,
        resource_roster,
    })
}

pub fn parse_public_key_anchor(bytes: &[u8]) -> Result<PublicKeyAnchor, BindingError> {
    let object = parse_canonical_object(bytes)?;
    match string_field(&object, "state")? {
        "unprovisioned" => {
            require_exact_keys(&object, &["schema", "state"])?;
            require_schema(&object)?;
            Ok(PublicKeyAnchor::Unprovisioned)
        }
        "pinned" => {
            require_exact_keys(
                &object,
                &[
                    "algorithm",
                    "key_id",
                    "public_key_x963_base64",
                    "schema",
                    "state",
                ],
            )?;
            require_schema(&object)?;
            parse_public_identity(&object)
        }
        _ => Err(BindingError::InvalidRecord),
    }
}

pub fn parse_bootstrap_record(bytes: &[u8]) -> Result<PublicKeyAnchor, BindingError> {
    let object = parse_canonical_object(bytes)?;
    require_exact_keys(
        &object,
        &[
            "algorithm",
            "application_tag",
            "key_id",
            "public_key_x963_base64",
            "schema",
        ],
    )?;
    require_schema(&object)?;
    if string_field(&object, "application_tag")? != OWNER_KEY_TAG {
        return Err(BindingError::InvalidRecord);
    }
    parse_public_identity(&object)
}

pub fn bootstrap_record_bytes(public_key_x963: &[u8]) -> Result<Vec<u8>, BindingError> {
    let public_key_x963 = validate_public_key(public_key_x963)?;
    let key_id: [u8; 32] = Sha256::digest(public_key_x963).into();
    canonical_json_bytes(&serde_json::json!({
        "algorithm": OWNER_SIGNATURE_ALGORITHM,
        "application_tag": OWNER_KEY_TAG,
        "key_id": lower_hex(&key_id),
        "public_key_x963_base64": STANDARD.encode(public_key_x963),
        "schema": PUBLIC_KEY_SCHEMA,
    }))
    .map_err(|_| BindingError::InvalidRecord)
}

fn parse_canonical_object(bytes: &[u8]) -> Result<Map<String, Value>, BindingError> {
    parse_canonical_object_bounded(bytes, MAX_PUBLIC_KEY_RECORD_BYTES)
}

fn parse_canonical_object_bounded(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<Map<String, Value>, BindingError> {
    if bytes.is_empty() || bytes.len() > maximum_bytes {
        return Err(BindingError::InvalidRecord);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| BindingError::InvalidRecord)?;
    if canonical_json_bytes(&value).map_err(|_| BindingError::InvalidRecord)? != bytes {
        return Err(BindingError::InvalidRecord);
    }
    value
        .as_object()
        .cloned()
        .ok_or(BindingError::InvalidRecord)
}

fn object_field<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Map<String, Value>, BindingError> {
    object
        .get(field)
        .and_then(Value::as_object)
        .ok_or(BindingError::InvalidRecord)
}

fn parse_resource_roster(values: &[Value]) -> Result<Vec<ResourceRecord>, BindingError> {
    if values.is_empty() {
        return Err(BindingError::InvalidRecord);
    }
    let mut resources = Vec::with_capacity(values.len());
    let mut previous: Option<&str> = None;
    for value in values {
        let object = value.as_object().ok_or(BindingError::InvalidRecord)?;
        require_exact_keys(object, &["bytes", "mode", "path", "sha256"])?;
        let path = string_field(object, "path")?;
        if !safe_relative_path(path) || previous.is_some_and(|item| item >= path) {
            return Err(BindingError::InvalidRecord);
        }
        let bytes = object
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or(BindingError::InvalidRecord)?;
        let mode = object
            .get("mode")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value <= 0o777)
            .ok_or(BindingError::InvalidRecord)?;
        let sha256 = decode_lower_hex(string_field(object, "sha256")?)
            .map_err(|_| BindingError::InvalidRecord)?;
        resources.push(ResourceRecord {
            path: path.to_owned(),
            bytes,
            mode,
            sha256,
        });
        previous = Some(path);
    }
    Ok(resources)
}

fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && path.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn expected_artifacts_value() -> Value {
    serde_json::json!([
        {
            "formula": "llama.cpp",
            "installed_bytes": 19_291_937,
            "layer_sha256": "eb6fb1624c1d7efc90d94092d95b120fbdcff8ab3c0fe148ba6108f9c44d3a2e",
            "repository": "homebrew/core/llama.cpp",
            "transfer_bytes": 7_707_135,
            "version": "0.4.1"
        },
        {
            "formula": "ggml",
            "installed_bytes": 4_775_140,
            "layer_sha256": "31e217a029d28065ffff52cdb09fa975a56717216b663fe95a4ff84dcac15640",
            "repository": "homebrew/core/ggml",
            "transfer_bytes": 1_350_818,
            "version": "0.24.0"
        },
        {
            "formula": "libomp",
            "installed_bytes": 1_811_000,
            "layer_sha256": "41257106da80d8d2f35bd2effc1877292b3462b00ebd5e85108c56dd0fdff879",
            "repository": "homebrew/core/libomp",
            "transfer_bytes": 597_172,
            "version": "23.1.1"
        },
        {
            "formula": "openssl@3",
            "installed_bytes": 37_759_406,
            "layer_sha256": "51eb22ee51d7b6e678ef5dc2be234adb46f9f541ac1d98e5ed08c9a7535789dc",
            "repository": "homebrew/core/openssl/3",
            "transfer_bytes": 10_965_156,
            "version": "3.6.4"
        },
        {
            "formula": "ca-certificates",
            "layer_sha256": "37f5c1413f65c05d86e1279d76699f5d27ad77f8126f5776db13f3d64ee3d010",
            "maximum_extracted_bytes": 16_777_216,
            "post_install_execution": "forbidden",
            "repository": "homebrew/core/ca-certificates",
            "transfer_bytes": 112_184,
            "version": "2026-08-13_1"
        }
    ])
}

fn expected_limits_value() -> Value {
    serde_json::json!({
        "artifact_count": 5,
        "formula_execution_allowed": false,
        "maximum_archive_entries": 20_000,
        "maximum_total_extracted_bytes": 536_870_912,
        "maximum_transfer_bytes": 67_108_864,
        "minimum_free_bytes_before_each_body_read": 2_818_572_288_u64,
        "origin_policy": {
            "allowed_blob_redirect_origin": "https://pkg-containers.githubusercontent.com:443",
            "authorization_forwarding": "same-origin-only",
            "content_length_required": true,
            "redirect_limit": 1,
            "registry": "https://ghcr.io:443",
            "reject_redirect_from_blob_origin": true,
            "service": "ghcr.io",
            "strip_authorization_on_cross_origin": true,
            "token_realm": "https://ghcr.io:443/token"
        },
        "post_install_execution_allowed": false,
        "verified_layer_total_bytes": 20_732_465
    })
}

fn expected_artifacts() -> Result<[ArtifactBinding; 5], BindingError> {
    Ok([
        artifact(
            "llama.cpp",
            "0.4.1",
            "homebrew/core/llama.cpp",
            "eb6fb1624c1d7efc90d94092d95b120fbdcff8ab3c0fe148ba6108f9c44d3a2e",
            7_707_135,
            Some(19_291_937),
            None,
            None,
        )?,
        artifact(
            "ggml",
            "0.24.0",
            "homebrew/core/ggml",
            "31e217a029d28065ffff52cdb09fa975a56717216b663fe95a4ff84dcac15640",
            1_350_818,
            Some(4_775_140),
            None,
            None,
        )?,
        artifact(
            "libomp",
            "23.1.1",
            "homebrew/core/libomp",
            "41257106da80d8d2f35bd2effc1877292b3462b00ebd5e85108c56dd0fdff879",
            597_172,
            Some(1_811_000),
            None,
            None,
        )?,
        artifact(
            "openssl@3",
            "3.6.4",
            "homebrew/core/openssl/3",
            "51eb22ee51d7b6e678ef5dc2be234adb46f9f541ac1d98e5ed08c9a7535789dc",
            10_965_156,
            Some(37_759_406),
            None,
            None,
        )?,
        artifact(
            "ca-certificates",
            "2026-08-13_1",
            "homebrew/core/ca-certificates",
            "37f5c1413f65c05d86e1279d76699f5d27ad77f8126f5776db13f3d64ee3d010",
            112_184,
            None,
            Some(16_777_216),
            Some("forbidden"),
        )?,
    ])
}

#[allow(clippy::too_many_arguments)]
fn artifact(
    formula: &str,
    version: &str,
    repository: &str,
    layer_sha256: &str,
    transfer_bytes: u64,
    installed_bytes: Option<u64>,
    maximum_extracted_bytes: Option<u64>,
    post_install_execution: Option<&str>,
) -> Result<ArtifactBinding, BindingError> {
    Ok(ArtifactBinding {
        formula: formula.to_owned(),
        version: version.to_owned(),
        repository: repository.to_owned(),
        layer_sha256: decode_lower_hex(layer_sha256).map_err(|_| BindingError::InvalidRecord)?,
        transfer_bytes,
        installed_bytes,
        maximum_extracted_bytes,
        post_install_execution: post_install_execution.map(str::to_owned),
    })
}

fn expected_limits() -> LimitBinding {
    LimitBinding {
        artifact_count: 5,
        verified_layer_total_bytes: 20_732_465,
        maximum_transfer_bytes: 67_108_864,
        maximum_total_extracted_bytes: 536_870_912,
        maximum_archive_entries: 20_000,
        minimum_free_bytes_before_each_body_read: 2_818_572_288,
        formula_execution_allowed: false,
        post_install_execution_allowed: false,
        origin_policy: OriginPolicyBinding {
            registry: "https://ghcr.io:443".to_owned(),
            token_realm: "https://ghcr.io:443/token".to_owned(),
            service: "ghcr.io".to_owned(),
            allowed_blob_redirect_origin: "https://pkg-containers.githubusercontent.com:443"
                .to_owned(),
            authorization_forwarding: "same-origin-only".to_owned(),
            strip_authorization_on_cross_origin: true,
            redirect_limit: 1,
            reject_redirect_from_blob_origin: true,
            content_length_required: true,
        },
    }
}

fn parse_public_identity(object: &Map<String, Value>) -> Result<PublicKeyAnchor, BindingError> {
    if string_field(object, "algorithm")? != OWNER_SIGNATURE_ALGORITHM {
        return Err(BindingError::InvalidRecord);
    }
    let public_key = decode_canonical_base64(string_field(object, "public_key_x963_base64")?, 65)
        .map_err(|_| BindingError::InvalidPublicKey)?;
    let public_key = validate_public_key(&public_key)?;
    let key_id = decode_lower_hex::<32>(string_field(object, "key_id")?)
        .map_err(|_| BindingError::InvalidPublicKey)?;
    let expected_key_id: [u8; 32] = Sha256::digest(public_key).into();
    if key_id != expected_key_id {
        return Err(BindingError::InvalidPublicKey);
    }
    Ok(PublicKeyAnchor::Pinned {
        key_id,
        public_key_x963: *public_key,
    })
}

fn require_schema(object: &Map<String, Value>) -> Result<(), BindingError> {
    if string_field(object, "schema")? != PUBLIC_KEY_SCHEMA {
        return Err(BindingError::InvalidRecord);
    }
    Ok(())
}

fn require_exact_keys(object: &Map<String, Value>, expected: &[&str]) -> Result<(), BindingError> {
    if object.len() != expected.len() || !expected.iter().all(|key| object.contains_key(*key)) {
        return Err(BindingError::InvalidRecord);
    }
    Ok(())
}

fn string_field<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str, BindingError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(BindingError::InvalidRecord)
}

fn validate_public_key(public_key: &[u8]) -> Result<&[u8; 65], BindingError> {
    let public_key: &[u8; 65] = public_key
        .try_into()
        .map_err(|_| BindingError::InvalidPublicKey)?;
    if public_key[0] != 0x04 {
        return Err(BindingError::InvalidPublicKey);
    }
    Ok(public_key)
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_BASE64: &str =
        "BBERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERE=";
    const KEY_ID: &str = "f0b9315a0459dab7ab5fa6903e733c917761154216cd90e5d979da6c852c602a";
    const PINNED: &[u8] = b"{\"algorithm\":\"ecdsa-p256-sha256-x962\",\"key_id\":\"f0b9315a0459dab7ab5fa6903e733c917761154216cd90e5d979da6c852c602a\",\"public_key_x963_base64\":\"BBERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERE=\",\"schema\":\"saturday.local-provider.stage1-owner-public-key.v1\",\"state\":\"pinned\"}";
    const BOOTSTRAP: &[u8] = b"{\"algorithm\":\"ecdsa-p256-sha256-x962\",\"application_tag\":\"dev.rohtolos.saturday.stage1-owner-authorization.v1\",\"key_id\":\"f0b9315a0459dab7ab5fa6903e733c917761154216cd90e5d979da6c852c602a\",\"public_key_x963_base64\":\"BBERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERE=\",\"schema\":\"saturday.local-provider.stage1-owner-public-key.v1\"}";

    #[test]
    fn parses_only_exact_unprovisioned_and_pinned_anchors() {
        assert_eq!(
            parse_public_key_anchor(
                b"{\"schema\":\"saturday.local-provider.stage1-owner-public-key.v1\",\"state\":\"unprovisioned\"}"
            )
            .unwrap(),
            PublicKeyAnchor::Unprovisioned
        );

        let mut expected_key = [0x11; 65];
        expected_key[0] = 0x04;
        assert_eq!(
            parse_public_key_anchor(PINNED).unwrap(),
            PublicKeyAnchor::Pinned {
                key_id: hex_key_id(),
                public_key_x963: expected_key,
            }
        );
    }

    #[test]
    fn rejects_noncanonical_unknown_mismatched_and_private_anchor_fields() {
        let invalid = [
            b"{\"schema\":\"saturday.local-provider.stage1-owner-public-key.v1\",\"state\":\"wrong\"}".as_slice(),
            b"{\"extra\":1,\"schema\":\"saturday.local-provider.stage1-owner-public-key.v1\",\"state\":\"unprovisioned\"}".as_slice(),
            b"{ \"schema\":\"saturday.local-provider.stage1-owner-public-key.v1\",\"state\":\"unprovisioned\"}".as_slice(),
        ];
        for bytes in invalid {
            assert!(parse_public_key_anchor(bytes).is_err());
        }

        for (field, replacement) in [
            ("algorithm", "wrong"),
            ("key_id", &format!("{}0", &KEY_ID[..63])),
            ("public_key_x963_base64", "YQ=="),
            ("public_key_x963_base64", &format!("A{}", &KEY_BASE64[1..])),
            ("state", "unprovisioned"),
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(PINNED).unwrap();
            value[field] = serde_json::Value::String(replacement.to_owned());
            let bytes = canonical_json_bytes(&value).unwrap();
            assert!(parse_public_key_anchor(&bytes).is_err(), "accepted {field}");
        }

        for forbidden in ["application_tag", "private_key", "signature"] {
            let mut value: serde_json::Value = serde_json::from_slice(PINNED).unwrap();
            value[forbidden] = serde_json::Value::String("forbidden".to_owned());
            let bytes = canonical_json_bytes(&value).unwrap();
            assert!(parse_public_key_anchor(&bytes).is_err());
        }
    }

    #[test]
    fn bootstrap_record_is_exact_and_validates_tag_key_id_and_public_key() {
        let mut public_key = [0x11; 65];
        public_key[0] = 0x04;
        assert_eq!(bootstrap_record_bytes(&public_key).unwrap(), BOOTSTRAP);
        assert_eq!(
            parse_bootstrap_record(BOOTSTRAP).unwrap(),
            PublicKeyAnchor::Pinned {
                key_id: hex_key_id(),
                public_key_x963: public_key,
            }
        );

        for (field, value) in [
            ("application_tag", "wrong"),
            ("algorithm", "wrong"),
            ("key_id", &format!("{}0", &KEY_ID[..63])),
            ("public_key_x963_base64", "YQ=="),
        ] {
            let mut record: serde_json::Value = serde_json::from_slice(BOOTSTRAP).unwrap();
            record[field] = serde_json::Value::String(value.to_owned());
            let bytes = canonical_json_bytes(&record).unwrap();
            assert!(parse_bootstrap_record(&bytes).is_err(), "accepted {field}");
        }
    }

    #[test]
    fn checked_in_resource_is_exact_unprovisioned_anchor_and_is_bundled() {
        const RESOURCE: &[u8] = include_bytes!("../../resources/stage1-owner-public-key.json");
        assert_eq!(
            RESOURCE,
            b"{\"schema\":\"saturday.local-provider.stage1-owner-public-key.v1\",\"state\":\"unprovisioned\"}"
        );
        assert_eq!(
            parse_public_key_anchor(RESOURCE).unwrap(),
            PublicKeyAnchor::Unprovisioned
        );
        let tauri_config = include_str!("../../tauri.conf.json");
        assert!(tauri_config.contains("\"generated/stage1\": \"saturday-resources/stage1\""));
    }

    #[test]
    fn replays_the_shared_exact_frozen_binding_fixture() {
        const FIXTURE: &[u8] =
            include_bytes!("../../../scripts/tests/fixtures/stage1-owner-binding-frozen.json");
        let binding = parse_frozen_binding(FIXTURE).unwrap();
        assert_eq!(
            binding.candidate.candidate_id,
            "qwen35-4b-q4km-homebrew-llamacpp-0.4.1"
        );
        assert_eq!(binding.artifacts.len(), 5);
        assert_eq!(binding.artifacts[0].formula, "llama.cpp");
        assert_eq!(binding.artifacts[4].formula, "ca-certificates");
        assert_eq!(binding.limits.artifact_count, 5);
        assert_eq!(binding.model_transfer, ModelTransferState::Blocked);
        assert_eq!(binding.resource_roster[0].path, "fixture/resource");
        let expected_digest: [u8; 32] = Sha256::digest(FIXTURE).into();
        assert_eq!(binding.binding_sha256, expected_digest);
    }

    #[test]
    fn rejects_unfrozen_revoked_reordered_and_mutated_binding_values() {
        const FIXTURE: &[u8] =
            include_bytes!("../../../scripts/tests/fixtures/stage1-owner-binding-frozen.json");
        let original: serde_json::Value = serde_json::from_slice(FIXTURE).unwrap();
        let mut invalid = Vec::new();

        let mut value = original.clone();
        value["state"] = serde_json::json!("unfrozen");
        invalid.push(value);

        let mut value = original.clone();
        value["model_transfer"] = serde_json::json!("allowed");
        invalid.push(value);

        let mut value = original.clone();
        value["review"]["review_aggregate_sha256"] =
            serde_json::json!("f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0");
        invalid.push(value);

        let mut value = original.clone();
        value["artifacts"].as_array_mut().unwrap().reverse();
        invalid.push(value);

        let mut value = original.clone();
        value["limits"]["maximum_archive_entries"] = serde_json::json!(1);
        invalid.push(value);

        let mut value = original.clone();
        value["resource_roster"][0]["path"] = serde_json::json!("../escape");
        invalid.push(value);

        for value in invalid {
            let bytes = canonical_json_bytes(&value).unwrap();
            assert!(parse_frozen_binding(&bytes).is_err());
        }
        let mut noncanonical = FIXTURE.to_vec();
        noncanonical.push(b'\n');
        assert!(parse_frozen_binding(&noncanonical).is_err());
    }

    #[test]
    fn checked_in_binding_starts_as_exact_unfrozen_resource() {
        const RESOURCE: &[u8] = include_bytes!("../../resources/stage1-owner-binding.json");
        assert_eq!(
            RESOURCE,
            b"{\"schema\":\"saturday.local-provider.stage1-owner-binding.v1\",\"state\":\"unfrozen\"}"
        );
        assert!(parse_frozen_binding(RESOURCE).is_err());
    }

    fn hex_key_id() -> [u8; 32] {
        decode_lower_hex(KEY_ID).unwrap()
    }
}
