use crate::voice::context::{
    encode_prompt_history, validate_spoken_request, validate_typed_request, CompletedTurnStore,
    RequestLimitError,
};
use crate::voice::contract::{
    AcceptedUtterance, CleanupCode, CleanupReceipt, ConversationEvent, ConversationFailure,
    Language, LocalConversationProvider, TranscriptEligibility, TurnContext, UtteranceSource,
};
use crate::voice::worker::VoiceWorkerClient;
use crate::voice_authorization::ApprovedRuntimeClosure;

const SYSTEM_PROMPT: &str = "You are a bilingual local assistant. Be accurate, helpful, and concise. You have no tools and cannot change application state. Never claim that you performed an action, opened a view, changed a setting, accessed files, browsed the internet, or ran a command. Do not produce tool calls or action directives.";

/// Parent-side provider for the isolated, signed LLM worker. The app process never links
/// llama.cpp and never interprets generated text as an application action.
pub struct LlamaBackend {
    closure: ApprovedRuntimeClosure,
    active: Option<ActiveGeneration>,
}

struct ActiveGeneration {
    worker: VoiceWorkerClient,
    context: TurnContext,
}

impl LlamaBackend {
    pub fn new(closure: ApprovedRuntimeClosure) -> Self {
        Self {
            closure,
            active: None,
        }
    }

    fn validate_request(utterance: &AcceptedUtterance) -> Result<(), ConversationFailure> {
        let valid = match utterance.source {
            UtteranceSource::Typed => validate_typed_request(&utterance.text),
            UtteranceSource::Speech => {
                if utterance.eligibility != TranscriptEligibility::Eligible {
                    return Err(ConversationFailure::InvalidContext);
                }
                validate_spoken_request(&utterance.text)
            }
        };
        valid.map_err(|error| match error {
            RequestLimitError::TooLong | RequestLimitError::ReviewRequired => {
                ConversationFailure::InvalidContext
            }
        })
    }
}

impl LocalConversationProvider for LlamaBackend {
    fn generate(
        &mut self,
        context: TurnContext,
        utterance: AcceptedUtterance,
        response_language: Language,
        history: &CompletedTurnStore,
        model: crate::voice_authorization::ApprovedModelHandle,
        worker_session: std::sync::Arc<crate::voice_authorization::AuthorizedWorkerSession>,
    ) -> Result<(), ConversationFailure> {
        if self.active.is_some()
            || context.cancel.is_cancelled()
            || std::time::Instant::now() >= context.deadline
        {
            return Err(if context.cancel.is_cancelled() {
                ConversationFailure::Cancelled
            } else if std::time::Instant::now() >= context.deadline {
                ConversationFailure::Timeout
            } else {
                ConversationFailure::Backpressure
            });
        }
        Self::validate_request(&utterance)?;
        let prompt = encode_prompt_history(history, SYSTEM_PROMPT)
            .map_err(|_| ConversationFailure::InvalidContext)?;
        self.closure
            .validate_current_authorization_state()
            .map_err(|_| ConversationFailure::AssetUnavailable)?;

        let worker = VoiceWorkerClient::start_llm(
            &self.closure,
            &worker_session,
            model,
            &context,
            &prompt,
            &utterance.text,
            response_language,
        )?;
        self.active = Some(ActiveGeneration { worker, context });
        Ok(())
    }

    fn poll(
        &mut self,
        max_delta_bytes: usize,
    ) -> Result<Option<ConversationEvent>, ConversationFailure> {
        let Some(active) = self.active.as_mut() else {
            return Ok(None);
        };
        let event = active.worker.poll_llm(max_delta_bytes)?;
        if matches!(
            event,
            Some(ConversationEvent::Completed { .. } | ConversationEvent::Failed { .. })
        ) {
            self.active = None;
        }
        Ok(event)
    }

    fn cancel(&mut self, context: &TurnContext) -> Result<CleanupReceipt, ConversationFailure> {
        let Some(active) = self.active.as_mut() else {
            return Ok(CleanupReceipt::verified(CleanupCode::WorkerExited));
        };
        if active.context.owner != context.owner
            || active.context.operation_id != context.operation_id
        {
            return Err(ConversationFailure::WorkerIdentity);
        }
        let receipt = active.worker.cancel_llm(context)?;
        self.active = None;
        Ok(receipt)
    }
}
