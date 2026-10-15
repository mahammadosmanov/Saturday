use crate::voice::contract::{
    Language, MAX_HISTORY_BYTES, MAX_HISTORY_MESSAGES, MAX_SPOKEN_BYTES, MAX_SPOKEN_SCALARS,
    MAX_TYPED_BYTES, MAX_TYPED_SCALARS,
};
use serde::Serialize;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestLimitError {
    TooLong,
    ReviewRequired,
}

pub fn validate_typed_request(text: &str) -> Result<(), RequestLimitError> {
    if text.len() > MAX_TYPED_BYTES || text.chars().count() > MAX_TYPED_SCALARS {
        Err(RequestLimitError::TooLong)
    } else {
        Ok(())
    }
}

pub fn validate_spoken_request(text: &str) -> Result<(), RequestLimitError> {
    if text.len() > MAX_SPOKEN_BYTES || text.chars().count() > MAX_SPOKEN_SCALARS {
        Err(RequestLimitError::ReviewRequired)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedTurn {
    pub(crate) turn_id: String,
    pub(crate) user_language: Language,
    pub(crate) assistant_language: Language,
    pub(crate) user_text: String,
    pub(crate) assistant_text: String,
}

impl CompletedTurn {
    pub(crate) fn new(
        turn_id: String,
        user_language: Language,
        assistant_language: Language,
        user_text: String,
        assistant_text: String,
    ) -> Self {
        Self {
            turn_id,
            user_language,
            assistant_language,
            user_text,
            assistant_text,
        }
    }

    fn text_bytes(&self) -> usize {
        self.user_text.len() + self.assistant_text.len()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryEviction {
    pub turns: usize,
    pub text_bytes: usize,
}

#[derive(Debug, Default)]
pub struct CompletedTurnStore {
    completed: VecDeque<CompletedTurn>,
    text_bytes: usize,
}

impl CompletedTurnStore {
    pub fn commit(&mut self, turn: CompletedTurn) -> Result<HistoryEviction, HistoryError> {
        let Some(turn_bytes) = turn.user_text.len().checked_add(turn.assistant_text.len()) else {
            return Err(HistoryError::TurnTooLarge);
        };
        if turn_bytes > MAX_HISTORY_BYTES {
            return Err(HistoryError::TurnTooLarge);
        }
        let mut eviction = HistoryEviction::default();
        self.completed.push_back(turn);
        self.text_bytes += turn_bytes;

        while self.completed.len() * 2 > MAX_HISTORY_MESSAGES || self.text_bytes > MAX_HISTORY_BYTES
        {
            if let Some(oldest) = self.completed.pop_front() {
                let bytes = oldest.text_bytes();
                self.text_bytes -= bytes;
                eviction.turns += 1;
                eviction.text_bytes += bytes;
            } else {
                return Err(HistoryError::TurnTooLarge);
            }
        }
        Ok(eviction)
    }

    pub fn clear(&mut self) {
        self.completed = VecDeque::new();
        self.text_bytes = 0;
    }

    pub fn completed(&self) -> &VecDeque<CompletedTurn> {
        &self.completed
    }

    pub fn text_bytes(&self) -> usize {
        self.text_bytes
    }
}

#[derive(Serialize)]
struct PromptHistory<'a> {
    prompt_version: u8,
    system_prompt: &'a str,
    history: Vec<PromptHistoryPair<'a>>,
}

#[derive(Serialize)]
struct PromptHistoryPair<'a> {
    user: &'a str,
    assistant: &'a str,
}

pub fn encode_prompt_history(
    history: &CompletedTurnStore,
    system_prompt: &str,
) -> Result<String, HistoryError> {
    if history.completed.len() * 2 > MAX_HISTORY_MESSAGES
        || history.text_bytes > MAX_HISTORY_BYTES
        || system_prompt.chars().any(invalid_context_control)
        || history.completed.iter().any(|turn| {
            turn.user_text.chars().any(invalid_context_control)
                || turn.assistant_text.chars().any(invalid_context_control)
        })
    {
        return Err(HistoryError::InvalidContext);
    }
    let history_pairs = history
        .completed
        .iter()
        .map(|turn| PromptHistoryPair {
            user: &turn.user_text,
            assistant: &turn.assistant_text,
        })
        .collect();
    let serialized = serde_json::to_string(&PromptHistory {
        prompt_version: 1,
        system_prompt,
        history: history_pairs,
    })
    .map_err(|_| HistoryError::InvalidContext)?;
    if serialized.len() > MAX_HISTORY_BYTES {
        return Err(HistoryError::ContextTooLarge);
    }
    Ok(serialized)
}

fn invalid_context_control(character: char) -> bool {
    character.is_control() && !matches!(character, '\n' | '\t')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryError {
    TurnTooLarge,
    ContextTooLarge,
    InvalidContext,
}

#[cfg(test)]
mod tests {
    use super::{
        encode_prompt_history, validate_spoken_request, validate_typed_request, CompletedTurn,
        CompletedTurnStore, HistoryError, RequestLimitError,
    };
    use crate::voice::contract::Language;

    fn turn(index: usize, user: &str, assistant: &str) -> CompletedTurn {
        CompletedTurn::new(
            format!("turn-{index}"),
            Language::En,
            Language::En,
            user.to_owned(),
            assistant.to_owned(),
        )
    }

    #[test]
    fn typed_overflow_rejected_unsliced() {
        assert!(validate_typed_request(&"a".repeat(500)).is_ok());
        assert_eq!(
            validate_typed_request(&"a".repeat(501)),
            Err(RequestLimitError::TooLong)
        );
        assert_eq!(
            validate_typed_request(&"🧭".repeat(501)),
            Err(RequestLimitError::TooLong)
        );
        assert_eq!(validate_typed_request(&"🧭".repeat(500)), Ok(()));
    }

    #[test]
    fn spoken_overflow_is_review_only_not_generation() {
        assert!(validate_spoken_request(&"ş".repeat(2000)).is_ok());
        assert_eq!(
            validate_spoken_request(&"ş".repeat(2001)),
            Err(RequestLimitError::ReviewRequired)
        );
        assert_eq!(
            validate_spoken_request(&"🧭".repeat(2001)),
            Err(RequestLimitError::ReviewRequired)
        );
    }

    #[test]
    fn history_evicts_whole_completed_turn_pairs() {
        let mut history = CompletedTurnStore::default();
        for index in 0..21 {
            history.commit(turn(index, "user", "assistant")).unwrap();
        }
        assert_eq!(history.completed().len(), 20);
        assert_eq!(history.completed().front().unwrap().turn_id, "turn-1");
        assert_eq!(history.completed().back().unwrap().turn_id, "turn-20");
    }

    #[test]
    fn history_byte_cap_evicts_whole_oldest_pairs() {
        let mut history = CompletedTurnStore::default();
        for index in 0..5 {
            history
                .commit(turn(index, &"u".repeat(8_192), &"a".repeat(8_192)))
                .unwrap();
        }
        assert_eq!(history.completed().len(), 4);
        assert_eq!(history.completed().front().unwrap().turn_id, "turn-1");
        assert_eq!(history.completed().back().unwrap().turn_id, "turn-4");
        assert!(history.text_bytes() <= 65_536);
    }

    #[test]
    fn clear_releases_all_history_records() {
        let mut history = CompletedTurnStore::default();
        history
            .commit(turn(0, "private prompt", "private answer"))
            .unwrap();
        history.clear();
        assert!(history.completed().is_empty());
        assert_eq!(history.text_bytes(), 0);
    }

    #[test]
    fn model_context_serializes_complete_pairs_without_ids_or_partial_messages() {
        let mut history = CompletedTurnStore::default();
        history
            .commit(turn(0, "Önceki soru", "Önceki yanıt"))
            .unwrap();
        let encoded = encode_prompt_history(&history, "Bilingual system").unwrap();
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["prompt_version"], 1);
        assert_eq!(value["system_prompt"], "Bilingual system");
        assert_eq!(value["history"].as_array().unwrap().len(), 1);
        assert_eq!(value["history"][0]["user"], "Önceki soru");
        assert_eq!(value["history"][0]["assistant"], "Önceki yanıt");
        assert!(value["history"][0].get("turn_id").is_none());
    }

    #[test]
    fn model_context_rejects_controls_and_serialized_oversize_without_truncating() {
        let mut history = CompletedTurnStore::default();
        history.commit(turn(0, "unsafe\0text", "answer")).unwrap();
        assert_eq!(
            encode_prompt_history(&history, "system"),
            Err(HistoryError::InvalidContext)
        );
        history.clear();
        assert_eq!(
            encode_prompt_history(&history, &"s".repeat(65_537)),
            Err(HistoryError::ContextTooLarge)
        );
    }
}
