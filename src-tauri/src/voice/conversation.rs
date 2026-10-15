use crate::voice::context::{
    validate_spoken_request, validate_typed_request, CompletedTurn, CompletedTurnStore,
    HistoryEviction,
};
use crate::voice::contract::{
    AcceptedUtterance, ConversationEvent, ConversationFailure, Language, OperationId,
    TranscriptEligibility, UtteranceSource, VoiceOwner, MAX_MODEL_OUTPUT_BYTES,
    MAX_MODEL_OUTPUT_TOKENS, MAX_TEXT_DELTA_BYTES,
};

#[derive(Debug)]
struct ActiveTurn {
    owner: VoiceOwner,
    operation_id: OperationId,
    utterance: AcceptedUtterance,
    response_language: Language,
    answer: String,
    next_sequence: u64,
}
#[derive(Debug)]
struct PartialAnswer {
    text: String,
    language: Language,
    reason: ConversationFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnUpdate {
    TextDelta {
        sequence: u64,
        start: usize,
        end: usize,
    },
    Completed {
        output_tokens: u16,
        history_eviction: HistoryEviction,
    },
    Partial {
        reason: ConversationFailure,
    },
}

#[derive(Debug, Default)]
pub struct ConversationBuffer {
    active: Option<ActiveTurn>,
    partial: Option<PartialAnswer>,
}

impl ConversationBuffer {
    pub fn begin(
        &mut self,
        owner: VoiceOwner,
        operation_id: OperationId,
        utterance: AcceptedUtterance,
        response_language: Language,
    ) -> Result<(), ConversationFailure> {
        if self.active.is_some() {
            return Err(ConversationFailure::InvalidContext);
        }
        let limit_result = match utterance.source {
            UtteranceSource::Typed => validate_typed_request(&utterance.text),
            UtteranceSource::Speech => {
                if utterance.eligibility != TranscriptEligibility::Eligible {
                    return Err(ConversationFailure::InvalidContext);
                }
                validate_spoken_request(&utterance.text)
            }
        };
        if limit_result.is_err() {
            return Err(ConversationFailure::InvalidContext);
        }
        self.partial = None;
        self.active = Some(ActiveTurn {
            owner,
            operation_id,
            utterance,
            response_language,
            answer: String::with_capacity(MAX_MODEL_OUTPUT_BYTES),
            next_sequence: 0,
        });
        Ok(())
    }

    pub fn accept(
        &mut self,
        event: ConversationEvent,
        history: &mut CompletedTurnStore,
    ) -> Result<TurnUpdate, ConversationFailure> {
        match event {
            ConversationEvent::TextDelta {
                owner,
                operation_id,
                sequence,
                text,
            } => {
                let active = self
                    .active
                    .as_mut()
                    .ok_or(ConversationFailure::WorkerProtocol)?;
                validate_owner(active, &owner, operation_id)?;
                if sequence != active.next_sequence
                    || text.is_empty()
                    || text.len() > MAX_TEXT_DELTA_BYTES
                {
                    return Err(ConversationFailure::WorkerProtocol);
                }
                let next_sequence = sequence
                    .checked_add(1)
                    .ok_or(ConversationFailure::WorkerProtocol)?;
                let start = active.answer.len();
                let end = start
                    .checked_add(text.len())
                    .ok_or(ConversationFailure::InvalidOutput)?;
                if end > MAX_MODEL_OUTPUT_BYTES {
                    return Err(ConversationFailure::InvalidOutput);
                }
                active.answer.push_str(&text);
                active.next_sequence = next_sequence;
                Ok(TurnUpdate::TextDelta {
                    sequence,
                    start,
                    end,
                })
            }
            ConversationEvent::Completed {
                owner,
                operation_id,
                output_tokens,
            } => {
                let current = self
                    .active
                    .as_ref()
                    .ok_or(ConversationFailure::WorkerProtocol)?;
                validate_owner(current, &owner, operation_id)?;
                let active = self
                    .active
                    .take()
                    .ok_or(ConversationFailure::WorkerProtocol)?;
                if output_tokens == 0
                    || output_tokens > MAX_MODEL_OUTPUT_TOKENS as u16
                    || active.answer.is_empty()
                    || active
                        .utterance
                        .text
                        .len()
                        .checked_add(active.answer.len())
                        .is_none_or(|bytes| bytes > crate::voice::contract::MAX_HISTORY_BYTES)
                {
                    self.partial = Some(PartialAnswer {
                        text: active.answer,
                        language: active.response_language,
                        reason: ConversationFailure::InvalidOutput,
                    });
                    return Err(ConversationFailure::InvalidOutput);
                }
                let turn = CompletedTurn::new(
                    active.owner.turn_id,
                    active.utterance.language,
                    active.response_language,
                    active.utterance.text,
                    active.answer,
                );
                let history_eviction = history
                    .commit(turn)
                    .map_err(|_| ConversationFailure::InvalidContext)?;
                Ok(TurnUpdate::Completed {
                    output_tokens,
                    history_eviction,
                })
            }
            ConversationEvent::Failed {
                owner,
                operation_id,
                failure,
            } => {
                let current = self
                    .active
                    .as_ref()
                    .ok_or(ConversationFailure::WorkerProtocol)?;
                validate_owner(current, &owner, operation_id)?;
                let active = self
                    .active
                    .take()
                    .ok_or(ConversationFailure::WorkerProtocol)?;
                self.partial = Some(PartialAnswer {
                    text: active.answer,
                    language: active.response_language,
                    reason: failure,
                });
                Ok(TurnUpdate::Partial { reason: failure })
            }
        }
    }

    pub fn available_delta_bytes(&self) -> usize {
        self.active.as_ref().map_or(0, |active| {
            MAX_MODEL_OUTPUT_BYTES
                .saturating_sub(active.answer.len())
                .min(MAX_TEXT_DELTA_BYTES)
        })
    }

    pub fn answer_bytes(&self) -> usize {
        self.active.as_ref().map_or_else(
            || {
                self.partial
                    .as_ref()
                    .map_or(0, |partial| partial.text.len())
            },
            |active| active.answer.len(),
        )
    }

    pub fn active_text(&self) -> Option<&str> {
        self.active.as_ref().map(|active| active.answer.as_str())
    }

    pub fn partial_text(&self) -> Option<&str> {
        self.partial.as_ref().map(|partial| partial.text.as_str())
    }

    pub fn partial_language(&self) -> Option<Language> {
        self.partial.as_ref().map(|partial| partial.language)
    }

    pub fn partial_reason(&self) -> Option<&ConversationFailure> {
        self.partial.as_ref().map(|partial| &partial.reason)
    }

    pub fn clear(&mut self) {
        self.active = None;
        self.partial = None;
    }
}

fn validate_owner(
    active: &ActiveTurn,
    owner: &VoiceOwner,
    operation_id: OperationId,
) -> Result<(), ConversationFailure> {
    if active.owner != *owner || active.operation_id != operation_id {
        Err(ConversationFailure::WorkerProtocol)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ConversationBuffer, TurnUpdate};
    use crate::voice::context::CompletedTurnStore;
    use crate::voice::contract::{
        AcceptedUtterance, ConversationEvent, ConversationFailure, Language, OperationId,
        TranscriptEligibility, UtteranceSource, VoiceOwner,
    };

    fn owner() -> VoiceOwner {
        VoiceOwner {
            session_id: "session-1".to_owned(),
            turn_id: "turn-1".to_owned(),
            generation: 1,
        }
    }

    fn utterance(text: &str) -> AcceptedUtterance {
        AcceptedUtterance {
            text: text.to_owned(),
            language: Language::En,
            source: UtteranceSource::Typed,
            eligibility: TranscriptEligibility::Eligible,
        }
    }

    #[test]
    fn answer_buffer_enforces_exact_delta_and_answer_byte_caps() {
        let mut buffer = ConversationBuffer::default();
        let mut history = CompletedTurnStore::default();
        buffer
            .begin(owner(), OperationId(1), utterance("question"), Language::En)
            .unwrap();
        for sequence in 0..2 {
            assert_eq!(
                buffer.accept(
                    ConversationEvent::TextDelta {
                        owner: owner(),
                        operation_id: OperationId(1),
                        sequence,
                        text: "x".repeat(4096),
                    },
                    &mut history,
                ),
                Ok(TurnUpdate::TextDelta {
                    sequence,
                    start: sequence as usize * 4096,
                    end: (sequence as usize + 1) * 4096,
                })
            );
        }
        assert_eq!(buffer.answer_bytes(), 8192);
        assert_eq!(
            buffer.accept(
                ConversationEvent::TextDelta {
                    owner: owner(),
                    operation_id: OperationId(1),
                    sequence: 2,
                    text: "x".to_owned(),
                },
                &mut history,
            ),
            Err(ConversationFailure::InvalidOutput)
        );
        assert!(history.completed().is_empty());
    }

    #[test]
    fn history_accepts_only_completed_successful_turns() {
        let mut buffer = ConversationBuffer::default();
        let mut history = CompletedTurnStore::default();
        buffer
            .begin(owner(), OperationId(1), utterance("question"), Language::En)
            .unwrap();
        buffer
            .accept(
                ConversationEvent::TextDelta {
                    owner: owner(),
                    operation_id: OperationId(1),
                    sequence: 0,
                    text: "partial".to_owned(),
                },
                &mut history,
            )
            .unwrap();
        assert_eq!(
            buffer.accept(
                ConversationEvent::Failed {
                    owner: owner(),
                    operation_id: OperationId(1),
                    failure: ConversationFailure::Cancelled,
                },
                &mut history,
            ),
            Ok(TurnUpdate::Partial {
                reason: ConversationFailure::Cancelled
            })
        );
        assert!(history.completed().is_empty());
        assert_eq!(buffer.partial_text(), Some("partial"));

        buffer
            .begin(
                owner(),
                OperationId(2),
                utterance("next question"),
                Language::Tr,
            )
            .unwrap();
        buffer
            .accept(
                ConversationEvent::TextDelta {
                    owner: owner(),
                    operation_id: OperationId(2),
                    sequence: 0,
                    text: "Tamamlandı.".to_owned(),
                },
                &mut history,
            )
            .unwrap();
        assert!(matches!(
            buffer.accept(
                ConversationEvent::Completed {
                    owner: owner(),
                    operation_id: OperationId(2),
                    output_tokens: 8,
                },
                &mut history,
            ),
            Ok(TurnUpdate::Completed { .. })
        ));
        assert_eq!(history.completed().len(), 1);
        assert_eq!(
            history.completed().front().unwrap().user_text,
            "next question"
        );
        assert_eq!(
            history.completed().front().unwrap().assistant_text,
            "Tamamlandı."
        );
    }

    #[test]
    fn stale_owner_and_replayed_delta_sequence_are_rejected() {
        let mut buffer = ConversationBuffer::default();
        let mut history = CompletedTurnStore::default();
        buffer
            .begin(owner(), OperationId(1), utterance("question"), Language::En)
            .unwrap();
        let delta = |owner, sequence| ConversationEvent::TextDelta {
            owner,
            operation_id: OperationId(1),
            sequence,
            text: "answer".to_owned(),
        };
        let mut wrong_owner = owner();
        wrong_owner.generation = 2;
        assert_eq!(
            buffer.accept(delta(wrong_owner, 0), &mut history),
            Err(ConversationFailure::WorkerProtocol)
        );
        assert_eq!(buffer.answer_bytes(), 0);
        buffer.accept(delta(owner(), 0), &mut history).unwrap();
        assert_eq!(
            buffer.accept(delta(owner(), 0), &mut history),
            Err(ConversationFailure::WorkerProtocol)
        );
        assert_eq!(buffer.answer_bytes(), 6);
    }

    #[test]
    fn literal_reasoning_examples_remain_readable_and_inert() {
        let mut buffer = ConversationBuffer::default();
        let mut history = CompletedTurnStore::default();
        buffer
            .begin(
                owner(),
                OperationId(1),
                utterance("What does <think> mean in an example?"),
                Language::En,
            )
            .unwrap();
        buffer
            .accept(
                ConversationEvent::TextDelta {
                    owner: owner(),
                    operation_id: OperationId(1),
                    sequence: 0,
                    text: "The literal marker <think> can appear in documentation.".to_owned(),
                },
                &mut history,
            )
            .unwrap();
        assert_eq!(
            buffer.active_text(),
            Some("The literal marker <think> can appear in documentation.")
        );
        assert!(history.completed().is_empty());
    }
}
