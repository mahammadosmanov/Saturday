use crate::voice::contract::{Language, SpeechSegment, TurnContext, VoiceOwner};
use std::ffi::{c_void, CStr};
use std::os::raw::{c_char, c_int};
use std::ptr::NonNull;

pub const MAX_VOICES: usize = 128;
pub const MAX_VOICE_ID_BYTES: usize = 512;
pub const MAX_DISPLAY_NAME_BYTES: usize = 256;
pub const MAX_LOCALE_BYTES: usize = 64;
pub const MAX_OWNER_ID_BYTES: usize = 128;
pub const MAX_SEGMENT_BYTES: usize = 1_120;
pub const MAX_SEGMENT_SCALARS: usize = 280;
pub type NativeSpeechCallback = unsafe extern "C" fn(*mut c_void, *const c_void);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeVoiceRecord {
    pub id: String,
    pub display_name: String,
    pub locale: String,
    pub language: Language,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSpeechEventKind {
    DidStart,
    DidFinish,
    Cancelled,
    Failed,
    RouteLost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSpeechEvent {
    pub owner: VoiceOwner,
    pub operation_id: u64,
    pub utterance_id: u64,
    pub kind: NativeSpeechEventKind,
    pub error_code: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSpeechError {
    Unavailable,
    QueueFull,
    VoiceUnavailable,
    WrongLocale,
    RouteLost,
    StopUnverified,
    InvalidArgument,
    BufferTooSmall,
    Failed,
}

impl NativeSpeechError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "speech_unavailable",
            Self::QueueFull => "speech_backpressure",
            Self::VoiceUnavailable => "speech_voice_unavailable",
            Self::WrongLocale => "speech_wrong_locale",
            Self::RouteLost => "speech_output_route_lost",
            Self::StopUnverified => "speech_stop_unverified",
            Self::InvalidArgument => "speech_invalid_argument",
            Self::BufferTooSmall => "speech_catalog_exceeded",
            Self::Failed => "speech_provider_failed",
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::mem::size_of;
    use std::ptr;
    use std::sync::atomic::{AtomicPtr, Ordering};

    const ABI_VERSION: u32 = 1;
    const RESULT_OK: c_int = 0;
    const RESULT_INVALID_ARGUMENT: c_int = 1;
    const RESULT_UNAVAILABLE: c_int = 2;
    const RESULT_QUEUE_FULL: c_int = 3;
    const RESULT_VOICE_UNAVAILABLE: c_int = 4;
    const RESULT_WRONG_LOCALE: c_int = 5;
    const RESULT_ROUTE_LOST: c_int = 6;
    const RESULT_STOP_UNVERIFIED: c_int = 7;
    const RESULT_INTERNAL_ERROR: c_int = 8;
    const RESULT_BUFFER_TOO_SMALL: c_int = 9;

    #[repr(C)]
    struct SpeechHandle {
        _private: [u8; 0],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VoiceRecord {
        abi_version: u32,
        struct_size: u32,
        language: u32,
        reserved: u32,
        identifier: [c_char; MAX_VOICE_ID_BYTES + 1],
        display_name: [c_char; MAX_DISPLAY_NAME_BYTES + 1],
        locale: [c_char; MAX_LOCALE_BYTES + 1],
    }

    impl Default for VoiceRecord {
        fn default() -> Self {
            Self {
                abi_version: 0,
                struct_size: 0,
                language: 0,
                reserved: 0,
                identifier: [0; MAX_VOICE_ID_BYTES + 1],
                display_name: [0; MAX_DISPLAY_NAME_BYTES + 1],
                locale: [0; MAX_LOCALE_BYTES + 1],
            }
        }
    }

    #[repr(C)]
    struct SpeechOwner {
        session_id: *const u8,
        session_id_length: usize,
        turn_id: *const u8,
        turn_id_length: usize,
        generation: u64,
        operation_id: u64,
    }

    #[repr(C)]
    struct SpeechEvent {
        abi_version: u32,
        struct_size: u32,
        kind: u32,
        error_code: u32,
        generation: u64,
        operation_id: u64,
        utterance_id: u64,
        session_id_length: u32,
        turn_id_length: u32,
        session_id: [c_char; MAX_OWNER_ID_BYTES + 1],
        turn_id: [c_char; MAX_OWNER_ID_BYTES + 1],
    }

    type SpeechCallback = NativeSpeechCallback;

    #[link(name = "saturday_voice_speech", kind = "static")]
    unsafe extern "C" {
        fn saturday_speech_create(
            callback: SpeechCallback,
            user_data: *mut c_void,
            out_handle: *mut *mut SpeechHandle,
        ) -> c_int;
        fn saturday_speech_enumerate_voices(
            records: *mut VoiceRecord,
            capacity: u32,
            out_count: *mut u32,
            out_revision: *mut u64,
        ) -> c_int;
        fn saturday_speech_start(
            handle: *mut SpeechHandle,
            owner: *const SpeechOwner,
            language: u32,
            voice_id: *const u8,
            voice_id_length: usize,
            locale: *const u8,
            locale_length: usize,
            text: *const u8,
            text_length: usize,
            utterance_id: u64,
        ) -> c_int;
        fn saturday_speech_available_credits(
            handle: *mut SpeechHandle,
            out_credits: *mut u32,
        ) -> c_int;
        fn saturday_speech_cancel(
            handle: *mut SpeechHandle,
            session_id: *const u8,
            session_id_length: usize,
            turn_id: *const u8,
            turn_id_length: usize,
            generation: u64,
        ) -> c_int;
        fn saturday_speech_is_stopped(handle: *mut SpeechHandle, out_is_stopped: *mut u32)
            -> c_int;
        fn saturday_speech_destroy(handle: *mut SpeechHandle) -> c_int;
        fn saturday_speech_normalize_nfc(
            input: *const u8,
            input_length: usize,
            output: *mut u8,
            output_capacity: usize,
            out_output_length: *mut usize,
        ) -> c_int;
    }

    pub struct NativeSpeech {
        handle: AtomicPtr<SpeechHandle>,
    }

    unsafe impl Send for NativeSpeech {}
    unsafe impl Sync for NativeSpeech {}

    impl NativeSpeech {
        /// Creates a speech handle that retains `user_data` for native callbacks.
        ///
        /// # Safety
        /// `user_data` must remain valid for every callback until this handle has been
        /// destroyed and native callback dispatch has quiesced. The callback must also
        /// safely interpret that pointer for the entire period.
        pub unsafe fn create(
            callback: SpeechCallback,
            user_data: *mut c_void,
        ) -> Result<Self, NativeSpeechError> {
            let mut handle = ptr::null_mut();
            let result = unsafe { saturday_speech_create(callback, user_data, &mut handle) };
            map_result(result)?;
            let handle = NonNull::new(handle).ok_or(NativeSpeechError::Failed)?;
            Ok(Self {
                handle: AtomicPtr::new(handle.as_ptr()),
            })
        }

        pub fn enumerate_voices() -> Result<(Vec<NativeVoiceRecord>, u64), NativeSpeechError> {
            let mut records = [VoiceRecord::default(); MAX_VOICES];
            let mut count = 0_u32;
            let mut revision = 0_u64;
            let result = unsafe {
                saturday_speech_enumerate_voices(
                    records.as_mut_ptr(),
                    MAX_VOICES as u32,
                    &mut count,
                    &mut revision,
                )
            };
            map_result(result)?;
            let count = usize::try_from(count).map_err(|_| NativeSpeechError::Failed)?;
            if count > MAX_VOICES {
                return Err(NativeSpeechError::BufferTooSmall);
            }
            let mut voices = Vec::with_capacity(count);
            for record in records.into_iter().take(count) {
                if record.abi_version != ABI_VERSION
                    || record.struct_size as usize != size_of::<VoiceRecord>()
                {
                    return Err(NativeSpeechError::Failed);
                }
                let language = match record.language {
                    1 => Language::Tr,
                    2 => Language::En,
                    _ => return Err(NativeSpeechError::Failed),
                };
                let id = read_fixed_utf8(&record.identifier)?;
                let display_name = read_fixed_utf8(&record.display_name)?;
                let locale = read_fixed_utf8(&record.locale)?;
                if id.is_empty()
                    || id.len() > MAX_VOICE_ID_BYTES
                    || display_name.is_empty()
                    || display_name.len() > MAX_DISPLAY_NAME_BYTES
                    || locale.is_empty()
                    || locale.len() > MAX_LOCALE_BYTES
                    || !locale_language_matches(&locale, language)
                {
                    return Err(NativeSpeechError::Failed);
                }
                voices.push(NativeVoiceRecord {
                    id,
                    display_name,
                    locale,
                    language,
                });
            }
            Ok((voices, revision))
        }

        pub fn start(
            &self,
            context: &TurnContext,
            segment: &SpeechSegment,
            locale: &str,
            utterance_id: u64,
        ) -> Result<(), NativeSpeechError> {
            if utterance_id == 0
                || context.owner.session_id.len() > MAX_OWNER_ID_BYTES
                || context.owner.turn_id.len() > MAX_OWNER_ID_BYTES
                || segment.voice_id.0.len() > MAX_VOICE_ID_BYTES
                || locale.len() > MAX_LOCALE_BYTES
                || segment.text.len() > MAX_SEGMENT_BYTES
                || segment.text.chars().count() > MAX_SEGMENT_SCALARS
            {
                return Err(NativeSpeechError::InvalidArgument);
            }
            let handle = self.handle()?;
            let owner = SpeechOwner {
                session_id: context.owner.session_id.as_ptr(),
                session_id_length: context.owner.session_id.len(),
                turn_id: context.owner.turn_id.as_ptr(),
                turn_id_length: context.owner.turn_id.len(),
                generation: context.owner.generation,
                operation_id: context.operation_id.0,
            };
            map_result(unsafe {
                saturday_speech_start(
                    handle.as_ptr(),
                    &owner,
                    language_code(segment.language),
                    segment.voice_id.0.as_ptr(),
                    segment.voice_id.0.len(),
                    locale.as_ptr(),
                    locale.len(),
                    segment.text.as_ptr(),
                    segment.text.len(),
                    utterance_id,
                )
            })
        }

        pub fn available_credits(&self) -> Result<u8, NativeSpeechError> {
            let handle = self.handle()?;
            let mut credits = 0_u32;
            map_result(unsafe {
                saturday_speech_available_credits(handle.as_ptr(), &mut credits)
            })?;
            u8::try_from(credits)
                .ok()
                .filter(|credits| *credits <= 3)
                .ok_or(NativeSpeechError::Failed)
        }

        pub fn cancel(&self, owner: &VoiceOwner) -> Result<(), NativeSpeechError> {
            if owner.session_id.len() > MAX_OWNER_ID_BYTES
                || owner.turn_id.len() > MAX_OWNER_ID_BYTES
            {
                return Err(NativeSpeechError::InvalidArgument);
            }
            let handle = self.handle()?;
            map_result(unsafe {
                saturday_speech_cancel(
                    handle.as_ptr(),
                    owner.session_id.as_ptr(),
                    owner.session_id.len(),
                    owner.turn_id.as_ptr(),
                    owner.turn_id.len(),
                    owner.generation,
                )
            })
        }

        pub fn is_stopped(&self) -> Result<bool, NativeSpeechError> {
            let handle = self.handle()?;
            let mut stopped = 0_u32;
            map_result(unsafe { saturday_speech_is_stopped(handle.as_ptr(), &mut stopped) })?;
            match stopped {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(NativeSpeechError::Failed),
            }
        }

        pub fn destroy(&self) -> Result<(), NativeSpeechError> {
            let handle = self.handle()?;
            map_result(unsafe { saturday_speech_destroy(handle.as_ptr()) })?;
            let _ = self.handle.compare_exchange(
                handle.as_ptr(),
                ptr::null_mut(),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            Ok(())
        }

        pub fn normalize_nfc(text: &str) -> Result<String, NativeSpeechError> {
            if text.len() > 8_192 {
                return Err(NativeSpeechError::InvalidArgument);
            }
            let mut output = vec![0_u8; 8_192];
            let mut output_length = 0_usize;
            map_result(unsafe {
                saturday_speech_normalize_nfc(
                    text.as_ptr(),
                    text.len(),
                    output.as_mut_ptr(),
                    output.len(),
                    &mut output_length,
                )
            })?;
            output.truncate(output_length);
            String::from_utf8(output).map_err(|_| NativeSpeechError::Failed)
        }

        fn handle(&self) -> Result<NonNull<SpeechHandle>, NativeSpeechError> {
            NonNull::new(self.handle.load(Ordering::Acquire)).ok_or(NativeSpeechError::Unavailable)
        }
    }

    impl Drop for NativeSpeech {
        fn drop(&mut self) {
            if self.handle.load(Ordering::Acquire).is_null() {
                return;
            }
            let _ = self.destroy();
        }
    }

    fn read_fixed_utf8<const N: usize>(bytes: &[c_char; N]) -> Result<String, NativeSpeechError> {
        let value = unsafe { CStr::from_ptr(bytes.as_ptr()) };
        value
            .to_str()
            .map(str::to_owned)
            .map_err(|_| NativeSpeechError::Failed)
    }

    fn locale_language_matches(locale: &str, language: Language) -> bool {
        match language {
            Language::Tr => locale
                .split('-')
                .next()
                .is_some_and(|part| part.eq_ignore_ascii_case("tr")),
            Language::En => locale
                .split('-')
                .next()
                .is_some_and(|part| part.eq_ignore_ascii_case("en")),
        }
    }

    fn language_code(language: Language) -> u32 {
        match language {
            Language::Tr => 1,
            Language::En => 2,
        }
    }

    fn map_result(result: c_int) -> Result<(), NativeSpeechError> {
        match result {
            RESULT_OK => Ok(()),
            RESULT_INVALID_ARGUMENT => Err(NativeSpeechError::InvalidArgument),
            RESULT_UNAVAILABLE => Err(NativeSpeechError::Unavailable),
            RESULT_QUEUE_FULL => Err(NativeSpeechError::QueueFull),
            RESULT_VOICE_UNAVAILABLE => Err(NativeSpeechError::VoiceUnavailable),
            RESULT_WRONG_LOCALE => Err(NativeSpeechError::WrongLocale),
            RESULT_ROUTE_LOST => Err(NativeSpeechError::RouteLost),
            RESULT_STOP_UNVERIFIED => Err(NativeSpeechError::StopUnverified),
            RESULT_BUFFER_TOO_SMALL => Err(NativeSpeechError::BufferTooSmall),
            RESULT_INTERNAL_ERROR => Err(NativeSpeechError::Failed),
            _ => Err(NativeSpeechError::Failed),
        }
    }

    /// Copies an event supplied by the native speech callback.
    ///
    /// # Safety
    /// `event` must be non-null, properly aligned, and point to a readable `SpeechEvent`
    /// allocation for the duration of this call. Its fixed-size fields must be initialized
    /// and its declared lengths must not exceed their corresponding arrays.
    pub unsafe fn copy_event(event: *const c_void) -> Result<NativeSpeechEvent, NativeSpeechError> {
        if event.is_null() {
            return Err(NativeSpeechError::InvalidArgument);
        }
        let event = &*(event.cast::<SpeechEvent>());
        if event.abi_version != ABI_VERSION
            || event.struct_size as usize != size_of::<SpeechEvent>()
            || event.session_id_length == 0
            || event.session_id_length as usize > MAX_OWNER_ID_BYTES
            || event.turn_id_length == 0
            || event.turn_id_length as usize > MAX_OWNER_ID_BYTES
            || event.generation == 0
            || event.operation_id == 0
            || event.utterance_id == 0
        {
            return Err(NativeSpeechError::InvalidArgument);
        }
        let session_bytes = std::slice::from_raw_parts(
            event.session_id.as_ptr().cast::<u8>(),
            event.session_id_length as usize,
        );
        let turn_bytes = std::slice::from_raw_parts(
            event.turn_id.as_ptr().cast::<u8>(),
            event.turn_id_length as usize,
        );
        let session_id =
            std::str::from_utf8(session_bytes).map_err(|_| NativeSpeechError::InvalidArgument)?;
        let turn_id =
            std::str::from_utf8(turn_bytes).map_err(|_| NativeSpeechError::InvalidArgument)?;
        if session_id.contains('\0') || turn_id.contains('\0') {
            return Err(NativeSpeechError::InvalidArgument);
        }
        let kind = match event.kind {
            1 => NativeSpeechEventKind::DidStart,
            2 => NativeSpeechEventKind::DidFinish,
            3 => NativeSpeechEventKind::Cancelled,
            4 => NativeSpeechEventKind::Failed,
            5 => NativeSpeechEventKind::RouteLost,
            _ => return Err(NativeSpeechError::InvalidArgument),
        };
        Ok(NativeSpeechEvent {
            owner: VoiceOwner {
                session_id: session_id.to_owned(),
                turn_id: turn_id.to_owned(),
                generation: event.generation,
            },
            operation_id: event.operation_id,
            utterance_id: event.utterance_id,
            kind,
            error_code: event.error_code,
        })
    }
}

#[cfg(target_os = "macos")]
pub use macos::{copy_event as copy_native_event, NativeSpeech};

#[cfg(not(target_os = "macos"))]
pub struct NativeSpeech;

#[cfg(not(target_os = "macos"))]
impl NativeSpeech {
    /// # Safety
    /// `user_data` must remain valid for every native callback until this handle has been
    /// destroyed and callback dispatch has quiesced; the callback must safely interpret it.
    pub unsafe fn create(
        _callback: NativeSpeechCallback,
        _user_data: *mut c_void,
    ) -> Result<Self, NativeSpeechError> {
        Err(NativeSpeechError::Unavailable)
    }

    pub fn enumerate_voices() -> Result<(Vec<NativeVoiceRecord>, u64), NativeSpeechError> {
        Err(NativeSpeechError::Unavailable)
    }

    pub fn start(
        &self,
        _context: &TurnContext,
        _segment: &SpeechSegment,
        _locale: &str,
        _utterance_id: u64,
    ) -> Result<(), NativeSpeechError> {
        Err(NativeSpeechError::Unavailable)
    }

    pub fn available_credits(&self) -> Result<u8, NativeSpeechError> {
        Ok(0)
    }

    pub fn cancel(&self, _owner: &VoiceOwner) -> Result<(), NativeSpeechError> {
        Err(NativeSpeechError::Unavailable)
    }

    pub fn is_stopped(&self) -> Result<bool, NativeSpeechError> {
        Ok(true)
    }

    pub fn destroy(&self) -> Result<(), NativeSpeechError> {
        Ok(())
    }

    pub fn normalize_nfc(_text: &str) -> Result<String, NativeSpeechError> {
        Err(NativeSpeechError::Unavailable)
    }
}

#[cfg(not(target_os = "macos"))]
/// # Safety
/// `event` must be a readable native speech event allocation for the duration of this call.
/// It is unused on unsupported platforms.
pub unsafe fn copy_native_event(
    _event: *const c_void,
) -> Result<NativeSpeechEvent, NativeSpeechError> {
    Err(NativeSpeechError::Unavailable)
}
