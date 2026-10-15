use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use saturday_voice_types::{
    AsrError, AsrLanguage, AsrQualityFlag, AsrResult, CancelToken, LanguageEvidence,
    LanguageMode, TranscriptEligibility,
};

const MAX_SAMPLES: u32 = 480_000;
const MAX_PCM_BYTES: usize = MAX_SAMPLES as usize * std::mem::size_of::<f32>();
const MAX_TEXT_BYTES: usize = 8_000;
const MAX_TEXT_SCALARS: usize = 2_000;
const MAX_LANGUAGES: usize = 128;
const MAX_LANGUAGE_CODE_BYTES: usize = 7;
const MAX_QUALITY_FLAGS: usize = 8;
const PCM_POLL_MILLIS: i32 = 20;

const STATUS_OK: i32 = 0;
const STATUS_INVALID_ARGUMENT: i32 = 1;
const STATUS_INVALID_MODEL: i32 = 2;
const STATUS_UNSUPPORTED_MODEL: i32 = 3;
const STATUS_CANCELLED: i32 = 4;
const STATUS_FAILED: i32 = 5;
const FLAG_NO_SPEECH: u32 = 1 << 0;
const FLAG_LOW_SIGNAL: u32 = 1 << 1;
const FLAG_UNSUPPORTED_LANGUAGE: u32 = 1 << 2;
const FLAG_MIXED_LANGUAGE: u32 = 1 << 3;
const FLAG_INCOMPLETE: u32 = 1 << 4;
const FLAG_OVERLONG: u32 = 1 << 5;
const KNOWN_FLAGS: u32 = FLAG_NO_SPEECH
    | FLAG_LOW_SIGNAL
    | FLAG_UNSUPPORTED_LANGUAGE
    | FLAG_MIXED_LANGUAGE
    | FLAG_INCOMPLETE
    | FLAG_OVERLONG;

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeLanguageEvidence {
    code: [libc::c_char; 8],
    score: f32,
    score_gap: f32,
}

impl Default for NativeLanguageEvidence {
    fn default() -> Self {
        Self {
            code: [0; 8],
            score: 0.0,
            score_gap: 0.0,
        }
    }
}

#[repr(C)]
struct NativeWhisperResult {
    status: i32,
    text_length: u32,
    evidence_count: u32,
    quality_flags: u32,
    has_no_speech_score: u8,
    reserved: [u8; 3],
    no_speech_score: f32,
    text: [u8; MAX_TEXT_BYTES],
    languages: [NativeLanguageEvidence; MAX_LANGUAGES],
}

impl Default for NativeWhisperResult {
    fn default() -> Self {
        Self {
            status: STATUS_INVALID_ARGUMENT,
            text_length: 0,
            evidence_count: 0,
            quality_flags: 0,
            has_no_speech_score: 0,
            reserved: [0; 3],
            no_speech_score: 0.0,
            text: [0; MAX_TEXT_BYTES],
            languages: [NativeLanguageEvidence::default(); MAX_LANGUAGES],
        }
    }
}

impl Drop for NativeWhisperResult {
    fn drop(&mut self) {
        zero_native_result(self);
    }
}

type AbortCallback = unsafe extern "C" fn(*mut c_void) -> i32;

extern "C" {
    fn saturday_whisper_transcribe_fd(
        model_fd: i32,
        samples: *const f32,
        sample_count: u32,
        language_mode: u32,
        should_abort: AbortCallback,
        abort_context: *mut c_void,
        result: *mut NativeWhisperResult,
    ) -> i32;
}

struct AbortContext<'a> {
    cancel: &'a CancelToken,
    deadline: Instant,
}

unsafe extern "C" fn should_abort(context: *mut c_void) -> i32 {
    if context.is_null() {
        return 1;
    }
    let context = &*(context.cast::<AbortContext<'_>>());
    i32::from(context.cancel.is_cancelled() || Instant::now() >= context.deadline)
}

pub fn run_asr_worker(
    model: File,
    pcm: File,
    sample_count: u32,
    language_mode: LanguageMode,
    cancel: &CancelToken,
    deadline: Instant,
) -> Result<AsrResult, AsrError> {
    if cancel.is_cancelled() {
        return Err(AsrError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(AsrError::Timeout);
    }
    let mut samples = read_pcm_fifo(pcm, sample_count, cancel, deadline)?;
    if cancel.is_cancelled() {
        zero_samples(&mut samples);
        return Err(AsrError::Cancelled);
    }
    if Instant::now() >= deadline {
        zero_samples(&mut samples);
        return Err(AsrError::Timeout);
    }
    if let Err(error) = validate_model_fd(&model) {
        zero_samples(&mut samples);
        return Err(error);
    }
    let mut result = NativeWhisperResult::default();
    let abort_context = AbortContext { cancel, deadline };
    let status = unsafe {
        saturday_whisper_transcribe_fd(
            model.as_raw_fd(),
            samples.as_ptr(),
            sample_count,
            language_mode as u32,
            should_abort,
            (&abort_context as *const AbortContext<'_>).cast_mut().cast(),
            &mut result,
        )
    };
    zero_samples(&mut samples);
    if cancel.is_cancelled() {
        zero_native_result(&mut result);
        return Err(AsrError::Cancelled);
    }
    if Instant::now() >= deadline {
        zero_native_result(&mut result);
        return Err(AsrError::Timeout);
    }
    if status != result.status {
        zero_native_result(&mut result);
        return Err(AsrError::WorkerFailed);
    }
    match status {
        STATUS_OK => parse_result(result, language_mode),
        STATUS_INVALID_MODEL => Err(AsrError::AssetUnavailable),
        STATUS_UNSUPPORTED_MODEL => Err(AsrError::AssetUnavailable),
        STATUS_CANCELLED => Err(if cancel.is_cancelled() {
            AsrError::Cancelled
        } else {
            AsrError::Timeout
        }),
        STATUS_INVALID_ARGUMENT => Err(AsrError::InvalidAudio),
        STATUS_FAILED => Err(AsrError::WorkerFailed),
        _ => Err(AsrError::InvalidEvidence),
    }
}

fn read_pcm_fifo(
    pcm: File,
    sample_count: u32,
    cancel: &CancelToken,
    deadline: Instant,
) -> Result<Vec<f32>, AsrError> {
    let fd = pcm.as_raw_fd();
    if sample_count == 0 || sample_count > MAX_SAMPLES {
        return Err(AsrError::InvalidAudio);
    }
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe { libc::fstat(fd, &mut metadata) } != 0
        || metadata.st_mode & libc::S_IFMT != libc::S_IFIFO
        || metadata.st_uid != unsafe { libc::geteuid() }
    {
        return Err(AsrError::InvalidAudio);
    }
    let access = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    let descriptor_flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if access < 0
        || access & libc::O_ACCMODE != libc::O_RDONLY
        || descriptor_flags < 0
        || descriptor_flags & libc::FD_CLOEXEC == 0
        || unsafe { libc::fcntl(fd, libc::F_SETFL, access | libc::O_NONBLOCK) } < 0
    {
        return Err(AsrError::InvalidAudio);
    }

    let byte_count = usize::try_from(sample_count)
        .ok()
        .and_then(|count| count.checked_mul(std::mem::size_of::<f32>()))
        .filter(|count| *count <= MAX_PCM_BYTES)
        .ok_or(AsrError::InvalidAudio)?;
    let mut samples = Vec::new();
    samples.try_reserve_exact(sample_count as usize).map_err(|_| AsrError::Unavailable)?;
    samples.resize(sample_count as usize, 0.0f32);
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(samples.as_mut_ptr().cast::<u8>(), byte_count)
    };
    let mut offset = 0usize;
    while offset < byte_count {
        if let Err(error) = ensure_live(cancel, deadline) {
            zero_samples(&mut samples);
            return Err(error);
        }
        if let Err(error) = wait_readable(fd, cancel, deadline) {
            zero_samples(&mut samples);
            return Err(error);
        }
        let amount = unsafe {
            libc::read(
                fd,
                bytes[offset..].as_mut_ptr().cast(),
                (byte_count - offset).min(64 * 1024),
            )
        };
        if amount > 0 {
            offset += amount as usize;
        } else if amount == 0 {
            zero_samples(&mut samples);
            return Err(AsrError::InvalidAudio);
        } else {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::WouldBlock && error.kind() != io::ErrorKind::Interrupted {
                zero_samples(&mut samples);
                return Err(AsrError::InvalidAudio);
            }
        }
    }

    loop {
        if let Err(error) = ensure_live(cancel, deadline) {
            zero_samples(&mut samples);
            return Err(error);
        }
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };
        let timeout = remaining_poll_millis(deadline);
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            zero_samples(&mut samples);
            return Err(AsrError::InvalidAudio);
        }
        if ready == 0 {
            if let Err(error) = ensure_live(cancel, deadline) {
                zero_samples(&mut samples);
                return Err(error);
            }
            continue;
        }
        let mut extra = 0u8;
        let amount = unsafe { libc::read(fd, (&mut extra as *mut u8).cast(), 1) };
        if amount == 0 {
            break;
        }
        if amount > 0 {
            zero_samples(&mut samples);
            return Err(AsrError::InvalidAudio);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::WouldBlock && error.kind() != io::ErrorKind::Interrupted {
            zero_samples(&mut samples);
            return Err(AsrError::InvalidAudio);
        }
    }

    if samples
        .iter()
        .any(|sample| !sample.is_finite() || !(-1.0..=1.0).contains(sample))
    {
        zero_samples(&mut samples);
        return Err(AsrError::InvalidAudio);
    }
    Ok(samples)
}

fn wait_readable(fd: i32, cancel: &CancelToken, deadline: Instant) -> Result<(), AsrError> {
    loop {
        ensure_live(cancel, deadline)?;
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, remaining_poll_millis(deadline)) };
        if ready > 0 {
            return Ok(());
        }
        if ready == 0 {
            continue;
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(AsrError::InvalidAudio);
        }
    }
}

fn ensure_live(cancel: &CancelToken, deadline: Instant) -> Result<(), AsrError> {
    if cancel.is_cancelled() {
        Err(AsrError::Cancelled)
    } else if Instant::now() >= deadline {
        Err(AsrError::Timeout)
    } else {
        Ok(())
    }
}

fn remaining_poll_millis(deadline: Instant) -> i32 {
    deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_millis(PCM_POLL_MILLIS as u64))
        .as_millis()
        .max(1) as i32
}

fn validate_model_fd(model: &File) -> Result<(), AsrError> {
    let fd = model.as_raw_fd();
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    let access = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    let descriptor_flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if unsafe { libc::fstat(fd, &mut metadata) } != 0
        || metadata.st_mode & libc::S_IFMT != libc::S_IFREG
        || metadata.st_size <= 0
        || access < 0
        || access & libc::O_ACCMODE != libc::O_RDONLY
        || descriptor_flags < 0
        || descriptor_flags & libc::FD_CLOEXEC == 0
    {
        return Err(AsrError::AssetUnavailable);
    }
    Ok(())
}

fn parse_result(mut raw: NativeWhisperResult, mode: LanguageMode) -> Result<AsrResult, AsrError> {
    if raw.evidence_count as usize > MAX_LANGUAGES
        || raw.quality_flags & !KNOWN_FLAGS != 0
        || raw.has_no_speech_score > 1
        || raw.reserved != [0; 3]
        || raw.text_length as usize > MAX_TEXT_BYTES
    {
        zero_native_result(&mut raw);
        return Err(AsrError::InvalidEvidence);
    }
    let length = raw.text_length as usize;
    let bytes = &raw.text[..length];
    let (valid_text, malformed_tail) = match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), false),
        Err(error) if error.error_len().is_none() => {
            let text = std::str::from_utf8(&bytes[..error.valid_up_to()])
                .map_err(|_| AsrError::InvalidEvidence)?
                .to_owned();
            (text, true)
        }
        Err(_) => {
            zero_native_result(&mut raw);
            return Err(AsrError::InvalidEvidence);
        }
    };
    let mut text = valid_text;
    let mut flags = quality_flags(raw.quality_flags);
    if malformed_tail {
        flags.push(AsrQualityFlag::Overlong);
    }
    if text.len() > MAX_TEXT_BYTES || text.chars().count() > MAX_TEXT_SCALARS {
        text = truncate_text(text, MAX_TEXT_BYTES, MAX_TEXT_SCALARS);
        flags.push(AsrQualityFlag::Overlong);
    }
    let has_no_speech_score = raw.has_no_speech_score != 0;
    if (has_no_speech_score
        && (!raw.no_speech_score.is_finite()
            || !(0.0..=1.0).contains(&raw.no_speech_score)))
        || (!has_no_speech_score && raw.no_speech_score != 0.0)
    {
        zero_native_result(&mut raw);
        return Err(AsrError::InvalidEvidence);
    }
    let no_speech_score = has_no_speech_score.then_some(raw.no_speech_score);
    let no_speech = raw.quality_flags & FLAG_NO_SPEECH != 0;
    if text.trim().is_empty() && !no_speech {
        zero_native_result(&mut raw);
        return Err(AsrError::InvalidEvidence);
    }
    let ranked_languages = parse_languages(&raw.languages[..raw.evidence_count as usize], mode)?;
    if mode == LanguageMode::Auto {
        push_flag(&mut flags, AsrQualityFlag::UncalibratedAuto);
    }
    let negation_detected = (mode == LanguageMode::Tr && contains_negation(&text, true))
        || (mode == LanguageMode::En && contains_negation(&text, false));
    flags.sort_by_key(|flag| *flag as u8);
    flags.dedup();
    if flags.len() > MAX_QUALITY_FLAGS {
        zero_native_result(&mut raw);
        return Err(AsrError::InvalidEvidence);
    }

    let needs_review = mode == LanguageMode::Auto || !flags.is_empty() || negation_detected;
    let eligibility = if no_speech {
        TranscriptEligibility::NoSpeech
    } else if needs_review {
        TranscriptEligibility::NeedsReview
    } else {
        TranscriptEligibility::Eligible
    };
    zero_native_result(&mut raw);
    Ok(AsrResult {
        text,
        ranked_languages,
        no_speech_score,
        quality_flags: flags,
        eligibility,
    })
}

fn parse_languages(
    candidates: &[NativeLanguageEvidence],
    mode: LanguageMode,
) -> Result<Vec<LanguageEvidence>, AsrError> {
    if mode != LanguageMode::Auto {
        return if candidates.is_empty() {
            Ok(Vec::new())
        } else {
            Err(AsrError::InvalidEvidence)
        };
    }
    if candidates.is_empty() || candidates.len() > MAX_LANGUAGES {
        return Err(AsrError::InvalidEvidence);
    }
    let mut output = Vec::with_capacity(candidates.len());
    let mut previous_score = f32::INFINITY;
    for (index, item) in candidates.iter().enumerate() {
        if !item.score.is_finite()
            || !item.score_gap.is_finite()
            || !(0.0..=1.0).contains(&item.score)
            || !(0.0..=1.0).contains(&item.score_gap)
            || item.score > previous_score + 1e-6
        {
            return Err(AsrError::InvalidEvidence);
        }
        let code_end = item.code.iter().position(|byte| *byte == 0).unwrap_or(item.code.len());
        if code_end == 0 || code_end > MAX_LANGUAGE_CODE_BYTES
            || item.code[code_end..].iter().any(|byte| *byte != 0)
        {
            return Err(AsrError::InvalidEvidence);
        }
        let mut code_bytes = [0u8; 8];
        for (output, input) in code_bytes.iter_mut().zip(&item.code[..code_end]) {
            *output = *input as u8;
        }
        let code = std::str::from_utf8(&code_bytes[..code_end])
            .map_err(|_| AsrError::InvalidEvidence)?;
        if !code.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()) {
            return Err(AsrError::InvalidEvidence);
        }
        let next_score = candidates.get(index + 1).map_or(0.0, |next| next.score);
        if (item.score_gap - (item.score - next_score)).abs() > 1e-5 {
            return Err(AsrError::InvalidEvidence);
        }
        output.push(LanguageEvidence {
            language: match code {
                "tr" => AsrLanguage::Tr,
                "en" => AsrLanguage::En,
                other => AsrLanguage::Other(other.to_owned()),
            },
            rank: u8::try_from(index + 1).map_err(|_| AsrError::InvalidEvidence)?,
            score: item.score,
            score_gap: item.score_gap,
        });
        previous_score = item.score;
    }
    Ok(output)
}

fn quality_flags(bits: u32) -> Vec<AsrQualityFlag> {
    let mut flags = Vec::with_capacity(6);
    for (bit, flag) in [
        (FLAG_NO_SPEECH, AsrQualityFlag::NoSpeech),
        (FLAG_LOW_SIGNAL, AsrQualityFlag::LowSignal),
        (FLAG_UNSUPPORTED_LANGUAGE, AsrQualityFlag::UnsupportedLanguage),
        (FLAG_MIXED_LANGUAGE, AsrQualityFlag::MixedLanguage),
        (FLAG_INCOMPLETE, AsrQualityFlag::Incomplete),
        (FLAG_OVERLONG, AsrQualityFlag::Overlong),
    ] {
        if bits & bit != 0 {
            flags.push(flag);
        }
    }
    flags
}

fn push_flag(flags: &mut Vec<AsrQualityFlag>, flag: AsrQualityFlag) {
    if !flags.contains(&flag) {
        flags.push(flag);
    }
}

fn truncate_text(text: String, byte_limit: usize, scalar_limit: usize) -> String {
    let mut end = text.len().min(byte_limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut count = 0;
    for (index, character) in text[..end].char_indices() {
        if count == scalar_limit {
            end = index;
            break;
        }
        count += 1;
        if count == scalar_limit {
            end = index + character.len_utf8();
        }
    }
    text[..end].to_owned()
}

fn contains_negation(text: &str, turkish: bool) -> bool {
    let markers: &[&str] = if turkish {
        &["değil", "yapma", "yapmayın", "etme", "etmeyin", "hayır", "asla", "hiç", "yok", "olmasın"]
    } else {
        &["no", "not", "never", "dont", "don't", "cannot", "cant", "can't", "without", "nothing", "neither", "nor"]
    };
    let mut word = [char::default(); 16];
    let mut length = 0usize;
    let mut too_long = false;
    let mut found = false;
    for character in text.chars().chain(std::iter::once(' ')) {
        let mut lowered = match (turkish, character) {
            (true, 'I') => 'ı'.to_lowercase(),
            (true, 'İ') => 'i'.to_lowercase(),
            (_, other) => other.to_lowercase(),
        };
        for normalized in &mut lowered {
            if normalized.is_alphanumeric() || normalized == '\'' {
                if length < word.len() {
                    word[length] = normalized;
                    length += 1;
                } else {
                    too_long = true;
                }
            } else if length > 0 || too_long {
                if !too_long
                    && markers.iter().any(|marker| {
                        marker.chars().eq(word[..length].iter().copied())
                    })
                {
                    found = true;
                    break;
                }
                length = 0;
                too_long = false;
            }
        }
        if found {
            return true;
        }
    }
    false
}

fn zero_samples(samples: &mut [f32]) {
    for sample in samples {
        unsafe { std::ptr::write_volatile(sample, 0.0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

fn zero_native_result(result: &mut NativeWhisperResult) {
    for byte in result.text.iter_mut() {
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    for candidate in result.languages.iter_mut() {
        for byte in candidate.code.iter_mut() {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        unsafe {
            std::ptr::write_volatile(&mut candidate.score, 0.0);
            std::ptr::write_volatile(&mut candidate.score_gap, 0.0);
        }
    }
    unsafe { std::ptr::write_volatile(&mut result.no_speech_score, 0.0) };
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::FromRawFd;

    fn pipe_with_payload(payload: &[u8]) -> File {
        let mut descriptors = [-1; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let reader = unsafe { File::from_raw_fd(descriptors[0]) };
        let mut writer = unsafe { File::from_raw_fd(descriptors[1]) };
        writer.write_all(payload).expect("write pipe");
        drop(writer);
        reader
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(1)
    }

    #[test]
    fn pcm_pipe_requires_exact_length_eof_and_valid_samples() {
        let one = 0.125f32.to_ne_bytes();
        assert_eq!(
            read_pcm_fifo(pipe_with_payload(&one), 2, &CancelToken::new(), deadline()),
            Err(AsrError::InvalidAudio)
        );

        let mut extra = Vec::from(one);
        extra.extend_from_slice(&0.25f32.to_ne_bytes());
        assert_eq!(
            read_pcm_fifo(pipe_with_payload(&extra), 1, &CancelToken::new(), deadline()),
            Err(AsrError::InvalidAudio)
        );

        for sample in [f32::NAN, f32::INFINITY, -1.1, 1.1] {
            assert_eq!(
                read_pcm_fifo(
                    pipe_with_payload(&sample.to_ne_bytes()),
                    1,
                    &CancelToken::new(),
                    deadline()
                ),
                Err(AsrError::InvalidAudio)
            );
        }
    }

    #[test]
    fn pcm_pipe_rejects_non_fifo_and_writable_descriptors() {
        assert_eq!(
            read_pcm_fifo(
                File::open("/dev/null").expect("open non-FIFO"),
                1,
                &CancelToken::new(),
                deadline()
            ),
            Err(AsrError::InvalidAudio)
        );
        let mut descriptors = [-1; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let writable = unsafe { File::from_raw_fd(descriptors[1]) };
        drop(unsafe { File::from_raw_fd(descriptors[0]) });
        assert_eq!(
            read_pcm_fifo(writable, 1, &CancelToken::new(), deadline()),
            Err(AsrError::InvalidAudio)
        );
    }

    #[test]
    fn cancelled_pcm_transfer_is_rejected_before_inference() {
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(
            read_pcm_fifo(
                pipe_with_payload(&0.0f32.to_ne_bytes()),
                1,
                &cancel,
                deadline()
            ),
            Err(AsrError::Cancelled)
        );
    }

    #[test]
    fn model_descriptor_must_be_a_read_only_regular_file() {
        assert_eq!(
            validate_model_fd(&File::open("/dev/null").expect("open non-regular file")),
            Err(AsrError::AssetUnavailable)
        );
    }


    #[test]
    fn malformed_language_evidence_fails_closed() {
        let mut candidate = NativeLanguageEvidence::default();
        for (slot, byte) in candidate.code.iter_mut().zip(b"tr\0") {
            *slot = *byte as libc::c_char;
        }
        candidate.score = f32::NAN;
        assert_eq!(parse_languages(&[candidate], LanguageMode::Auto), Err(AsrError::InvalidEvidence));
    }

    #[test]
    fn native_quality_and_optional_no_speech_evidence_are_preserved() {
        let mut raw = NativeWhisperResult::default();
        raw.status = STATUS_OK;
        raw.quality_flags = FLAG_NO_SPEECH | FLAG_LOW_SIGNAL;
        raw.has_no_speech_score = 1;
        raw.no_speech_score = 0.875;
        let result = parse_result(raw, LanguageMode::En).expect("valid native quality evidence");
        assert_eq!(result.no_speech_score, Some(0.875));
        assert_eq!(result.eligibility, TranscriptEligibility::NoSpeech);
        assert!(result.quality_flags.contains(&AsrQualityFlag::NoSpeech));
        assert!(result.quality_flags.contains(&AsrQualityFlag::LowSignal));

        let mut missing = NativeWhisperResult::default();
        missing.status = STATUS_OK;
        missing.quality_flags = FLAG_NO_SPEECH;
        let result =
            parse_result(missing, LanguageMode::En).expect("native may have no score for no segments");
        assert_eq!(result.no_speech_score, None);
        assert_eq!(result.eligibility, TranscriptEligibility::NoSpeech);
    }

    #[test]
    fn malformed_or_inconsistent_native_no_speech_evidence_is_rejected() {
        for score in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
            let mut raw = NativeWhisperResult::default();
            raw.status = STATUS_OK;
            raw.has_no_speech_score = 1;
            raw.no_speech_score = score;
            assert_eq!(
                parse_result(raw, LanguageMode::En),
                Err(AsrError::InvalidEvidence)
            );
        }

        let mut absent_but_nonzero = NativeWhisperResult::default();
        absent_but_nonzero.status = STATUS_OK;
        absent_but_nonzero.no_speech_score = 0.5;
        assert_eq!(
            parse_result(absent_but_nonzero, LanguageMode::En),
            Err(AsrError::InvalidEvidence)
        );

        let mut empty_without_no_speech_flag = NativeWhisperResult::default();
        empty_without_no_speech_flag.status = STATUS_OK;
        assert_eq!(
            parse_result(empty_without_no_speech_flag, LanguageMode::En),
            Err(AsrError::InvalidEvidence)
        );
    }

    #[test]
    fn native_no_speech_threshold_is_not_reconstructed_from_score() {
        let mut raw = NativeWhisperResult::default();
        raw.status = STATUS_OK;
        raw.has_no_speech_score = 1;
        raw.text[..5].copy_from_slice(b"hello");
        raw.text_length = 5;
        raw.no_speech_score = 0.875;
        let result = parse_result(raw, LanguageMode::En).expect("valid native score");
        assert_eq!(result.no_speech_score, Some(0.875));
        assert_eq!(result.eligibility, TranscriptEligibility::Eligible);
        assert!(!result.quality_flags.contains(&AsrQualityFlag::NoSpeech));
    }

    #[test]
    fn silence_mixed_unsupported_and_capped_results_are_review_blocked() {
        let mut raw = NativeWhisperResult::default();
        raw.status = STATUS_OK;
        raw.quality_flags = FLAG_NO_SPEECH | FLAG_UNSUPPORTED_LANGUAGE | FLAG_MIXED_LANGUAGE;
        let result = parse_result(raw, LanguageMode::Auto).expect("well-formed review evidence");
        assert_eq!(result.eligibility, TranscriptEligibility::NoSpeech);
        assert!(result.quality_flags.contains(&AsrQualityFlag::UncalibratedAuto));
        assert!(result.quality_flags.contains(&AsrQualityFlag::UnsupportedLanguage));
    }

    #[test]
    fn explicit_negation_and_duration_capped_text_never_becomes_eligible() {
        let mut raw = NativeWhisperResult::default();
        raw.status = STATUS_OK;
        raw.text[..9].copy_from_slice(b"do not go");
        raw.text_length = 9;
        let mut result = parse_result(raw, LanguageMode::En).expect("reviewable transcript");
        assert_eq!(result.eligibility, TranscriptEligibility::NeedsReview);
        push_flag(&mut result.quality_flags, AsrQualityFlag::Truncated);
        result.eligibility = TranscriptEligibility::NeedsReview;
        assert!(result.quality_flags.contains(&AsrQualityFlag::Truncated));
    }

    #[test]
    fn transcript_preview_obeys_scalar_and_utf8_bounds() {
        let text = "ş".repeat(MAX_TEXT_SCALARS + 1);
        let preview = truncate_text(text, MAX_TEXT_BYTES, MAX_TEXT_SCALARS);
        assert_eq!(preview.chars().count(), MAX_TEXT_SCALARS);
        assert!(preview.len() <= MAX_TEXT_BYTES);
    }
}
