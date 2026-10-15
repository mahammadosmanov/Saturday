use super::catalog::load_embedded_catalog;
use super::{
    is_trusted_voice_origin, OwnerConsent, OwnerConsentScope, VerifiedModelFile,
    VoiceAuthorizationError,
};
use crate::voice::contract::{AssetDisclosure, AssetError, CancelToken, VoiceConsentPrompt};
use std::sync::mpsc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const CONSENT_WAIT: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub struct NativeVoiceConsentPrompt {
    app: AppHandle,
}

impl NativeVoiceConsentPrompt {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    pub(crate) fn confirm_owner_setup(
        &self,
        body_en: String,
        body_tr: String,
    ) -> Result<(), VoiceAuthorizationError> {
        if body_en.is_empty()
            || body_en.len() > 16_384
            || body_tr.is_empty()
            || body_tr.len() > 16_384
        {
            return Err(VoiceAuthorizationError::ProtocolFailure);
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let app = self.app.clone();
        app.run_on_main_thread(move || {
            // No renderer caller or caller-provided strings authorize this route; the native
            // app must already have a visible, focused main window and the copy is fixed by the
            // reviewed catalog and current code identity.
            let status = unsafe {
                saturday_voice_present_consent(
                    2,
                    body_en.as_ptr(),
                    body_en.len(),
                    body_tr.as_ptr(),
                    body_tr.len(),
                )
            };
            let result = match status {
                0 => Ok(()),
                -3 => Err(VoiceAuthorizationError::OwnerCancelled),
                _ => Err(VoiceAuthorizationError::OwnerAuthentication),
            };
            let _ = sender.send(result);
        })
        .map_err(|_| VoiceAuthorizationError::OwnerAuthentication)?;
        receiver
            .recv_timeout(CONSENT_WAIT)
            .map_err(|_| VoiceAuthorizationError::OwnerAuthentication)?
    }

    pub(super) fn confirm_model_purpose(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
        candidate: &VerifiedModelFile,
        cancellation: &CancelToken,
    ) -> Result<OwnerConsent, VoiceAuthorizationError> {
        if candidate.purpose != crate::voice::contract::AssetPurpose::LlmWeights
            || candidate.template_sha256.is_none()
            || candidate.tokenizer_metadata_sha256.is_none()
        {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        let catalog = load_embedded_catalog()?;
        let asset = catalog.asset(&candidate.asset_id)?;
        if asset.purpose != candidate.purpose {
            return Err(VoiceAuthorizationError::AssetMismatch);
        }
        let purpose_bytes = model_purpose_bytes(&asset.disclosure, candidate)?;
        let (body_en, body_tr) = model_purpose_copy(&asset.disclosure, candidate)?;
        self.show_and_confirm(1, caller, body_en, body_tr, cancellation)
            .map_err(map_prompt_error)?;
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).map_err(|_| VoiceAuthorizationError::ProtocolFailure)?;
        OwnerConsent::after_native_confirmation(
            &purpose_bytes,
            caller,
            nonce,
            OwnerConsentScope::ModelPurpose {
                asset_id: candidate.asset_id.clone(),
                model_sha256: candidate.payload_sha256.clone(),
                template_sha256: super::sha256_hex(
                    candidate
                        .template_sha256
                        .as_ref()
                        .ok_or(VoiceAuthorizationError::ModelInvalid)?,
                ),
                tokenizer_metadata_sha256: super::sha256_hex(
                    candidate
                        .tokenizer_metadata_sha256
                        .as_ref()
                        .ok_or(VoiceAuthorizationError::ModelInvalid)?,
                ),
            },
        )
    }

    fn show_and_confirm(
        &self,
        purpose: i32,
        caller: &crate::voice::ipc::NativeCallerContext,
        body_en: String,
        body_tr: String,
        cancellation: &CancelToken,
    ) -> Result<(), AssetError> {
        if cancellation.is_cancelled() {
            return Err(AssetError::Cancelled);
        }
        if !caller.is_trusted() || !is_trusted_voice_origin(caller.origin()) {
            return Err(AssetError::ApprovalRequired);
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let app = self.app.clone();
        let caller = caller.clone();
        let cancellation = cancellation.clone();
        app.clone()
            .run_on_main_thread(move || {
                if cancellation.is_cancelled() {
                    let _ = sender.send(Err(AssetError::Cancelled));
                    return;
                }
                let Some(window) = app.get_webview_window(caller.window_label()) else {
                    let _ = sender.send(Err(AssetError::ApprovalRequired));
                    return;
                };
                if caller.ensure_live(&window).is_err() {
                    let _ = sender.send(Err(AssetError::ApprovalRequired));
                    return;
                }
                if body_en.is_empty()
                    || body_en.len() > 16_384
                    || body_tr.is_empty()
                    || body_tr.len() > 16_384
                {
                    let _ = sender.send(Err(AssetError::InvalidBinding));
                    return;
                }
                // SAFETY: both strings are owned by this main-thread closure, their byte lengths
                // are bounded below and the native bridge treats them as UTF-8 display text only.
                let status = unsafe {
                    saturday_voice_present_consent(
                        purpose,
                        body_en.as_ptr(),
                        body_en.len(),
                        body_tr.as_ptr(),
                        body_tr.len(),
                    )
                };
                let result = if cancellation.is_cancelled() {
                    Err(AssetError::Cancelled)
                } else {
                    match status {
                        0 if caller.ensure_live(&window).is_ok() => Ok(()),
                        0 => Err(AssetError::ApprovalRequired),
                        -3 => Err(AssetError::Cancelled),
                        _ => Err(AssetError::ApprovalRequired),
                    }
                };
                let _ = sender.send(result);
            })
            .map_err(|_| AssetError::Unavailable)?;
        receiver
            .recv_timeout(CONSENT_WAIT)
            .map_err(|_| AssetError::Timeout)?
    }
}

impl VoiceConsentPrompt for NativeVoiceConsentPrompt {
    fn confirm(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
        disclosure: &AssetDisclosure,
        cancellation: &CancelToken,
    ) -> Result<OwnerConsent, AssetError> {
        if cancellation.is_cancelled() {
            return Err(AssetError::Cancelled);
        }
        let catalog = load_embedded_catalog().map_err(|error| match error {
            VoiceAuthorizationError::BindingUnfrozen => AssetError::ApprovalRequired,
            VoiceAuthorizationError::BindingInvalid => AssetError::InvalidBinding,
            _ => AssetError::Unavailable,
        })?;
        let asset = catalog
            .asset_for_disclosure(disclosure)
            .map_err(|_| AssetError::InvalidBinding)?;
        let disclosure_bytes =
            serde_json::to_vec(disclosure).map_err(|_| AssetError::InvalidBinding)?;
        if disclosure_bytes.len() > super::MAX_DISCLOSURE_BYTES {
            return Err(AssetError::InvalidBinding);
        }
        let (body_en, body_tr) = download_copy(disclosure);
        self.show_and_confirm(0, caller, body_en, body_tr, cancellation)?;
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).map_err(|_| AssetError::Unavailable)?;
        OwnerConsent::after_native_confirmation(
            &disclosure_bytes,
            caller,
            nonce,
            OwnerConsentScope::Download {
                asset_id: asset.asset_id.clone(),
            },
        )
        .map_err(|_| AssetError::ApprovalRequired)
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn saturday_voice_present_consent(
        purpose: i32,
        body_en: *const u8,
        body_en_len: usize,
        body_tr: *const u8,
        body_tr_len: usize,
    ) -> i32;
}

#[cfg(not(target_os = "macos"))]
unsafe fn saturday_voice_present_consent(
    _purpose: i32,
    _body_en: *const u8,
    _body_en_len: usize,
    _body_tr: *const u8,
    _body_tr_len: usize,
) -> i32 {
    -1
}

fn disclosure_details(disclosure: &AssetDisclosure) -> (String, String) {
    let bytes = disclosure.expected_bytes.0;
    let redirects = disclosure.redirect_origins.join(", ");
    let licenses = disclosure.license_ids.join(", ");
    let notices = disclosure.notice_ids.join(", ");
    let english = format!(
        "Asset: {}\nVersion: {}\nExact file size: {} bytes ({} MB)\nPurpose: {}\nApproved source: {} @ {}\nAllowed redirects: {}\nLicense: {} (notices: {})\nStorage: app-private local storage, retained until you explicitly delete it.\nPrivacy: the file is downloaded only from the listed origins; it is not uploaded or sent to a cloud service. This screen does not record audio.",
        disclosure.candidate_name,
        disclosure.version,
        bytes,
        format_decimal_megabytes(bytes),
        purpose_label(disclosure.purpose, false),
        disclosure.source_origin,
        disclosure.source_revision,
        redirects,
        licenses,
        notices,
    );
    let turkish = format!(
        "Varlık: {}\nSürüm: {}\nKesin dosya boyutu: {} bayt ({} MB)\nAmaç: {}\nOnaylı kaynak: {} @ {}\nİzin verilen yönlendirmeler: {}\nLisans: {} (bildirimler: {})\nDepolama: yalnızca uygulamaya özel yerel depolama; siz açıkça silene kadar tutulur.\nGizlilik: dosya yalnızca listelenen kaynaklardan indirilir; yüklenmez veya bulut hizmetine gönderilmez. Bu ekran ses kaydetmez.",
        disclosure.candidate_name,
        disclosure.version,
        bytes,
        format_decimal_megabytes(bytes),
        purpose_label(disclosure.purpose, true),
        disclosure.source_origin,
        disclosure.source_revision,
        redirects,
        licenses,
        notices,
    );
    (english, turkish)
}

fn download_copy(disclosure: &AssetDisclosure) -> (String, String) {
    let (english, turkish) = disclosure_details(disclosure);
    (
        format!("{english}\nCancel now to create no grant and write no model file."),
        format!("{turkish}\nŞimdi vazgeçerseniz yetki verilmez ve model dosyası yazılmaz."),
    )
}

fn model_purpose_copy(
    disclosure: &AssetDisclosure,
    candidate: &VerifiedModelFile,
) -> Result<(String, String), VoiceAuthorizationError> {
    let template = candidate
        .template_sha256
        .as_ref()
        .ok_or(VoiceAuthorizationError::ModelInvalid)?;
    let tokenizer = candidate
        .tokenizer_metadata_sha256
        .as_ref()
        .ok_or(VoiceAuthorizationError::ModelInvalid)?;
    let template = super::sha256_hex(template);
    let tokenizer = super::sha256_hex(tokenizer);
    let (en, tr) = disclosure_details(disclosure);
    Ok((
        format!(
            "{}\n\nPurpose-binding before activation:\nModel SHA-256: {}\nGGUF architecture: qwen3\nContext length: 32768\nChat-template SHA-256: {}\nTokenizer/framing metadata SHA-256: {}\nCancel now to prevent activation; temporary staging bytes will be cleaned. No model inference or audio capture occurs during this check.",
            en, candidate.payload_sha256, template, tokenizer,
        ),
        format!(
            "{}\n\nEtkinleştirmeden önce amaç bağlama:\nModel SHA-256: {}\nGGUF mimarisi: qwen3\nBağlam uzunluğu: 32768\nSohbet şablonu SHA-256: {}\nBelirteçleyici/çerçeveleme meta verisi SHA-256: {}\nEtkinleştirmeyi önlemek için şimdi vazgeçin; geçici hazırlama verileri temizlenecektir. Bu denetim sırasında model çıkarımı veya ses kaydı yapılmaz.",
            tr, candidate.payload_sha256, template, tokenizer,
        ),
    ))
}

pub(super) fn model_purpose_bytes(
    disclosure: &AssetDisclosure,
    candidate: &VerifiedModelFile,
) -> Result<Vec<u8>, VoiceAuthorizationError> {
    #[derive(serde::Serialize)]
    struct PurposeDisclosure<'a> {
        schema: &'static str,
        disclosure: &'a AssetDisclosure,
        asset_id: &'a str,
        model_sha256: &'a str,
        template_sha256: String,
        tokenizer_metadata_sha256: String,
        architecture: &'static str,
        context_length: u32,
    }
    let purpose = PurposeDisclosure {
        schema: "saturday.voice.model-purpose-consent.v1",
        disclosure,
        asset_id: &candidate.asset_id,
        model_sha256: &candidate.payload_sha256,
        template_sha256: candidate
            .template_sha256
            .as_ref()
            .map(|value| super::sha256_hex(value))
            .ok_or(VoiceAuthorizationError::ModelInvalid)?,
        tokenizer_metadata_sha256: candidate
            .tokenizer_metadata_sha256
            .as_ref()
            .map(|value| super::sha256_hex(value))
            .ok_or(VoiceAuthorizationError::ModelInvalid)?,
        architecture: "qwen3",
        context_length: 32_768,
    };
    let bytes =
        serde_json::to_vec(&purpose).map_err(|_| VoiceAuthorizationError::ProtocolFailure)?;
    if bytes.len() > super::MAX_DISCLOSURE_BYTES {
        return Err(VoiceAuthorizationError::ProtocolFailure);
    }
    Ok(bytes)
}

fn purpose_label(purpose: crate::voice::contract::AssetPurpose, turkish: bool) -> &'static str {
    match (purpose, turkish) {
        (crate::voice::contract::AssetPurpose::AsrWeights, false) => {
            "local speech recognition only"
        }
        (crate::voice::contract::AssetPurpose::AsrWeights, true) => "yalnızca yerel konuşma tanıma",
        (crate::voice::contract::AssetPurpose::LlmWeights, false) => {
            "local bilingual text generation only"
        }
        (crate::voice::contract::AssetPurpose::LlmWeights, true) => {
            "yalnızca yerel iki dilli metin üretimi"
        }
        (_, false) => "voice-specific local use only",
        (_, true) => "yalnızca sese özgü yerel kullanım",
    }
}

fn format_decimal_megabytes(bytes: u64) -> String {
    let tenths = bytes.saturating_mul(10) / 1_000_000;
    format!("{}.{:01}", tenths / 10, tenths % 10)
}

fn map_prompt_error(error: AssetError) -> VoiceAuthorizationError {
    match error {
        AssetError::Cancelled => VoiceAuthorizationError::OwnerCancelled,
        AssetError::ApprovalRequired => VoiceAuthorizationError::OwnerAuthentication,
        AssetError::Timeout => VoiceAuthorizationError::ProtocolFailure,
        _ => VoiceAuthorizationError::KeyUnavailable,
    }
}
