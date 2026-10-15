use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Tr,
    En,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageMode {
    Tr,
    En,
    Auto,
}

impl From<Language> for LanguageMode {
    fn from(language: Language) -> Self {
        match language {
            Language::Tr => Self::Tr,
            Language::En => Self::En,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrQualityFlag {
    LowSignal,
    MixedLanguage,
    UnsupportedLanguage,
    Truncated,
    Overlong,
    NoSpeech,
    UncalibratedAuto,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptEligibility {
    Eligible,
    NeedsReview,
    NoSpeech,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "code", rename_all = "snake_case")]
pub enum AsrLanguage {
    Tr,
    En,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageEvidence {
    pub language: AsrLanguage,
    pub rank: u8,
    pub score: f32,
    pub score_gap: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsrResult {
    pub text: String,
    pub ranked_languages: Vec<LanguageEvidence>,
    pub no_speech_score: Option<f32>,
    pub quality_flags: Vec<AsrQualityFlag>,
    pub eligibility: TranscriptEligibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "snake_case")]
pub enum AsrError {
    #[error("unavailable")]
    Unavailable,
    #[error("asset_unavailable")]
    AssetUnavailable,
    #[error("invalid_audio")]
    InvalidAudio,
    #[error("no_speech")]
    NoSpeech,
    #[error("unsupported_language")]
    UnsupportedLanguage,
    #[error("timeout")]
    Timeout,
    #[error("cancelled")]
    Cancelled,
    #[error("worker_failed")]
    WorkerFailed,
    #[error("invalid_evidence")]
    InvalidEvidence,
    #[error("cleanup_failed")]
    CleanupFailed,
    #[error("failed")]
    Failed,
}

const CANCEL_ACTIVE: u8 = 0;
const CANCELLED_BY_USER: u8 = 1;
const COMMIT_IN_PROGRESS: u8 = 2;
const CANCELLED_BY_AUTHORIZATION_CHANGE: u8 = 4;

#[derive(Debug, Clone)]
pub struct CancelToken(Arc<AtomicU8>);

#[derive(Debug)]
pub struct CancelCommitGuard(Arc<AtomicU8>);

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationReason {
    User,
    AuthorizationChanged,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationFailure {
    Unavailable,
    AssetUnavailable,
    InvalidContext,
    InvalidOutput,
    Timeout,
    Cancelled,
    Backpressure,
    WorkerIdentity,
    WorkerProtocol,
    CleanupFailed,
    Failed,
}

impl CancelToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicU8::new(CANCEL_ACTIVE)))
    }

    pub fn cancel(&self) -> bool {
        self.0
            .compare_exchange(
                CANCEL_ACTIVE,
                CANCELLED_BY_USER,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub fn cancel_for_authorization_change(&self) -> bool {
        loop {
            let state = self.0.load(Ordering::Acquire);
            if state == COMMIT_IN_PROGRESS || state == CANCELLED_BY_AUTHORIZATION_CHANGE {
                return false;
            }
            if self
                .0
                .compare_exchange(
                    state,
                    CANCELLED_BY_AUTHORIZATION_CHANGE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return true;
            }
        }
    }

    pub fn cancellation_reason(&self) -> Option<CancellationReason> {
        match self.0.load(Ordering::Acquire) {
            CANCELLED_BY_USER => Some(CancellationReason::User),
            CANCELLED_BY_AUTHORIZATION_CHANGE => {
                Some(CancellationReason::AuthorizationChanged)
            }
            _ => None,
        }
    }

    pub fn begin_commit(&self) -> Option<CancelCommitGuard> {
        self.0
            .compare_exchange(
                CANCEL_ACTIVE,
                COMMIT_IN_PROGRESS,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()
            .map(|_| CancelCommitGuard(Arc::clone(&self.0)))
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation_reason().is_some()
    }
}

impl Drop for CancelCommitGuard {
    fn drop(&mut self) {
        self.0
            .fetch_and(!COMMIT_IN_PROGRESS, Ordering::Release);
    }
}

#[cfg(test)]
mod cancel_token_tests {
    use super::CancelToken;

    #[test]
    fn cancellation_wins_commit_for_all_clones() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(token.cancel());
        assert!(clone.begin_commit().is_none());
        assert!(clone.is_cancelled());
    }

    #[test]
    fn commit_fences_cancellation_only_until_the_effect_finishes() {
        let token = CancelToken::new();
        let clone = token.clone();
        let guard = token.begin_commit().unwrap();
        assert!(!clone.cancel());
        assert!(!clone.is_cancelled());
        drop(guard);
        assert!(clone.cancel());
        assert!(token.is_cancelled());
    }

    #[test]
    fn authorization_change_wins_user_cancel_race_but_not_commit() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        for _ in 0..64 {
            let token = CancelToken::new();
            let barrier = Arc::new(Barrier::new(3));
            let user = {
                let token = token.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    token.cancel();
                })
            };
            let authorization = {
                let token = token.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    token.cancel_for_authorization_change();
                })
            };
            barrier.wait();
            user.join().unwrap();
            authorization.join().unwrap();
            assert_eq!(
                token.cancellation_reason(),
                Some(super::CancellationReason::AuthorizationChanged)
            );
        }

        let fenced = CancelToken::new();
        let guard = fenced.begin_commit().unwrap();
        assert!(!fenced.cancel_for_authorization_change());
        assert_eq!(fenced.cancellation_reason(), None);
        drop(guard);
        assert!(fenced.cancel_for_authorization_change());
    }
}
