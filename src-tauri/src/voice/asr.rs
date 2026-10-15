use std::sync::Arc;

use crate::voice::contract::{
    AsrError, AsrResult, AssetManager, AssetPurpose, LanguageMode, LocalAsrProvider, OwnedPcm,
    TurnContext,
};
use crate::voice::worker::VoiceWorkerClient;
use crate::voice_authorization::{ApprovedModelHandle, WorkerRole};

struct ActiveOperation<'a>(&'a mut bool);

impl Drop for ActiveOperation<'_> {
    fn drop(&mut self) {
        *self.0 = false;
    }
}
/// Launches one reviewed, fixed-role ASR worker for each explicit transcription.
pub struct NativeAsrProvider {
    assets: Arc<dyn AssetManager>,
    active: bool,
}

impl NativeAsrProvider {
    pub fn new(assets: Arc<dyn AssetManager>) -> Self {
        Self {
            assets,
            active: false,
        }
    }
}

impl LocalAsrProvider for NativeAsrProvider {
    fn transcribe(
        &mut self,
        context: &TurnContext,
        pcm: OwnedPcm,
        language_mode: LanguageMode,
        model: ApprovedModelHandle,
        worker_session: Arc<crate::voice_authorization::AuthorizedWorkerSession>,
    ) -> Result<AsrResult, AsrError> {
        if self.active || model.purpose() != AssetPurpose::AsrWeights {
            return Err(AsrError::AssetUnavailable);
        }
        let closure = self
            .assets
            .runtime_closure(model.asset_id())
            .map_err(|_| AsrError::AssetUnavailable)?;
        if closure.role() != WorkerRole::Asr {
            return Err(AsrError::AssetUnavailable);
        }
        closure
            .validate_current_authorization_state()
            .map_err(|_| AsrError::AssetUnavailable)?;

        self.active = true;
        let active = ActiveOperation(&mut self.active);
        let result = VoiceWorkerClient::transcribe_asr(
            &closure,
            &worker_session,
            model,
            pcm,
            language_mode,
            context,
        );
        drop(active);
        result
    }
}
