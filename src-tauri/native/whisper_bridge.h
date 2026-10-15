#ifndef SATURDAY_WHISPER_BRIDGE_H
#define SATURDAY_WHISPER_BRIDGE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum SaturdayWhisperLanguageMode {
    SATURDAY_WHISPER_LANGUAGE_TR = 0,
    SATURDAY_WHISPER_LANGUAGE_EN = 1,
    SATURDAY_WHISPER_LANGUAGE_AUTO = 2,
};

enum SaturdayWhisperStatus {
    SATURDAY_WHISPER_OK = 0,
    SATURDAY_WHISPER_INVALID_ARGUMENT = 1,
    SATURDAY_WHISPER_INVALID_MODEL = 2,
    SATURDAY_WHISPER_UNSUPPORTED_MODEL = 3,
    SATURDAY_WHISPER_CANCELLED = 4,
    SATURDAY_WHISPER_FAILED = 5,
};

enum SaturdayWhisperQualityFlag {
    SATURDAY_WHISPER_FLAG_NO_SPEECH = 1u << 0,
    SATURDAY_WHISPER_FLAG_LOW_SIGNAL = 1u << 1,
    SATURDAY_WHISPER_FLAG_UNSUPPORTED_LANGUAGE = 1u << 2,
    SATURDAY_WHISPER_FLAG_MIXED_LANGUAGE = 1u << 3,
    SATURDAY_WHISPER_FLAG_INCOMPLETE = 1u << 4,
    SATURDAY_WHISPER_FLAG_OVERLONG = 1u << 5,
};

#define SATURDAY_WHISPER_MAX_LANGUAGES 128
#define SATURDAY_WHISPER_LANGUAGE_CODE_BYTES 8
#define SATURDAY_WHISPER_MAX_TEXT_BYTES 8000

typedef int (*SaturdayWhisperAbortCallback)(void *context);

typedef struct SaturdayWhisperLanguageEvidence {
    char code[SATURDAY_WHISPER_LANGUAGE_CODE_BYTES];
    float score;
    float score_gap;
} SaturdayWhisperLanguageEvidence;

typedef struct SaturdayWhisperResult {
    int32_t status;
    uint32_t text_length;
    uint32_t evidence_count;
    uint32_t quality_flags;
    uint8_t has_no_speech_score;
    uint8_t reserved[3];
    float no_speech_score;
    char text[SATURDAY_WHISPER_MAX_TEXT_BYTES];
    SaturdayWhisperLanguageEvidence languages[SATURDAY_WHISPER_MAX_LANGUAGES];
} SaturdayWhisperResult;

/* The descriptors stay owned by the caller. Model reads use pread(2), never a path. */
int32_t saturday_whisper_transcribe_fd(
    int model_fd,
    const float *samples,
    uint32_t sample_count,
    uint32_t language_mode,
    SaturdayWhisperAbortCallback should_abort,
    void *abort_context,
    SaturdayWhisperResult *result);

#ifdef __cplusplus
}
#endif

#endif
