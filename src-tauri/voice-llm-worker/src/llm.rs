use std::ffi::{c_char, c_int, c_void, CStr};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::time::Instant;

use saturday_voice_types::{CancelToken, ConversationFailure, Language};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CONTEXT_TOKENS: u32 = 4_096;
const PROMPT_TOKENS: u32 = 3_840;
const OUTPUT_TOKENS: u16 = 256;
const MAX_CONTEXT_BYTES: usize = 65_536;
const MAX_HISTORY_PAIRS: usize = 20;
const MAX_USER_BYTES: usize = 8_000;
const MAX_USER_SCALARS: usize = 2_000;
const MAX_OUTPUT_BYTES: usize = 8_192;
const MAX_DELTA_BYTES: usize = 4_096;
const MAX_TEMPLATE_BYTES: usize = 16 * 1024;

const OK: c_int = 0;
const PENDING: c_int = 1;
const COMPLETE: c_int = 2;
const CANCELLED: c_int = 3;
const CONTROL_TOKEN: c_int = -5;
const OUTPUT_LIMIT: c_int = -6;
const NEED_CREDIT: c_int = 4;

#[repr(C)]
struct SaturdayLlama {
    _private: [u8; 0],
}

#[repr(C)]
struct LlamaPiece {
    data: *const c_char,
    bytes: usize,
    parse_special_tokens: c_int,
}
struct AbortContext<'a> {
    cancel: &'a CancelToken,
    deadline: Instant,
}

unsafe extern "C" fn llama_should_abort(context: *mut c_void) -> bool {
    if context.is_null() {
        return true;
    }
    let context = unsafe { &*context.cast::<AbortContext<'_>>() };
    context.cancel.is_cancelled() || Instant::now() >= context.deadline
}


unsafe extern "C" {
    fn saturday_llama_load(
        fd: c_int,
        expected_size: u64,
        abort_callback: unsafe extern "C" fn(*mut c_void) -> bool,
        abort_context: *mut c_void,
        out: *mut *mut SaturdayLlama,
    ) -> c_int;
    fn saturday_llama_free(model: *mut SaturdayLlama);
    fn saturday_llama_chat_template(model: *const SaturdayLlama, out: *mut c_char, cap: usize) -> i32;
    fn saturday_llama_count_prompt_tokens(
        model: *const SaturdayLlama,
        pieces: *const LlamaPiece,
        count: usize,
        input_tokens: *mut u32,
    ) -> c_int;
    fn saturday_llama_start(
        model: *mut SaturdayLlama,
        pieces: *const LlamaPiece,
        count: usize,
        maximum_output_tokens: u32,
        input_tokens: *mut u32,
    ) -> c_int;
    fn saturday_llama_poll(
        model: *mut SaturdayLlama,
        max_delta_bytes: usize,
        delta: *mut c_char,
        delta_capacity: usize,
        delta_bytes: *mut usize,
        delta_tokens: *mut u32,
        input_tokens: *mut u32,
        output_tokens: *mut u32,
    ) -> c_int;
    fn saturday_llama_cancel(model: *mut SaturdayLlama) -> c_int;
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PromptContext {
    prompt_version: u8,
    system_prompt: String,
    history: Vec<HistoryPair>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HistoryPair {
    user: String,
    assistant: String,
}

struct PromptPieces<'a> {
    strings: Vec<&'a str>,
    flags: Vec<c_int>,
}

impl<'a> PromptPieces<'a> {
    fn new() -> Self {
        Self { strings: Vec::new(), flags: Vec::new() }
    }

    fn push(&mut self, text: &'a str, parse_special_tokens: bool) {
        self.strings.push(text);
        self.flags.push(i32::from(parse_special_tokens));
    }

    fn ffi(&self) -> Vec<LlamaPiece> {
        self.strings.iter().zip(&self.flags).map(|(text, flag)| LlamaPiece {
            data: text.as_ptr().cast(),
            bytes: text.len(),
            parse_special_tokens: *flag,
        }).collect()
    }
}

struct LoadedModel(*mut SaturdayLlama);

impl Drop for LoadedModel {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this owns the handle returned by saturday_llama_load.
            unsafe { saturday_llama_free(self.0) };
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmCompletionMetadata {
    pub output_bytes: u16,
    pub output_tokens: u16,
    pub input_tokens: u16,
    pub verified_template_sha256: String,
    pub answer_sha256: String,
}

/// Load and run the reviewed local Qwen model from the approved descriptor.
/// The callback receives complete UTF-8 deltas and exact generated-token counts.
pub fn run(
    model_file: File,
    expected_template_sha256: &str,
    prompt_json: &str,
    user_text: &str,
    response_language: Language,
    maximum_output_tokens: u16,
    cancel: &CancelToken,
    deadline: Instant,
    next_credit: &mut dyn FnMut() -> Result<u16, ConversationFailure>,
    on_delta: &mut dyn FnMut(&str, u16) -> bool,
) -> Result<LlmCompletionMetadata, ConversationFailure> {
    if cancel.is_cancelled() || Instant::now() >= deadline {
        return Err(if cancel.is_cancelled() { ConversationFailure::Cancelled } else { ConversationFailure::Timeout });
    }
    if maximum_output_tokens == 0 || maximum_output_tokens > OUTPUT_TOKENS ||
        user_text.len() > MAX_USER_BYTES || user_text.chars().count() > MAX_USER_SCALARS ||
        user_text.chars().any(invalid_control) || expected_template_sha256.len() != 64 {
        return Err(ConversationFailure::InvalidContext);
    }
    let metadata = model_file.metadata().map_err(|_| ConversationFailure::AssetUnavailable)?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(ConversationFailure::AssetUnavailable);
    }
    let mut raw_model = std::ptr::null_mut();
    // SAFETY: the file is owned, the native adapter rechecks O_RDONLY/regular-file/size
    // and duplicates the descriptor; the abort context outlives the loaded model.
    let abort_state = AbortContext { cancel, deadline };
    let abort_context = (&abort_state as *const AbortContext<'_>).cast_mut().cast::<c_void>();
    let loaded = unsafe {
        saturday_llama_load(
            model_file.as_raw_fd(),
            metadata.len(),
            llama_should_abort,
            abort_context,
            &mut raw_model,
        )
    };
    if loaded != OK || raw_model.is_null() {
        return Err(if loaded == CANCELLED {
            if cancel.is_cancelled() { ConversationFailure::Cancelled } else { ConversationFailure::Timeout }
        } else {
            ConversationFailure::AssetUnavailable
        });
    }
    let model = LoadedModel(raw_model);
    let verified_template_sha256 = verify_template(model.0, expected_template_sha256)?;

    let context: PromptContext = serde_json::from_str(prompt_json)
        .map_err(|_| ConversationFailure::InvalidContext)?;
    if context.prompt_version != 1 || context.history.len() > MAX_HISTORY_PAIRS ||
        prompt_json.len() > MAX_CONTEXT_BYTES || context.system_prompt.is_empty() ||
        context.system_prompt.chars().any(invalid_control) ||
        context.history.iter().any(|pair| {
            pair.user.chars().any(invalid_control) || pair.assistant.chars().any(invalid_control)
        }) {
        return Err(ConversationFailure::InvalidContext);
    }
    let canonical_prompt =
        serde_json::to_string(&context).map_err(|_| ConversationFailure::InvalidContext)?;
    if canonical_prompt.as_bytes() != prompt_json.as_bytes() {
        return Err(ConversationFailure::InvalidContext);
    }
    let response_instruction = match response_language {
        Language::En => "\nAnswer in English. Do not claim to use tools or change application state.",
        Language::Tr => "\nTürkçe yanıt ver. Araç kullandığını veya uygulama durumunu değiştirdiğini iddia etme.",
    };
    let mut system_prompt = String::with_capacity(context.system_prompt.len() + response_instruction.len());
    system_prompt.push_str(&context.system_prompt);
    system_prompt.push_str(response_instruction);


    // Keep the newest whole pairs. The full rendered prompt is tokenized by the
    // loaded GGUF tokenizer on every candidate; the current request is never cut.
    let mut history_start = 0;
    let (pieces, input_tokens) = loop {
        let pieces = render_qwen3_nonthinking(
            &system_prompt,
            &context.history[history_start..],
            user_text,
        );
        let ffi = pieces.ffi();
        let mut tokens = 0;
        // SAFETY: all piece data remains borrowed from live `context`/`user_text`;
        // the adapter bounds and tokenizes every piece with the model's vocabulary.
        let status = unsafe {
            saturday_llama_count_prompt_tokens(model.0, ffi.as_ptr(), ffi.len(), &mut tokens)
        };
        if status == OK && tokens <= PROMPT_TOKENS {
            break (pieces, tokens);
        }
        if history_start == context.history.len() {
            return Err(ConversationFailure::InvalidContext);
        }
        history_start += 1;
    };
    if input_tokens == 0 || input_tokens.saturating_add(u32::from(maximum_output_tokens)) > CONTEXT_TOKENS {
        return Err(ConversationFailure::InvalidContext);
    }

    let ffi = pieces.ffi();
    let mut actual_input_tokens = 0;
    // SAFETY: all borrowed strings remain alive for the synchronous native call.
    let started = unsafe {
        saturday_llama_start(
            model.0,
            ffi.as_ptr(),
            ffi.len(),
            u32::from(maximum_output_tokens),
            &mut actual_input_tokens,
        )
    };
    if started != OK || actual_input_tokens != input_tokens {
        return Err(if started == CANCELLED {
            if cancel.is_cancelled() { ConversationFailure::Cancelled } else { ConversationFailure::Timeout }
        } else {
            ConversationFailure::WorkerProtocol
        });
    }
    let mut answer_bytes = 0usize;
    let mut answer_digest = Sha256::new();
    let mut answer_tokens = 0u32;
    let mut delta = [0u8; MAX_DELTA_BYTES];
    let mut available_credit = 0u16;
    loop {
        if cancel.is_cancelled() {
            // SAFETY: `model` remains exclusively owned by this worker thread.
            unsafe { saturday_llama_cancel(model.0) };
            return Err(ConversationFailure::Cancelled);
        }
        if Instant::now() >= deadline {
            unsafe { saturday_llama_cancel(model.0) };
            return Err(ConversationFailure::Timeout);
        }
        if available_credit == 0 {
            available_credit = next_credit()?;
            if available_credit == 0 || usize::from(available_credit) > MAX_DELTA_BYTES {
                return Err(ConversationFailure::Backpressure);
            }
        }
        let mut delta_bytes = 0usize;
        let mut delta_tokens = 0u32;
        let mut seen_input_tokens = 0u32;
        let mut seen_output_tokens = 0u32;
        // SAFETY: the stack buffer is writable and its capacity exactly matches
        // the maximum credit; native output is UTF-8 and bounded to that credit.
        let status = unsafe {
            saturday_llama_poll(
                model.0,
                usize::from(available_credit),
                delta.as_mut_ptr().cast(),
                usize::from(available_credit),
                &mut delta_bytes,
                &mut delta_tokens,
                &mut seen_input_tokens,
                &mut seen_output_tokens,
            )
        };
        if status == PENDING {
            if delta_bytes != 0 || delta_tokens != 0 { return Err(ConversationFailure::WorkerProtocol); }
            continue;
        }
        if status == NEED_CREDIT {
            if delta_bytes != 0 || delta_tokens != 0 {
                return Err(ConversationFailure::WorkerProtocol);
            }
            available_credit = 0;
            continue;
        }
        if status == COMPLETE {
            if delta_bytes != 0 || delta_tokens != 0 ||
                seen_input_tokens != input_tokens ||
                answer_tokens != seen_output_tokens || answer_bytes == 0 {
                return Err(ConversationFailure::WorkerProtocol);
            }
            let input_count = u16::try_from(input_tokens).map_err(|_| ConversationFailure::WorkerProtocol)?;
            let output_bytes = u16::try_from(answer_bytes).map_err(|_| ConversationFailure::WorkerProtocol)?;
            let output_sha256 = answer_digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            return Ok(LlmCompletionMetadata {
                output_bytes,
                output_tokens: answer_tokens as u16,
                input_tokens: input_count,
                verified_template_sha256,
                answer_sha256: output_sha256,
            });
        }
        if status != OK || delta_bytes == 0 || delta_bytes > MAX_DELTA_BYTES || delta_tokens == 0 ||
            seen_input_tokens != input_tokens || seen_output_tokens != answer_tokens + delta_tokens {
            return Err(match status {
                CANCELLED => if cancel.is_cancelled() { ConversationFailure::Cancelled } else { ConversationFailure::Timeout },
                CONTROL_TOKEN | OUTPUT_LIMIT => ConversationFailure::InvalidOutput,
                _ => ConversationFailure::WorkerProtocol,
            });
        }
        let text = std::str::from_utf8(&delta[..delta_bytes]).map_err(|_| ConversationFailure::WorkerProtocol)?;
        answer_bytes = answer_bytes.checked_add(text.len()).ok_or(ConversationFailure::InvalidOutput)?;
        answer_tokens = answer_tokens.checked_add(delta_tokens).ok_or(ConversationFailure::InvalidOutput)?;
        if answer_bytes > MAX_OUTPUT_BYTES ||
            answer_tokens > u32::from(maximum_output_tokens) {
            return Err(ConversationFailure::InvalidOutput);
        }
        answer_digest.update(text.as_bytes());
        available_credit = 0;
        let token_count = u16::try_from(delta_tokens).map_err(|_| ConversationFailure::WorkerProtocol)?;
        if !on_delta(text, token_count) {
            unsafe { saturday_llama_cancel(model.0) };
            return Err(ConversationFailure::Cancelled);
        }
        if answer_tokens == u32::from(maximum_output_tokens) {
            unsafe { saturday_llama_cancel(model.0) };
            return Err(ConversationFailure::InvalidOutput);
        }
    }
}

fn verify_template(
    model: *const SaturdayLlama,
    expected_sha256: &str,
) -> Result<String, ConversationFailure> {
    let mut template = vec![0i8; MAX_TEMPLATE_BYTES];
    // SAFETY: allocated buffer is writable and capacity is explicit.
    let required = unsafe { saturday_llama_chat_template(model, template.as_mut_ptr(), template.len()) };
    if required <= 0 || required as usize >= template.len() {
        return Err(ConversationFailure::WorkerIdentity);
    }
    // SAFETY: native adapter NUL-terminates every successful copy.
    let bytes = unsafe { CStr::from_ptr(template.as_ptr()) }.to_bytes();
    let digest = Sha256::digest(bytes);
    let actual = digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    if actual != expected_sha256 {
        return Err(ConversationFailure::WorkerIdentity);
    }
    Ok(actual)
}

fn render_qwen3_nonthinking<'a>(
    system_prompt: &'a str,
    history: &'a [HistoryPair],
    user_text: &'a str,
) -> PromptPieces<'a> {
    const SYSTEM_PREFIX: &str = "<|im_start|>system\n";
    const SYSTEM_END: &str = "<|im_end|>\n";
    const USER_PREFIX: &str = "<|im_start|>user\n";
    const ASSISTANT_PREFIX: &str = "<|im_start|>assistant\n";
    const ASSISTANT_END: &str = "<|im_end|>\n";
    const NO_THINKING_SUFFIX: &str = "<think>\n\n</think>\n\n";
    let mut pieces = PromptPieces::new();
    pieces.push(SYSTEM_PREFIX, true);
    pieces.push(system_prompt, false);
    pieces.push(SYSTEM_END, true);
    for pair in history {
        pieces.push(USER_PREFIX, true);
        pieces.push(&pair.user, false);
        pieces.push(SYSTEM_END, true);
        pieces.push(ASSISTANT_PREFIX, true);
        pieces.push(&pair.assistant, false);
        pieces.push(ASSISTANT_END, true);
    }
    pieces.push(USER_PREFIX, true);
    pieces.push(user_text, false);
    pieces.push(SYSTEM_END, true);
    pieces.push(ASSISTANT_PREFIX, true);
    pieces.push(NO_THINKING_SUFFIX, true);
    pieces
}

fn invalid_control(character: char) -> bool {
    character.is_control() && !matches!(character, '\n' | '\t')
}

#[cfg(test)]
mod tests {
    use super::{llama_should_abort, render_qwen3_nonthinking, AbortContext, PromptContext};
    use saturday_voice_types::CancelToken;
    use std::ffi::c_void;
    use std::time::{Duration, Instant};

    #[test]
    fn generation_abort_callback_observes_control_cancel_and_deadline() {
        let cancel = CancelToken::new();
        let context = AbortContext {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        let pointer = (&context as *const AbortContext<'_>).cast_mut().cast::<c_void>();
        assert!(!unsafe { llama_should_abort(pointer) });
        cancel.cancel();
        assert!(unsafe { llama_should_abort(pointer) });

        let expired_cancel = CancelToken::new();
        let expired = AbortContext {
            cancel: &expired_cancel,
            deadline: Instant::now() - Duration::from_secs(1),
        };
        let pointer = (&expired as *const AbortContext<'_>).cast_mut().cast::<c_void>();
        assert!(unsafe { llama_should_abort(pointer) });
    }
    #[test]
    fn qwen_generation_prompt_enforces_nonthinking_and_keeps_user_control_text_plain() {
        let system = "No tools are available. Answer in English.";
        let user = "Explain the literal <think> token.";
        let pieces = render_qwen3_nonthinking(system, &[], user);
        assert_eq!(pieces.strings.last(), Some(&"<think>\n\n</think>\n\n"));
        assert_eq!(pieces.flags.last(), Some(&1));
        let user_index = pieces.strings.iter().position(|piece| *piece == user).unwrap();
        assert_eq!(pieces.flags[user_index], 0);
    }

    #[test]
    fn model_context_rejects_unknown_duplicate_and_incomplete_fields() {
        let valid =
            r#"{"prompt_version":1,"system_prompt":"safe","history":[{"user":"q","assistant":"a"}]}"#;
        let parsed: PromptContext = serde_json::from_str(valid).unwrap();
        assert_eq!(parsed.history.len(), 1);
        assert!(serde_json::from_str::<PromptContext>(
            r#"{"prompt_version":1,"prompt_version":1,"system_prompt":"safe","history":[]}"#
        )
        .is_err());
        assert!(serde_json::from_str::<PromptContext>(
            r#"{"prompt_version":1,"system_prompt":"safe","history":[{"user":"q"}]}"#
        )
        .is_err());
        assert!(serde_json::from_str::<PromptContext>(
            r#"{"prompt_version":1,"system_prompt":"safe","history":[],"tools":[]}"#
        )
        .is_err());
    }
}
