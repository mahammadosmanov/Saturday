#ifndef SATURDAY_VOICE_CAPTURE_H
#define SATURDAY_VOICE_CAPTURE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SATURDAY_CAPTURE_DEVICE_LIMIT 64
#define SATURDAY_CAPTURE_LABEL_BYTES 256
#define SATURDAY_CAPTURE_HANDLE_BYTES 16
#define SATURDAY_CAPTURE_MAX_SAMPLES 480000u
#define SATURDAY_CAPTURE_MAX_INPUT_FRAMES 2048u
#define SATURDAY_CAPTURE_CONVERTER_OUTPUT_FRAMES 4096u

enum SaturdayCaptureStatus {
    SATURDAY_CAPTURE_OK = 0,
    SATURDAY_CAPTURE_UNAVAILABLE = 1,
    SATURDAY_CAPTURE_PERMISSION_DENIED = 2,
    SATURDAY_CAPTURE_DEVICE_LOST = 3,
    SATURDAY_CAPTURE_UNSUPPORTED_FORMAT = 4,
    SATURDAY_CAPTURE_OVERFLOW = 5,
    SATURDAY_CAPTURE_INVALID_AUDIO = 6,
    SATURDAY_CAPTURE_CANCELLED = 7,
    SATURDAY_CAPTURE_TIMEOUT = 8,
    SATURDAY_CAPTURE_BUSY = 9,
    SATURDAY_CAPTURE_FAILED = 10,
};

enum SaturdayCaptureEvent {
    SATURDAY_CAPTURE_EVENT_STARTED = 1,
    SATURDAY_CAPTURE_EVENT_PERMISSION_DENIED = 2,
    SATURDAY_CAPTURE_EVENT_DEVICE_LOST = 3,
    SATURDAY_CAPTURE_EVENT_FAILED = 4,
    SATURDAY_CAPTURE_EVENT_METER = 5,
};

typedef struct SaturdayCaptureDevice {
    uint8_t handle[SATURDAY_CAPTURE_HANDLE_BYTES];
    uint16_t label_length;
    uint8_t is_default;
    uint8_t reserved;
    char label[SATURDAY_CAPTURE_LABEL_BYTES];
} SaturdayCaptureDevice;

typedef struct SaturdayCaptureCatalog {
    uint64_t revision;
    uint32_t count;
    uint32_t reserved;
    SaturdayCaptureDevice devices[SATURDAY_CAPTURE_DEVICE_LIMIT];
} SaturdayCaptureCatalog;

typedef int (*SaturdayCaptureEventCallback)(
    void *context,
    uint64_t operation_id,
    uint32_t event,
    uint32_t elapsed_ms,
    float level);

typedef struct SaturdayAudioConverter SaturdayAudioConverter;

typedef struct SaturdayVoiceCapture SaturdayVoiceCapture;

int32_t saturday_voice_capture_create(SaturdayVoiceCapture **out_capture);
void saturday_voice_capture_destroy(SaturdayVoiceCapture *capture);
int32_t saturday_voice_capture_enumerate(
    SaturdayVoiceCapture *capture,
    SaturdayCaptureCatalog *out_catalog);
int32_t saturday_voice_capture_start(
    SaturdayVoiceCapture *capture,
    const uint8_t handle[SATURDAY_CAPTURE_HANDLE_BYTES],
    uint64_t operation_id,
    SaturdayCaptureEventCallback callback,
    void *callback_context);
int32_t saturday_voice_capture_finish(
    SaturdayVoiceCapture *capture,
    uint64_t operation_id,
    int32_t *out_read_fd,
    uint32_t *out_sample_count,
    uint8_t *out_duration_capped);
int32_t saturday_voice_capture_cancel(
    SaturdayVoiceCapture *capture,
    uint64_t operation_id);

int32_t saturday_audio_converter_create(
    uint32_t sample_rate,
    uint32_t channels,
    uint32_t max_input_frames,
    SaturdayAudioConverter **out_converter);
int32_t saturday_audio_converter_convert(
    SaturdayAudioConverter *converter,
    const float *input,
    uint32_t input_frames,
    float *output,
    uint32_t output_capacity,
    uint32_t *out_output_frames);
int32_t saturday_audio_converter_finish(
    SaturdayAudioConverter *converter,
    float *output,
    uint32_t output_capacity,
    uint32_t *out_output_frames);
void saturday_audio_converter_destroy(SaturdayAudioConverter *converter);

#ifdef __cplusplus
}
#endif

#endif
