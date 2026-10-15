#ifndef SATURDAY_VOICE_SPEECH_H
#define SATURDAY_VOICE_SPEECH_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum {
    SATURDAY_SPEECH_ABI_VERSION = 1,
    SATURDAY_SPEECH_MAX_VOICES = 128,
    SATURDAY_SPEECH_MAX_VOICE_ID_BYTES = 512,
    SATURDAY_SPEECH_MAX_DISPLAY_NAME_BYTES = 256,
    SATURDAY_SPEECH_MAX_LOCALE_BYTES = 64,
    SATURDAY_SPEECH_MAX_OWNER_ID_BYTES = 128,
    SATURDAY_SPEECH_MAX_SEGMENT_BYTES = 1120,
    SATURDAY_SPEECH_MAX_SEGMENT_SCALARS = 280,
    SATURDAY_SPEECH_QUEUE_CAPACITY = 3
};

typedef struct SaturdaySpeechHandle SaturdaySpeechHandle;

typedef enum SaturdaySpeechResult {
    SATURDAY_SPEECH_OK = 0,
    SATURDAY_SPEECH_INVALID_ARGUMENT = 1,
    SATURDAY_SPEECH_UNAVAILABLE = 2,
    SATURDAY_SPEECH_QUEUE_FULL = 3,
    SATURDAY_SPEECH_VOICE_UNAVAILABLE = 4,
    SATURDAY_SPEECH_WRONG_LOCALE = 5,
    SATURDAY_SPEECH_ROUTE_LOST = 6,
    SATURDAY_SPEECH_STOP_UNVERIFIED = 7,
    SATURDAY_SPEECH_INTERNAL_ERROR = 8,
    SATURDAY_SPEECH_BUFFER_TOO_SMALL = 9
} SaturdaySpeechResult;

typedef enum SaturdaySpeechLanguage {
    SATURDAY_SPEECH_LANGUAGE_TR = 1,
    SATURDAY_SPEECH_LANGUAGE_EN = 2
} SaturdaySpeechLanguage;

typedef enum SaturdaySpeechEventKind {
    SATURDAY_SPEECH_EVENT_DID_START = 1,
    SATURDAY_SPEECH_EVENT_DID_FINISH = 2,
    SATURDAY_SPEECH_EVENT_CANCELLED = 3,
    SATURDAY_SPEECH_EVENT_FAILED = 4,
    SATURDAY_SPEECH_EVENT_ROUTE_LOST = 5
} SaturdaySpeechEventKind;

typedef struct SaturdaySpeechVoiceRecord {
    uint32_t abi_version;
    uint32_t struct_size;
    uint32_t language;
    uint32_t reserved;
    char identifier[SATURDAY_SPEECH_MAX_VOICE_ID_BYTES + 1];
    char display_name[SATURDAY_SPEECH_MAX_DISPLAY_NAME_BYTES + 1];
    char locale[SATURDAY_SPEECH_MAX_LOCALE_BYTES + 1];
} SaturdaySpeechVoiceRecord;

typedef struct SaturdaySpeechOwner {
    const uint8_t *session_id;
    size_t session_id_length;
    const uint8_t *turn_id;
    size_t turn_id_length;
    uint64_t generation;
    uint64_t operation_id;
} SaturdaySpeechOwner;

typedef struct SaturdaySpeechEvent {
    uint32_t abi_version;
    uint32_t struct_size;
    uint32_t kind;
    uint32_t error_code;
    uint64_t generation;
    uint64_t operation_id;
    uint64_t utterance_id;
    uint32_t session_id_length;
    uint32_t turn_id_length;
    char session_id[SATURDAY_SPEECH_MAX_OWNER_ID_BYTES + 1];
    char turn_id[SATURDAY_SPEECH_MAX_OWNER_ID_BYTES + 1];
} SaturdaySpeechEvent;

typedef void (*SaturdaySpeechCallback)(void *user_data, const SaturdaySpeechEvent *event);

SaturdaySpeechResult saturday_speech_create(
    SaturdaySpeechCallback callback,
    void *user_data,
    SaturdaySpeechHandle **out_handle);

SaturdaySpeechResult saturday_speech_enumerate_voices(
    SaturdaySpeechVoiceRecord *records,
    uint32_t capacity,
    uint32_t *out_count,
    uint64_t *out_revision);

SaturdaySpeechResult saturday_speech_start(
    SaturdaySpeechHandle *handle,
    const SaturdaySpeechOwner *owner,
    uint32_t language,
    const uint8_t *voice_id,
    size_t voice_id_length,
    const uint8_t *locale,
    size_t locale_length,
    const uint8_t *text,
    size_t text_length,
    uint64_t utterance_id);
SaturdaySpeechResult saturday_speech_available_credits(
    SaturdaySpeechHandle *handle,
    uint32_t *out_credits);

SaturdaySpeechResult saturday_speech_cancel(
    SaturdaySpeechHandle *handle,
    const uint8_t *session_id,
    size_t session_id_length,
    const uint8_t *turn_id,
    size_t turn_id_length,
    uint64_t generation);

SaturdaySpeechResult saturday_speech_is_stopped(
    SaturdaySpeechHandle *handle,
    uint32_t *out_is_stopped);

SaturdaySpeechResult saturday_speech_destroy(SaturdaySpeechHandle *handle);

SaturdaySpeechResult saturday_speech_normalize_nfc(
    const uint8_t *input,
    size_t input_length,
    uint8_t *output,
    size_t output_capacity,
    size_t *out_output_length);

#ifdef __cplusplus
}
#endif

#endif
