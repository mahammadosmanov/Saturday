#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <CoreAudio/CoreAudio.h>
#import <Foundation/Foundation.h>

#include "voice_capture.h"

#include <errno.h>
#include <math.h>
#include <poll.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/stat.h>

static const size_t kAudioRingBytes = 1024u * 1024u;
static const UInt32 kMaximumCallbackFrames = 8192u;
static const UInt32 kConverterInputFrames = SATURDAY_CAPTURE_MAX_INPUT_FRAMES;
static const UInt32 kConverterOutputFrames = SATURDAY_CAPTURE_CONVERTER_OUTPUT_FRAMES;
static const UInt32 kPermissionPollMillis = 10u;
static const UInt32 kMeterIntervalMillis = 50u;
static const UInt32 kStopWaitMillis = 2000u;
static void secure_zero_memory(void *memory, size_t length) {
    volatile unsigned char *cursor = (volatile unsigned char *)memory;
    while (length > 0) {
        *cursor++ = 0;
        --length;
    }
}
static uint64_t monotonic_nanos(void);
static void sleep_millis(uint32_t millis);
static void *capture_manager_main(void *context);
static void *capture_pcm_writer_main(void *context);

@interface SCaptureDevice : NSObject {
@public
    uint8_t handle[SATURDAY_CAPTURE_HANDLE_BYTES];
}
@property(nonatomic) AudioDeviceID deviceID;
@property(nonatomic, strong) NSString *uid;
@property(nonatomic) BOOL isDefault;
@end

@implementation SCaptureDevice
@end

@interface SVoiceCaptureController : NSObject
@property(nonatomic, strong) NSArray<SCaptureDevice *> *devices;
@property(nonatomic, strong) id activeSession;
@property(nonatomic) uint64_t revision;
@end

@implementation SVoiceCaptureController
@end

@interface SAudioConverterBox : NSObject
@property(nonatomic, strong) AVAudioConverter *converter;
@property(nonatomic, strong) AVAudioFormat *inputFormat;
@property(nonatomic, strong) AVAudioFormat *outputFormat;
@property(nonatomic, strong) AVAudioPCMBuffer *inputBuffer;
@property(nonatomic, strong) AVAudioPCMBuffer *outputBuffer;
@property(nonatomic) uint32_t maxInputFrames;
@property(nonatomic) uint32_t channels;
@property(nonatomic) BOOL finished;
@end

@implementation SAudioConverterBox
@end

@interface SVoiceCaptureSession : NSObject {
@public
    AudioUnit _unit;
    AudioDeviceID _deviceID;
    AudioStreamBasicDescription _streamFormat;
    uint32_t _inputBytesPerFrame;
    uint32_t _inputChannels;
    uint32_t _inputRate;
    uint32_t _maxCallbackFrames;
    uint64_t _maximumInputFrames;
    uint64_t _startedAtNanos;
    pthread_t _managerThread;
    pthread_t _writerThread;
    BOOL _managerThreadStarted;
    BOOL _writerThreadStarted;
    int _writerFD;
    int _readerFD;
    float *_callbackScratch;
    uint8_t *_workerScratch;
    uint8_t *_ring;
    size_t _ringCapacity;
    float *_canonicalSamples;
    uint32_t _canonicalCount;
    _Atomic(uint64_t) _ringHead;
    _Atomic(uint64_t) _ringTail;
    _Atomic(uint64_t) _inputFrameCount;
    _Atomic(uint32_t) _callbacksInFlight;
    _Atomic(SaturdayCaptureEventCallback) _eventCallback;
    _Atomic(void *) _eventContext;
    _Atomic(int32_t) _status;
    _Atomic(bool) _cancelled;
    _Atomic(bool) _finishRequested;
    _Atomic(bool) _formatChanged;
    _Atomic(bool) _deviceLost;
    _Atomic(bool) _durationCapped;
    _Atomic(bool) _started;
    _Atomic(bool) _unitStarted;
    _Atomic(bool) _unitInitialized;
    _Atomic(bool) _writerDone;
    _Atomic(bool) _permissionDone;
    _Atomic(bool) _permissionGranted;
    _Atomic(float) _meterLevel;
}
@property(nonatomic, strong) NSString *deviceUID;
@property(nonatomic) uint64_t operationID;
@property(nonatomic) uint32_t sampleRate;
@property(nonatomic) uint32_t channels;
@property(nonatomic, strong) AVAudioConverter *converter;
@property(nonatomic, strong) AVAudioFormat *inputFormat;
@property(nonatomic, strong) AVAudioFormat *outputFormat;
@property(nonatomic, strong) AVAudioPCMBuffer *converterInput;
@property(nonatomic, strong) AVAudioPCMBuffer *converterOutput;
- (instancetype)initWithDevice:(AudioDeviceID)device
                           uid:(NSString *)uid
                   operationID:(uint64_t)operationID
                      callback:(SaturdayCaptureEventCallback)callback
                       context:(void *)context;
- (int32_t)begin;
- (void)requestFinish;
- (void)requestCancel;
- (int32_t)joinManager;
- (int32_t)makePCMReader:(int32_t *)outReadFD sampleCount:(uint32_t *)outCount durationCapped:(uint8_t *)outCapped;
- (int32_t)joinWriter;
- (void)zeroAudioBuffers;
@end

@implementation SVoiceCaptureSession

- (instancetype)initWithDevice:(AudioDeviceID)device
                           uid:(NSString *)uid
                   operationID:(uint64_t)operationID
                      callback:(SaturdayCaptureEventCallback)callback
                       context:(void *)context {
    self = [super init];
    if (self == nil) {
        return nil;
    }
    _deviceID = device;
    _deviceUID = [uid copy];
    _operationID = operationID;
    _writerFD = -1;
    _readerFD = -1;
    atomic_init(&_ringHead, 0);
    atomic_init(&_ringTail, 0);
    atomic_init(&_inputFrameCount, 0);
    atomic_init(&_callbacksInFlight, 0);
    atomic_init(&_eventCallback, callback);
    atomic_init(&_eventContext, context);
    atomic_init(&_status, SATURDAY_CAPTURE_OK);
    atomic_init(&_cancelled, false);
    atomic_init(&_finishRequested, false);
    atomic_init(&_formatChanged, false);
    atomic_init(&_deviceLost, false);
    atomic_init(&_durationCapped, false);
    atomic_init(&_started, false);
    atomic_init(&_unitStarted, false);
    atomic_init(&_unitInitialized, false);
    atomic_init(&_permissionDone, false);
    atomic_init(&_permissionGranted, false);
    atomic_init(&_meterLevel, 0.0F);
    _ringCapacity = kAudioRingBytes;
    _ring = (uint8_t *)calloc(1, _ringCapacity);
    _canonicalSamples = (float *)calloc(SATURDAY_CAPTURE_MAX_SAMPLES, sizeof(float));
    if (_ring == NULL || _canonicalSamples == NULL) {
        [self zeroAudioBuffers];
        return nil;
    }
    return self;
}

- (int32_t)begin {
    if (pthread_create(&_managerThread, NULL, capture_manager_main, (__bridge_retained void *)self) != 0) {
        atomic_store_explicit(&_status, SATURDAY_CAPTURE_UNAVAILABLE, memory_order_release);
        return SATURDAY_CAPTURE_UNAVAILABLE;
    }
    _managerThreadStarted = YES;
    return SATURDAY_CAPTURE_OK;
}

- (void)requestFinish {
    atomic_store_explicit(&_finishRequested, true, memory_order_release);
}

- (void)requestCancel {
    atomic_store_explicit(&_cancelled, true, memory_order_release);
    atomic_store_explicit(&_finishRequested, true, memory_order_release);
    atomic_store_explicit(&_eventCallback, NULL, memory_order_release);
    atomic_store_explicit(&_eventContext, NULL, memory_order_release);

}
- (int32_t)joinManager {
    if (_managerThreadStarted) {
        if (pthread_join(_managerThread, NULL) != 0) {
            atomic_store_explicit(&_status, SATURDAY_CAPTURE_FAILED, memory_order_release);
            return SATURDAY_CAPTURE_FAILED;
        }
        _managerThreadStarted = NO;
    }
    if (atomic_load_explicit(&_callbacksInFlight, memory_order_acquire) != 0) {
        const uint64_t end = monotonic_nanos() + (uint64_t)kStopWaitMillis * 1000000ull;
        while (atomic_load_explicit(&_callbacksInFlight, memory_order_acquire) != 0 && monotonic_nanos() < end) {
            sleep_millis(1);
        }
        if (atomic_load_explicit(&_callbacksInFlight, memory_order_acquire) != 0) {
            atomic_store_explicit(&_status, SATURDAY_CAPTURE_FAILED, memory_order_release);
            return SATURDAY_CAPTURE_FAILED;
        }
    }
    return atomic_load_explicit(&_status, memory_order_acquire);
}

- (int32_t)makePCMReader:(int32_t *)outReadFD sampleCount:(uint32_t *)outCount durationCapped:(uint8_t *)outCapped {
    if (outReadFD == NULL || outCount == NULL || outCapped == NULL ||
        atomic_load_explicit(&_cancelled, memory_order_acquire) ||
        atomic_load_explicit(&_status, memory_order_acquire) != SATURDAY_CAPTURE_OK ||
        !atomic_load_explicit(&_started, memory_order_acquire)) {
        return atomic_load_explicit(&_status, memory_order_acquire) == SATURDAY_CAPTURE_OK
            ? SATURDAY_CAPTURE_CANCELLED
            : atomic_load_explicit(&_status, memory_order_acquire);
    }
    if (_canonicalCount == 0 || _canonicalCount > SATURDAY_CAPTURE_MAX_SAMPLES) {
        [self zeroAudioBuffers];
        return SATURDAY_CAPTURE_INVALID_AUDIO;
    }
    int descriptors[2] = {-1, -1};
    if (pipe(descriptors) != 0) {
        [self zeroAudioBuffers];
        return SATURDAY_CAPTURE_UNAVAILABLE;
    }
    const int read_flags = fcntl(descriptors[0], F_GETFD);
    const int write_flags = fcntl(descriptors[1], F_GETFD);
    const int read_status = read_flags < 0 ? -1 : fcntl(descriptors[0], F_SETFD, read_flags | FD_CLOEXEC);
    const int write_status = write_flags < 0 ? -1 : fcntl(descriptors[1], F_SETFD, write_flags | FD_CLOEXEC);
    const int write_mode = fcntl(descriptors[1], F_GETFL);
    if (read_status < 0 || write_status < 0 || write_mode < 0 ||
        fcntl(descriptors[1], F_SETFL, write_mode | O_NONBLOCK) < 0 ||
        fcntl(descriptors[1], F_SETNOSIGPIPE, 1) < 0) {
        close(descriptors[0]);
        close(descriptors[1]);
        [self zeroAudioBuffers];
        return SATURDAY_CAPTURE_UNAVAILABLE;
    }

    _readerFD = descriptors[0];
    _writerFD = descriptors[1];
    if (pthread_create(&_writerThread, NULL, capture_pcm_writer_main, (__bridge_retained void *)self) != 0) {
        close(_readerFD);
        close(_writerFD);
        _readerFD = -1;
        _writerFD = -1;
        [self zeroAudioBuffers];
        return SATURDAY_CAPTURE_UNAVAILABLE;
    }
    _writerThreadStarted = YES;
    *outReadFD = _readerFD;
    _readerFD = -1;
    *outCount = _canonicalCount;
    *outCapped = atomic_load_explicit(&_durationCapped, memory_order_acquire) ? 1 : 0;
    return SATURDAY_CAPTURE_OK;
}

- (int32_t)joinWriter {
    if (_writerThreadStarted) {
        if (pthread_join(_writerThread, NULL) != 0) {
            return SATURDAY_CAPTURE_FAILED;
        }
        _writerThreadStarted = NO;
    }
    return atomic_load_explicit(&_writerDone, memory_order_acquire) ||
            atomic_load_explicit(&_cancelled, memory_order_acquire)
        ? SATURDAY_CAPTURE_OK
        : SATURDAY_CAPTURE_FAILED;
}

- (void)zeroAudioBuffers {
    if (_canonicalSamples != NULL) {
        secure_zero_memory(_canonicalSamples, SATURDAY_CAPTURE_MAX_SAMPLES * sizeof(float));
        free(_canonicalSamples);
        _canonicalSamples = NULL;
    }
    if (_ring != NULL) {
        secure_zero_memory(_ring, _ringCapacity);
        free(_ring);
        _ring = NULL;
    }
    if (_callbackScratch != NULL) {
        secure_zero_memory(_callbackScratch, (size_t)kMaximumCallbackFrames * 2u * sizeof(float));
        free(_callbackScratch);
        _callbackScratch = NULL;
    }
    if (_workerScratch != NULL) {
        secure_zero_memory(_workerScratch, (size_t)kConverterInputFrames * 2u * sizeof(float));
        free(_workerScratch);
        _workerScratch = NULL;
    }
    _canonicalCount = 0;
}

- (void)dealloc {
    [self requestCancel];
    [self joinManager];
    [self joinWriter];
    if (_readerFD >= 0) {
        close(_readerFD);
        _readerFD = -1;
    }
    if (_writerFD >= 0) {
        close(_writerFD);
        _writerFD = -1;
    }
    [self zeroAudioBuffers];
}
@end

struct SaturdayAudioConverter {
    void *object;
};

struct SaturdayVoiceCapture {
    pthread_mutex_t mutex;
    void *controller;
};

static _Atomic(uint64_t) gCatalogRevision = 0;

static uint64_t monotonic_nanos(void) {
    struct timespec value;
    if (clock_gettime(CLOCK_MONOTONIC, &value) != 0) {
        return 0;
    }
    return (uint64_t)value.tv_sec * 1000000000ull + (uint64_t)value.tv_nsec;
}

static void sleep_millis(uint32_t millis) {
    struct timespec remaining = {
        .tv_sec = (time_t)(millis / 1000u),
        .tv_nsec = (long)(millis % 1000u) * 1000000l,
    };
    while (nanosleep(&remaining, &remaining) != 0 && errno == EINTR) {}
}

static bool bytes_are_zero(const uint8_t *bytes, size_t length) {
    uint8_t combined = 0;
    for (size_t index = 0; index < length; ++index) {
        combined |= bytes[index];
    }
    return combined == 0;
}

static bool device_has_input_channels(AudioDeviceID device, uint32_t *outChannels) {
    AudioObjectPropertyAddress address = {
        kAudioDevicePropertyStreamConfiguration,
        kAudioDevicePropertyScopeInput,
        kAudioObjectPropertyElementMain,
    };
    UInt32 dataSize = 0;
    if (AudioObjectGetPropertyDataSize(device, &address, 0, NULL, &dataSize) != noErr ||
        dataSize < sizeof(AudioBufferList) || dataSize > 64u * 1024u) {
        return false;
    }
    AudioBufferList *list = (AudioBufferList *)calloc(1, dataSize);
    if (list == NULL) {
        return false;
    }
    const OSStatus status = AudioObjectGetPropertyData(device, &address, 0, NULL, &dataSize, list);
    uint64_t channels = 0;
    if (status == noErr) {
        for (UInt32 index = 0; index < list->mNumberBuffers; ++index) {
            channels += list->mBuffers[index].mNumberChannels;
        }
    }
    free(list);
    if (status != noErr || channels == 0 || channels > 2) {
        return false;
    }
    *outChannels = (uint32_t)channels;
    return true;
}

static NSString *device_uid(AudioDeviceID device) {
    AudioObjectPropertyAddress address = {
        kAudioDevicePropertyDeviceUID,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    };
    CFStringRef uid = NULL;
    UInt32 size = sizeof(uid);
    if (AudioObjectGetPropertyData(device, &address, 0, NULL, &size, &uid) != noErr || uid == NULL) {
        return nil;
    }
    NSString *value = [(__bridge NSString *)uid copy];
    CFRelease(uid);
    return value;
}

static bool find_device_for_uid(NSString *uid, AudioDeviceID *outDevice, uint32_t *outChannels) {
    AudioObjectPropertyAddress address = {
        kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    };
    UInt32 dataSize = 0;
    if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &address, 0, NULL, &dataSize) != noErr ||
        dataSize == 0 || dataSize > 64u * 1024u || dataSize % sizeof(AudioDeviceID) != 0) {
        return false;
    }
    const size_t count = dataSize / sizeof(AudioDeviceID);
    AudioDeviceID *devices = (AudioDeviceID *)calloc(count, sizeof(AudioDeviceID));
    if (devices == NULL) {
        return false;
    }
    const OSStatus status = AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &dataSize, devices);
    bool found = false;
    if (status == noErr) {
        for (size_t index = 0; index < count; ++index) {
            NSString *candidate = device_uid(devices[index]);
            uint32_t channels = 0;
            if (candidate != nil && [candidate isEqualToString:uid] &&
                device_has_input_channels(devices[index], &channels)) {
                *outDevice = devices[index];
                *outChannels = channels;
                found = true;
                break;
            }
        }
    }
    free(devices);
    return found;
}

static bool is_device_alive(AudioDeviceID device) {
    AudioObjectPropertyAddress address = {
        kAudioDevicePropertyDeviceIsAlive,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    };
    UInt32 alive = 0;
    UInt32 size = sizeof(alive);
    return AudioObjectGetPropertyData(device, &address, 0, NULL, &size, &alive) == noErr && alive != 0;
}

static bool enumerate_devices(SVoiceCaptureController *controller, SaturdayCaptureCatalog *catalog) {
    AudioObjectPropertyAddress listAddress = {
        kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    };
    UInt32 listSize = 0;
    if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &listAddress, 0, NULL, &listSize) != noErr ||
        listSize == 0 || listSize > 64u * 1024u || listSize % sizeof(AudioDeviceID) != 0) {
        return false;
    }
    const size_t count = listSize / sizeof(AudioDeviceID);
    AudioDeviceID *ids = (AudioDeviceID *)calloc(count, sizeof(AudioDeviceID));
    if (ids == NULL) {
        return false;
    }
    const OSStatus listStatus = AudioObjectGetPropertyData(kAudioObjectSystemObject, &listAddress, 0, NULL, &listSize, ids);
    if (listStatus != noErr) {
        free(ids);
        return false;
    }

    AudioDeviceID defaultDevice = kAudioObjectUnknown;
    AudioObjectPropertyAddress defaultAddress = {
        kAudioHardwarePropertyDefaultInputDevice,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    };
    UInt32 defaultSize = sizeof(defaultDevice);
    (void)AudioObjectGetPropertyData(kAudioObjectSystemObject, &defaultAddress, 0, NULL, &defaultSize, &defaultDevice);

    NSMutableArray<SCaptureDevice *> *entries = [NSMutableArray arrayWithCapacity:MIN(count, SATURDAY_CAPTURE_DEVICE_LIMIT)];
    memset(catalog, 0, sizeof(*catalog));
    for (size_t index = 0; index < count && entries.count < SATURDAY_CAPTURE_DEVICE_LIMIT; ++index) {
        uint32_t channels = 0;
        if (!device_has_input_channels(ids[index], &channels)) {
            continue;
        }
        NSString *uid = device_uid(ids[index]);
        if (uid == nil || uid.length == 0) {
            continue;
        }
        SCaptureDevice *entry = [[SCaptureDevice alloc] init];
        entry.deviceID = ids[index];
        entry.uid = uid;
        entry.isDefault = ids[index] == defaultDevice;
        arc4random_buf(entry->handle, SATURDAY_CAPTURE_HANDLE_BYTES);
        if (bytes_are_zero(entry->handle, SATURDAY_CAPTURE_HANDLE_BYTES)) {
            entry->handle[0] = 1;
        }
        SaturdayCaptureDevice *record = &catalog->devices[entries.count];
        memcpy(record->handle, entry->handle, SATURDAY_CAPTURE_HANDLE_BYTES);
        const int written = snprintf(record->label, sizeof(record->label), "Audio input %u", (unsigned)entries.count + 1u);
        if (written <= 0 || (size_t)written >= sizeof(record->label)) {
            continue;
        }
        record->label_length = (uint16_t)written;
        record->is_default = entry.isDefault ? 1u : 0u;
        entries[entries.count] = entry;
    }
    free(ids);
    catalog->count = (uint32_t)entries.count;
    catalog->revision = atomic_fetch_add_explicit(&gCatalogRevision, 1, memory_order_relaxed) + 1;
    controller.devices = [entries copy];
    controller.revision = catalog->revision;
    return true;
}

static SCaptureDevice *selected_device(SVoiceCaptureController *controller, const uint8_t *handle) {
    if (handle == NULL) {
        return nil;
    }
    for (SCaptureDevice *entry in controller.devices) {
        if (memcmp(entry->handle, handle, SATURDAY_CAPTURE_HANDLE_BYTES) == 0) {
            return entry;
        }
    }
    return nil;
}

static bool ring_write(SVoiceCaptureSession *session, const uint8_t *bytes, size_t count) {
    const uint64_t head = atomic_load_explicit(&session->_ringHead, memory_order_relaxed);
    const uint64_t tail = atomic_load_explicit(&session->_ringTail, memory_order_acquire);
    if (head < tail || count > session->_ringCapacity || head - tail > session->_ringCapacity - count) {
        return false;
    }
    const size_t start = (size_t)(head % session->_ringCapacity);
    const size_t first = MIN(count, session->_ringCapacity - start);
    memcpy(session->_ring + start, bytes, first);
    if (first < count) {
        memcpy(session->_ring, bytes + first, count - first);
    }
    atomic_store_explicit(&session->_ringHead, head + count, memory_order_release);
    return true;
}

static size_t ring_read(SVoiceCaptureSession *session, uint8_t *output, size_t capacity) {
    const uint64_t tail = atomic_load_explicit(&session->_ringTail, memory_order_relaxed);
    const uint64_t head = atomic_load_explicit(&session->_ringHead, memory_order_acquire);
    if (head < tail || head == tail) {
        return 0;
    }
    size_t count = (size_t)MIN((uint64_t)capacity, head - tail);
    count -= count % session->_inputBytesPerFrame;
    if (count == 0) {
        return 0;
    }
    const size_t start = (size_t)(tail % session->_ringCapacity);
    const size_t first = MIN(count, session->_ringCapacity - start);
    memcpy(output, session->_ring + start, first);
    if (first < count) {
        memcpy(output + first, session->_ring, count - first);
    }
    atomic_store_explicit(&session->_ringTail, tail + count, memory_order_release);
    return count;
}

static void set_session_error(SVoiceCaptureSession *session, int32_t status) {
    int32_t expected = SATURDAY_CAPTURE_OK;
    (void)atomic_compare_exchange_strong_explicit(
        &session->_status, &expected, status, memory_order_acq_rel, memory_order_acquire);
    atomic_store_explicit(&session->_finishRequested, true, memory_order_release);
}

static void emit_event(SVoiceCaptureSession *session, uint32_t event, uint32_t elapsed, float level) {
    atomic_fetch_add_explicit(&session->_callbacksInFlight, 1, memory_order_acq_rel);
    if (atomic_load_explicit(&session->_cancelled, memory_order_acquire)) {
        atomic_fetch_sub_explicit(&session->_callbacksInFlight, 1, memory_order_release);
        return;
    }
    SaturdayCaptureEventCallback callback = atomic_load_explicit(&session->_eventCallback, memory_order_acquire);
    void *context = atomic_load_explicit(&session->_eventContext, memory_order_acquire);
    if (callback == NULL || context == NULL || atomic_load_explicit(&session->_cancelled, memory_order_acquire)) {
        atomic_fetch_sub_explicit(&session->_callbacksInFlight, 1, memory_order_release);
        return;
    }
    const int accepted = callback(context, session.operationID, event, elapsed, level);
    atomic_fetch_sub_explicit(&session->_callbacksInFlight, 1, memory_order_release);
    if (accepted != 0 && event != SATURDAY_CAPTURE_EVENT_METER) {
        atomic_store_explicit(&session->_cancelled, true, memory_order_release);
        atomic_store_explicit(&session->_finishRequested, true, memory_order_release);
    }
}

static bool configure_converter(SVoiceCaptureSession *session, const AudioStreamBasicDescription *input) {
    AVAudioFormat *inputFormat = [[AVAudioFormat alloc] initWithStreamDescription:input];
    AVAudioFormat *outputFormat = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                                 sampleRate:16000.0
                                                                   channels:1
                                                                interleaved:YES];
    if (inputFormat == nil || outputFormat == nil) {
        return false;
    }
    AVAudioConverter *converter = [[AVAudioConverter alloc] initFromFormat:inputFormat toFormat:outputFormat];
    if (converter == nil) {
        return false;
    }
    AVAudioPCMBuffer *inputBuffer = [[AVAudioPCMBuffer alloc] initWithPCMFormat:inputFormat
                                                                  frameCapacity:kConverterInputFrames];
    AVAudioPCMBuffer *outputBuffer = [[AVAudioPCMBuffer alloc] initWithPCMFormat:outputFormat
                                                                   frameCapacity:kConverterOutputFrames];
    if (inputBuffer == nil || outputBuffer == nil || inputBuffer.floatChannelData == NULL ||
        outputBuffer.floatChannelData == NULL) {
        return false;
    }
    session.inputFormat = inputFormat;
    session.outputFormat = outputFormat;
    session.converter = converter;
    session.converterInput = inputBuffer;
    session.converterOutput = outputBuffer;
    return true;
}

static bool append_converted(SVoiceCaptureSession *session, AVAudioPCMBuffer *outputBuffer) {
    const uint32_t frames = outputBuffer.frameLength;
    float *samples = outputBuffer.floatChannelData[0];
    if (frames == 0 || samples == NULL) {
        return true;
    }
    if (session->_canonicalCount > SATURDAY_CAPTURE_MAX_SAMPLES) {
        return false;
    }
    const uint32_t remaining = SATURDAY_CAPTURE_MAX_SAMPLES - session->_canonicalCount;
    const uint32_t kept = MIN(frames, remaining);
    for (uint32_t index = 0; index < frames; ++index) {
        if (!isfinite(samples[index])) {
            return false;
        }
    }
    for (uint32_t index = 0; index < kept; ++index) {
        session->_canonicalSamples[session->_canonicalCount + index] = fmaxf(-1.0f, fminf(1.0f, samples[index]));
    }
    session->_canonicalCount += kept;
    if (kept < frames) {
        atomic_store_explicit(&session->_durationCapped, true, memory_order_release);
        atomic_store_explicit(&session->_finishRequested, true, memory_order_release);
    }
    return true;
}

static bool convert_input_block(SVoiceCaptureSession *session, const uint8_t *bytes, uint32_t frames) {
    if (frames == 0 || frames > kConverterInputFrames ||
        session.converterInput.floatChannelData == NULL || session.converterOutput.floatChannelData == NULL) {
        return false;
    }
    const size_t byteCount = (size_t)frames * session->_inputBytesPerFrame;
    float *input = session.converterInput.floatChannelData[0];
    const size_t sampleCount = (size_t)frames * session->_inputChannels;
    memcpy(input, bytes, byteCount);
    for (size_t index = 0; index < sampleCount; ++index) {
        if (!isfinite(input[index])) {
            return false;
        }
    }
    session.converterInput.frameLength = frames;
    session.converterOutput.frameLength = 0;
    __block BOOL suppliedInput = NO;
    NSError *error = nil;
    AVAudioConverterOutputStatus status = [session.converter convertToBuffer:session.converterOutput
                                                                        error:&error
                                                           withInputFromBlock:^AVAudioBuffer *(AVAudioPacketCount, AVAudioConverterInputStatus *inputStatus) {
        if (suppliedInput) {
            *inputStatus = AVAudioConverterInputStatus_NoDataNow;
            return nil;
        }
        suppliedInput = YES;
        *inputStatus = AVAudioConverterInputStatus_HaveData;
        return session.converterInput;
    }];
    if (error != nil || status == AVAudioConverterOutputStatus_Error ||
        status == AVAudioConverterOutputStatus_EndOfStream) {
        return false;
    }
    return append_converted(session, session.converterOutput);
}

static bool flush_converter(SVoiceCaptureSession *session) {
    if (session.converter == nil || session.converterOutput == nil) {
        return false;
    }
    for (uint32_t attempt = 0; attempt < 8; ++attempt) {
        session.converterOutput.frameLength = 0;
        __block BOOL suppliedEnd = NO;
        NSError *error = nil;
        AVAudioConverterOutputStatus status = [session.converter convertToBuffer:session.converterOutput
                                                                            error:&error
                                                               withInputFromBlock:^AVAudioBuffer *(AVAudioPacketCount, AVAudioConverterInputStatus *inputStatus) {
            suppliedEnd = YES;
            *inputStatus = AVAudioConverterInputStatus_EndOfStream;
            return nil;
        }];
        if (error != nil || status == AVAudioConverterOutputStatus_Error ||
            !append_converted(session, session.converterOutput)) {
            return false;
        }
        if (status == AVAudioConverterOutputStatus_EndOfStream ||
            (suppliedEnd && session.converterOutput.frameLength == 0)) {
            return true;
        }
    }
    return false;
}

static OSStatus input_callback(void *context,
                               AudioUnitRenderActionFlags *flags,
                               const AudioTimeStamp *timeStamp,
                               UInt32,
                               UInt32 frames,
                               AudioBufferList *) {
    SVoiceCaptureSession *session = (__bridge SVoiceCaptureSession *)context;
    atomic_fetch_add_explicit(&session->_callbacksInFlight, 1, memory_order_acq_rel);
    if (atomic_load_explicit(&session->_cancelled, memory_order_acquire) ||
        atomic_load_explicit(&session->_finishRequested, memory_order_acquire)) {
        atomic_fetch_sub_explicit(&session->_callbacksInFlight, 1, memory_order_release);
        return noErr;
    }
    if (frames == 0 || frames > session->_maxCallbackFrames || session->_unit == NULL) {
        set_session_error(session, SATURDAY_CAPTURE_UNSUPPORTED_FORMAT);
        atomic_fetch_sub_explicit(&session->_callbacksInFlight, 1, memory_order_release);
        return noErr;
    }
    struct {
        UInt32 mNumberBuffers;
        AudioBuffer mBuffers[1];
    } rendered = {0};
    rendered.mNumberBuffers = 1;
    rendered.mBuffers[0].mNumberChannels = session->_inputChannels;
    rendered.mBuffers[0].mDataByteSize = frames * session->_inputBytesPerFrame;
    rendered.mBuffers[0].mData = session->_callbackScratch;
    const OSStatus renderStatus = AudioUnitRender(session->_unit, flags, timeStamp, 1, frames,
                                                   (AudioBufferList *)&rendered);
    if (renderStatus != noErr || rendered.mBuffers[0].mDataByteSize != frames * session->_inputBytesPerFrame) {
        set_session_error(session, SATURDAY_CAPTURE_DEVICE_LOST);
        atomic_fetch_sub_explicit(&session->_callbacksInFlight, 1, memory_order_release);
        return noErr;
    }

    float peak = 0.0f;
    const size_t sourceSamples = (size_t)frames * session->_inputChannels;
    for (size_t index = 0; index < sourceSamples; ++index) {
        const float value = session->_callbackScratch[index];
        if (!isfinite(value)) {
            set_session_error(session, SATURDAY_CAPTURE_INVALID_AUDIO);
            atomic_fetch_sub_explicit(&session->_callbacksInFlight, 1, memory_order_release);
            return noErr;
        }
        peak = fmaxf(peak, fabsf(value));
    }

    const uint64_t alreadyCaptured = atomic_load_explicit(&session->_inputFrameCount, memory_order_relaxed);
    if (alreadyCaptured > session->_maximumInputFrames) {
        set_session_error(session, SATURDAY_CAPTURE_OVERFLOW);
        atomic_fetch_sub_explicit(&session->_callbacksInFlight, 1, memory_order_release);
        return noErr;
    }
    const uint64_t remaining = session->_maximumInputFrames - alreadyCaptured;
    const uint32_t keptFrames = (uint32_t)MIN((uint64_t)frames, remaining);
    const size_t keptBytes = (size_t)keptFrames * session->_inputBytesPerFrame;
    if (keptBytes > 0 && !ring_write(session, (const uint8_t *)session->_callbackScratch, keptBytes)) {
        set_session_error(session, SATURDAY_CAPTURE_OVERFLOW);
        atomic_fetch_sub_explicit(&session->_callbacksInFlight, 1, memory_order_release);
        return noErr;
    }
    atomic_store_explicit(&session->_inputFrameCount, alreadyCaptured + keptFrames, memory_order_release);
    atomic_store_explicit(&session->_meterLevel, fminf(peak, 1.0f), memory_order_release);
    if (keptFrames < frames || alreadyCaptured + keptFrames >= session->_maximumInputFrames) {
        atomic_store_explicit(&session->_durationCapped, true, memory_order_release);
        atomic_store_explicit(&session->_finishRequested, true, memory_order_release);
    }
    atomic_fetch_sub_explicit(&session->_callbacksInFlight, 1, memory_order_release);
    return noErr;
}

static OSStatus device_property_changed(AudioObjectID objectID,
                                        UInt32 addressCount,
                                        const AudioObjectPropertyAddress addresses[],
                                        void *context) {
    SVoiceCaptureSession *session = (__bridge SVoiceCaptureSession *)context;
    if (atomic_load_explicit(&session->_cancelled, memory_order_acquire)) {
        return noErr;
    }
    for (UInt32 index = 0; index < addressCount; ++index) {
        if (addresses[index].mSelector == kAudioDevicePropertyDeviceIsAlive) {
            if (objectID != session->_deviceID || !is_device_alive(session->_deviceID)) {
                atomic_store_explicit(&session->_deviceLost, true, memory_order_release);
            }
        } else {
            atomic_store_explicit(&session->_formatChanged, true, memory_order_release);
        }
    }
    return noErr;
}

static void remove_device_listeners(SVoiceCaptureSession *session) {
    AudioObjectPropertyAddress alive = {
        kAudioDevicePropertyDeviceIsAlive,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    };
    AudioObjectPropertyAddress rate = {
        kAudioDevicePropertyNominalSampleRate,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    };
    AudioObjectPropertyAddress streams = {
        kAudioDevicePropertyStreamConfiguration,
        kAudioDevicePropertyScopeInput,
        kAudioObjectPropertyElementMain,
    };
    (void)AudioObjectRemovePropertyListener(session->_deviceID, &alive, device_property_changed, (__bridge void *)session);
    (void)AudioObjectRemovePropertyListener(session->_deviceID, &rate, device_property_changed, (__bridge void *)session);
    (void)AudioObjectRemovePropertyListener(session->_deviceID, &streams, device_property_changed, (__bridge void *)session);
}

static bool add_device_listeners(SVoiceCaptureSession *session) {
    AudioObjectPropertyAddress addresses[] = {
        {kAudioDevicePropertyDeviceIsAlive, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain},
        {kAudioDevicePropertyNominalSampleRate, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain},
        {kAudioDevicePropertyStreamConfiguration, kAudioDevicePropertyScopeInput, kAudioObjectPropertyElementMain},
    };
    for (size_t index = 0; index < sizeof(addresses) / sizeof(addresses[0]); ++index) {
        if (AudioObjectAddPropertyListener(session->_deviceID, &addresses[index], device_property_changed,
                                           (__bridge void *)session) != noErr) {
            remove_device_listeners(session);
            return false;
        }
    }
    return true;
}

static bool dispose_audio_unit(SVoiceCaptureSession *session) {
    if (session->_unit == NULL) {
        return true;
    }
    if (atomic_load_explicit(&session->_unitStarted, memory_order_acquire)) {
        if (AudioOutputUnitStop(session->_unit) != noErr) {
            set_session_error(session, SATURDAY_CAPTURE_FAILED);
            return false;
        }
        atomic_store_explicit(&session->_unitStarted, false, memory_order_release);
    }
    const uint64_t deadline = monotonic_nanos() + (uint64_t)kStopWaitMillis * 1000000ull;
    while (atomic_load_explicit(&session->_callbacksInFlight, memory_order_acquire) != 0 && monotonic_nanos() < deadline) {
        sleep_millis(1);
    }
    if (atomic_load_explicit(&session->_callbacksInFlight, memory_order_acquire) != 0) {
        set_session_error(session, SATURDAY_CAPTURE_FAILED);
        return false;
    }
    remove_device_listeners(session);
    if (atomic_load_explicit(&session->_unitInitialized, memory_order_acquire)) {
        if (AudioUnitUninitialize(session->_unit) != noErr) {
            set_session_error(session, SATURDAY_CAPTURE_FAILED);
        } else {
            atomic_store_explicit(&session->_unitInitialized, false, memory_order_release);
        }
    }
    if (AudioComponentInstanceDispose(session->_unit) != noErr) {
        set_session_error(session, SATURDAY_CAPTURE_FAILED);
        return false;
    }
    session->_unit = NULL;
    return true;
}

static bool setup_audio_unit(SVoiceCaptureSession *session) {
    AudioDeviceID currentDevice = kAudioObjectUnknown;
    uint32_t channels = 0;
    if (!find_device_for_uid(session.deviceUID, &currentDevice, &channels) || currentDevice != session->_deviceID ||
        !is_device_alive(currentDevice)) {
        set_session_error(session, SATURDAY_CAPTURE_DEVICE_LOST);
        return false;
    }
    AudioComponentDescription description = {
        kAudioUnitType_Output,
        kAudioUnitSubType_HALOutput,
        kAudioUnitManufacturer_Apple,
        0,
        0,
    };
    AudioComponent component = AudioComponentFindNext(NULL, &description);
    if (component == NULL || AudioComponentInstanceNew(component, &session->_unit) != noErr) {
        set_session_error(session, SATURDAY_CAPTURE_UNAVAILABLE);
        return false;
    }

    UInt32 enableInput = 1;
    UInt32 disableOutput = 0;
    if (AudioUnitSetProperty(session->_unit, kAudioOutputUnitProperty_EnableIO, kAudioUnitScope_Input,
                             1, &enableInput, sizeof(enableInput)) != noErr ||
        AudioUnitSetProperty(session->_unit, kAudioOutputUnitProperty_EnableIO, kAudioUnitScope_Output,
                             0, &disableOutput, sizeof(disableOutput)) != noErr ||
        AudioUnitSetProperty(session->_unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global,
                             0, &currentDevice, sizeof(currentDevice)) != noErr) {
        set_session_error(session, SATURDAY_CAPTURE_UNAVAILABLE);
        dispose_audio_unit(session);
        return false;
    }

    AudioStreamBasicDescription nativeFormat = {0};
    UInt32 formatSize = sizeof(nativeFormat);
    if (AudioUnitGetProperty(session->_unit, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Output,
                              1, &nativeFormat, &formatSize) != noErr ||
        nativeFormat.mFormatID != kAudioFormatLinearPCM || !isfinite(nativeFormat.mSampleRate) ||
        nativeFormat.mSampleRate < 8000.0 || nativeFormat.mSampleRate > 192000.0 ||
        nativeFormat.mChannelsPerFrame != channels || (channels != 1 && channels != 2)) {
        set_session_error(session, SATURDAY_CAPTURE_UNSUPPORTED_FORMAT);
        dispose_audio_unit(session);
        return false;
    }

    AudioStreamBasicDescription clientFormat = {0};
    clientFormat.mSampleRate = nativeFormat.mSampleRate;
    clientFormat.mFormatID = kAudioFormatLinearPCM;
    clientFormat.mFormatFlags = kAudioFormatFlagsNativeEndian | kAudioFormatFlagIsPacked | kAudioFormatFlagIsFloat;
    clientFormat.mBytesPerPacket = sizeof(float) * channels;
    clientFormat.mFramesPerPacket = 1;
    clientFormat.mBytesPerFrame = sizeof(float) * channels;
    clientFormat.mChannelsPerFrame = channels;
    clientFormat.mBitsPerChannel = 32;
    if (AudioUnitSetProperty(session->_unit, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Output,
                             1, &clientFormat, sizeof(clientFormat)) != noErr) {
        set_session_error(session, SATURDAY_CAPTURE_UNSUPPORTED_FORMAT);
        dispose_audio_unit(session);
        return false;
    }

    AURenderCallbackStruct callback = {input_callback, (__bridge void *)session};
    if (AudioUnitSetProperty(session->_unit, kAudioOutputUnitProperty_SetInputCallback, kAudioUnitScope_Global,
                             0, &callback, sizeof(callback)) != noErr ||
        !add_device_listeners(session)) {
        set_session_error(session, SATURDAY_CAPTURE_UNAVAILABLE);
        dispose_audio_unit(session);
        return false;
    }

    UInt32 maximumFrames = 0;
    UInt32 maximumFramesSize = sizeof(maximumFrames);
    if (AudioUnitGetProperty(session->_unit, kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitScope_Global,
                              0, &maximumFrames, &maximumFramesSize) != noErr ||
        maximumFrames == 0 || maximumFrames > kMaximumCallbackFrames) {
        set_session_error(session, SATURDAY_CAPTURE_UNSUPPORTED_FORMAT);
        dispose_audio_unit(session);
        return false;
    }
    session->_maxCallbackFrames = maximumFrames;
    session->_inputBytesPerFrame = sizeof(float) * channels;
    session->_inputChannels = channels;
    session->_inputRate = (uint32_t)llround(nativeFormat.mSampleRate);
    session->_maximumInputFrames = (uint64_t)session->_inputRate * 30ull;
    session.sampleRate = session->_inputRate;
    session.channels = channels;
    session->_streamFormat = clientFormat;
    session->_callbackScratch = (float *)calloc((size_t)kMaximumCallbackFrames * 2u, sizeof(float));
    session->_workerScratch = (uint8_t *)calloc((size_t)kConverterInputFrames * 2u, sizeof(float));
    if (session->_callbackScratch == NULL || session->_workerScratch == NULL ||
        !configure_converter(session, &clientFormat)) {
        set_session_error(session, SATURDAY_CAPTURE_UNSUPPORTED_FORMAT);
        (void)dispose_audio_unit(session);
        return false;
    }
    if (AudioUnitInitialize(session->_unit) != noErr) {
        set_session_error(session, SATURDAY_CAPTURE_UNAVAILABLE);
        (void)dispose_audio_unit(session);
        return false;
    }
    atomic_store_explicit(&session->_unitInitialized, true, memory_order_release);
    return true;
}

static bool permission_granted(SVoiceCaptureSession *session) {
    const AVAuthorizationStatus current = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
    if (current == AVAuthorizationStatusAuthorized) {
        return true;
    }
    if (current == AVAuthorizationStatusDenied || current == AVAuthorizationStatusRestricted) {
        set_session_error(session, SATURDAY_CAPTURE_PERMISSION_DENIED);
        emit_event(session, SATURDAY_CAPTURE_EVENT_PERMISSION_DENIED, 0, 0.0f);
        return false;
    }

    atomic_store_explicit(&session->_permissionDone, false, memory_order_release);
    atomic_store_explicit(&session->_permissionGranted, false, memory_order_release);
    [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL granted) {
        if (!atomic_load_explicit(&session->_cancelled, memory_order_acquire)) {
            atomic_store_explicit(&session->_permissionGranted, granted, memory_order_release);
        }
        atomic_store_explicit(&session->_permissionDone, true, memory_order_release);
    }];
    while (!atomic_load_explicit(&session->_cancelled, memory_order_acquire) &&
           !atomic_load_explicit(&session->_permissionDone, memory_order_acquire)) {
        sleep_millis(kPermissionPollMillis);
    }
    if (atomic_load_explicit(&session->_cancelled, memory_order_acquire)) {
        return false;
    }
    if (!atomic_load_explicit(&session->_permissionGranted, memory_order_acquire)) {
        set_session_error(session, SATURDAY_CAPTURE_PERMISSION_DENIED);
        emit_event(session, SATURDAY_CAPTURE_EVENT_PERMISSION_DENIED, 0, 0.0f);
        return false;
    }
    return true;
}

static bool start_audio_unit(SVoiceCaptureSession *session) {
    if (atomic_load_explicit(&session->_cancelled, memory_order_acquire)) {
        return false;
    }
    if (!setup_audio_unit(session)) {
        return false;
    }
    if (atomic_load_explicit(&session->_cancelled, memory_order_acquire)) {
        (void)dispose_audio_unit(session);
        return false;
    }
    if (AudioOutputUnitStart(session->_unit) != noErr) {
        set_session_error(session, SATURDAY_CAPTURE_UNAVAILABLE);
        (void)dispose_audio_unit(session);
        return false;
    }
    atomic_store_explicit(&session->_unitStarted, true, memory_order_release);
    session->_startedAtNanos = monotonic_nanos();
    atomic_store_explicit(&session->_started, true, memory_order_release);
    emit_event(session, SATURDAY_CAPTURE_EVENT_STARTED, 0, 0.0f);
    return !atomic_load_explicit(&session->_cancelled, memory_order_acquire);
}

static bool drain_input_ring(SVoiceCaptureSession *session) {
    bool converted = false;
    while (true) {
        const size_t bytes = ring_read(session, session->_workerScratch,
                                       (size_t)kConverterInputFrames * session->_inputBytesPerFrame);
        if (bytes == 0) {
            break;
        }
        const uint32_t frames = (uint32_t)(bytes / session->_inputBytesPerFrame);
        if (!convert_input_block(session, session->_workerScratch, frames)) {
            set_session_error(session, SATURDAY_CAPTURE_INVALID_AUDIO);
            return false;
        }
        secure_zero_memory(session->_workerScratch, bytes);
        converted = true;
    }
    return converted;
}

static void run_capture_session(SVoiceCaptureSession *session) {
    if (!permission_granted(session) || atomic_load_explicit(&session->_cancelled, memory_order_acquire)) {
        return;
    }
    if (!start_audio_unit(session)) {
        if (!atomic_load_explicit(&session->_cancelled, memory_order_acquire) &&
            atomic_load_explicit(&session->_status, memory_order_acquire) != SATURDAY_CAPTURE_DEVICE_LOST) {
            emit_event(session, SATURDAY_CAPTURE_EVENT_FAILED, 0, 0.0f);
        } else if (atomic_load_explicit(&session->_deviceLost, memory_order_acquire)) {
            emit_event(session, SATURDAY_CAPTURE_EVENT_DEVICE_LOST, 0, 0.0f);
        }
        dispose_audio_unit(session);
        return;
    }

    uint64_t lastMeter = session->_startedAtNanos;
    bool ended = false;
    while (!atomic_load_explicit(&session->_cancelled, memory_order_acquire)) {
        if (atomic_load_explicit(&session->_deviceLost, memory_order_acquire)) {
            set_session_error(session, SATURDAY_CAPTURE_DEVICE_LOST);
            emit_event(session, SATURDAY_CAPTURE_EVENT_DEVICE_LOST, 0, 0.0f);
            ended = true;
            break;
        }
        if (atomic_load_explicit(&session->_formatChanged, memory_order_acquire)) {
            set_session_error(session, SATURDAY_CAPTURE_UNSUPPORTED_FORMAT);
            emit_event(session, SATURDAY_CAPTURE_EVENT_FAILED, 0, 0.0f);
            ended = true;
            break;
        }
        const int32_t status = atomic_load_explicit(&session->_status, memory_order_acquire);
        if (status != SATURDAY_CAPTURE_OK) {
            emit_event(session, status == SATURDAY_CAPTURE_DEVICE_LOST
                ? SATURDAY_CAPTURE_EVENT_DEVICE_LOST
                : SATURDAY_CAPTURE_EVENT_FAILED, 0, 0.0f);
            ended = true;
            break;
        }
        const uint64_t now = monotonic_nanos();
        if (now - lastMeter >= (uint64_t)kMeterIntervalMillis * 1000000ull) {
            const uint64_t elapsed = now > session->_startedAtNanos ? (now - session->_startedAtNanos) / 1000000ull : 0;
            const uint32_t elapsedMillis = (uint32_t)MIN(elapsed, UINT32_MAX);
            const float level = atomic_load_explicit(&session->_meterLevel, memory_order_acquire);
            emit_event(session, SATURDAY_CAPTURE_EVENT_METER, elapsedMillis, isfinite(level) ? fminf(fmaxf(level, 0.0f), 1.0f) : 0.0f);
            lastMeter = now;
        }
        if (atomic_load_explicit(&session->_finishRequested, memory_order_acquire)) {
            break;
        }
        (void)drain_input_ring(session);
        sleep_millis(2);
    }

    if (!dispose_audio_unit(session)) {
        ended = true;
    }
    if (!atomic_load_explicit(&session->_cancelled, memory_order_acquire) && !ended &&
        atomic_load_explicit(&session->_status, memory_order_acquire) == SATURDAY_CAPTURE_OK) {
        (void)drain_input_ring(session);
        if (!flush_converter(session)) {
            set_session_error(session, SATURDAY_CAPTURE_INVALID_AUDIO);
            ended = true;
        }
    }
    if (ended || atomic_load_explicit(&session->_status, memory_order_acquire) != SATURDAY_CAPTURE_OK ||
        atomic_load_explicit(&session->_cancelled, memory_order_acquire)) {
        [session zeroAudioBuffers];
    }
}

static void *capture_manager_main(void *context) {
    @autoreleasepool {
        SVoiceCaptureSession *session = (__bridge_transfer SVoiceCaptureSession *)context;
        run_capture_session(session);
    }
    return NULL;
}

static void *capture_pcm_writer_main(void *context) {
    @autoreleasepool {
        SVoiceCaptureSession *session = (__bridge_transfer SVoiceCaptureSession *)context;
        size_t written = 0;
        const size_t total = (size_t)session->_canonicalCount * sizeof(float);
        const uint8_t *bytes = (const uint8_t *)session->_canonicalSamples;
        bool failed = false;
        while (written < total && !atomic_load_explicit(&session->_cancelled, memory_order_acquire)) {
            struct pollfd descriptor = {.fd = session->_writerFD, .events = POLLOUT, .revents = 0};
            const int ready = poll(&descriptor, 1, 20);
            if (ready < 0 && errno == EINTR) {
                continue;
            }
            if (ready < 0) {
                failed = true;
                break;
            }
            if (ready == 0) {
                continue;
            }
            const ssize_t amount = write(session->_writerFD, bytes + written, total - written);
            if (amount < 0 && (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK)) {
                continue;
            }
            if (amount <= 0) {
                failed = true;
                break;
            }
            written += (size_t)amount;
        }
        if (written != total || failed) {
            set_session_error(session, SATURDAY_CAPTURE_CANCELLED);
        }
        if (session->_writerFD >= 0) {
            close(session->_writerFD);
            session->_writerFD = -1;
        }
        [session zeroAudioBuffers];
        atomic_store_explicit(&session->_writerDone, true, memory_order_release);
    }
    return NULL;
}

static int32_t map_error(int32_t status) {
    if (status < SATURDAY_CAPTURE_OK || status > SATURDAY_CAPTURE_FAILED) {
        return SATURDAY_CAPTURE_FAILED;
    }
    return status;
}

static bool make_converter(uint32_t sampleRate, uint32_t channels, uint32_t maxInputFrames, SAudioConverterBox **outBox) {
    if (outBox == NULL || sampleRate < 8000 || sampleRate > 192000 ||
        (channels != 1 && channels != 2) || maxInputFrames == 0 || maxInputFrames > 4096) {
        return false;
    }
    // Downmix explicitly before resampling. AVAudioConverter's implicit channel
    // conversion can depend on an inferred channel layout when none is supplied.
    AVAudioFormat *input = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                            sampleRate:sampleRate
                                                              channels:1
                                                           interleaved:YES];
    AVAudioFormat *output = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                             sampleRate:16000.0
                                                               channels:1
                                                            interleaved:YES];
    if (input == nil || output == nil) {
        return false;
    }
    AVAudioConverter *converter = [[AVAudioConverter alloc] initFromFormat:input toFormat:output];
    AVAudioPCMBuffer *inputBuffer = [[AVAudioPCMBuffer alloc] initWithPCMFormat:input frameCapacity:maxInputFrames];
    AVAudioPCMBuffer *outputBuffer = [[AVAudioPCMBuffer alloc] initWithPCMFormat:output frameCapacity:kConverterOutputFrames];
    if (converter == nil || inputBuffer == nil || outputBuffer == nil ||
        inputBuffer.floatChannelData == NULL || outputBuffer.floatChannelData == NULL) {
        return false;
    }
    SAudioConverterBox *box = [[SAudioConverterBox alloc] init];
    box.converter = converter;
    box.inputFormat = input;
    box.outputFormat = output;
    box.inputBuffer = inputBuffer;
    box.outputBuffer = outputBuffer;
    box.maxInputFrames = maxInputFrames;
    box.channels = channels;
    box.finished = NO;
    *outBox = box;
    return true;
}

static int32_t converter_convert(SAudioConverterBox *converter,
                                 const float *input,
                                 uint32_t inputFrames,
                                 float *output,
                                 uint32_t outputCapacity,
                                 uint32_t *outFrames) {
    if (converter == nil || input == NULL || output == NULL || outFrames == NULL ||
        converter.finished || inputFrames == 0 || inputFrames > converter.maxInputFrames ||
        outputCapacity < kConverterOutputFrames || converter.inputBuffer.floatChannelData == NULL ||
        converter.outputBuffer.floatChannelData == NULL) {
        return SATURDAY_CAPTURE_INVALID_AUDIO;
    }
    const size_t inputSamples = (size_t)inputFrames * converter.channels;
    for (size_t index = 0; index < inputSamples; ++index) {
        if (!isfinite(input[index]) || input[index] < -1.0f || input[index] > 1.0f) {
            return SATURDAY_CAPTURE_INVALID_AUDIO;
        }
    }
    float *monoInput = converter.inputBuffer.floatChannelData[0];
    if (converter.channels == 1) {
        memcpy(monoInput, input, inputSamples * sizeof(float));
    } else {
        for (uint32_t frame = 0; frame < inputFrames; ++frame) {
            monoInput[frame] = (input[(size_t)frame * 2] + input[(size_t)frame * 2 + 1]) * 0.5f;
        }
    }
    converter.inputBuffer.frameLength = inputFrames;
    converter.outputBuffer.frameLength = 0;
    __block BOOL suppliedInput = NO;
    NSError *error = nil;
    AVAudioConverterOutputStatus status = [converter.converter convertToBuffer:converter.outputBuffer
                                                                          error:&error
                                                             withInputFromBlock:^AVAudioBuffer *(AVAudioPacketCount, AVAudioConverterInputStatus *inputStatus) {
        if (suppliedInput) {
            *inputStatus = AVAudioConverterInputStatus_NoDataNow;
            return nil;
        }
        suppliedInput = YES;
        *inputStatus = AVAudioConverterInputStatus_HaveData;
        return converter.inputBuffer;
    }];
    if (error != nil || status == AVAudioConverterOutputStatus_Error ||
        status == AVAudioConverterOutputStatus_EndOfStream || converter.outputBuffer.frameLength > outputCapacity) {
        return SATURDAY_CAPTURE_INVALID_AUDIO;
    }
    const uint32_t frames = converter.outputBuffer.frameLength;
    float *converted = converter.outputBuffer.floatChannelData[0];
    for (uint32_t index = 0; index < frames; ++index) {
        if (!isfinite(converted[index])) {
            return SATURDAY_CAPTURE_INVALID_AUDIO;
        }
        output[index] = fmaxf(-1.0f, fminf(1.0f, converted[index]));
    }
    *outFrames = frames;
    return SATURDAY_CAPTURE_OK;
}

static int32_t converter_finish(SAudioConverterBox *converter,
                                float *output,
                                uint32_t outputCapacity,
                                uint32_t *outFrames) {
    if (converter == nil || output == NULL || outFrames == NULL ||
        outputCapacity < kConverterOutputFrames) {
        return SATURDAY_CAPTURE_INVALID_AUDIO;
    }
    if (converter.finished) {
        *outFrames = 0;
        return SATURDAY_CAPTURE_OK;
    }
    converter.outputBuffer.frameLength = 0;
    __block BOOL suppliedEnd = NO;
    NSError *error = nil;
    AVAudioConverterOutputStatus status = [converter.converter convertToBuffer:converter.outputBuffer
                                                                          error:&error
                                                             withInputFromBlock:^AVAudioBuffer *(AVAudioPacketCount, AVAudioConverterInputStatus *inputStatus) {
        suppliedEnd = YES;
        *inputStatus = AVAudioConverterInputStatus_EndOfStream;
        return nil;
    }];
    if (error != nil || status == AVAudioConverterOutputStatus_Error ||
        converter.outputBuffer.frameLength > outputCapacity) {
        return SATURDAY_CAPTURE_INVALID_AUDIO;
    }
    const uint32_t frames = converter.outputBuffer.frameLength;
    float *converted = converter.outputBuffer.floatChannelData[0];
    for (uint32_t index = 0; index < frames; ++index) {
        if (!isfinite(converted[index])) {
            return SATURDAY_CAPTURE_INVALID_AUDIO;
        }
        output[index] = fmaxf(-1.0f, fminf(1.0f, converted[index]));
    }
    *outFrames = frames;
    if (status == AVAudioConverterOutputStatus_EndOfStream || (suppliedEnd && frames == 0)) {
        converter.finished = YES;
    }
    return SATURDAY_CAPTURE_OK;
}

static bool make_controller(SaturdayVoiceCapture **outCapture) {
    if (outCapture == NULL) {
        return false;
    }
    SaturdayVoiceCapture *capture = (SaturdayVoiceCapture *)calloc(1, sizeof(SaturdayVoiceCapture));
    if (capture == NULL || pthread_mutex_init(&capture->mutex, NULL) != 0) {
        free(capture);
        return false;
    }
    SVoiceCaptureController *controller = [[SVoiceCaptureController alloc] init];
    controller.devices = @[];
    controller.revision = 0;
    capture->controller = (__bridge_retained void *)controller;
    *outCapture = capture;
    return true;
}

int32_t saturday_voice_capture_create(SaturdayVoiceCapture **out_capture) {
    @autoreleasepool {
        return make_controller(out_capture) ? SATURDAY_CAPTURE_OK : SATURDAY_CAPTURE_UNAVAILABLE;
    }
}

void saturday_voice_capture_destroy(SaturdayVoiceCapture *capture) {
    if (capture == NULL) {
        return;
    }
    if (saturday_voice_capture_cancel(capture, 0) != SATURDAY_CAPTURE_OK) {
        return;
    }
    SVoiceCaptureController *controller = (__bridge_transfer SVoiceCaptureController *)capture->controller;
    capture->controller = NULL;
    controller.activeSession = nil;
    controller.devices = @[];
    pthread_mutex_destroy(&capture->mutex);
    free(capture);
}

int32_t saturday_voice_capture_enumerate(SaturdayVoiceCapture *capture, SaturdayCaptureCatalog *out_catalog) {
    @autoreleasepool {
        if (capture == NULL || out_catalog == NULL) {
            return SATURDAY_CAPTURE_FAILED;
        }
        pthread_mutex_lock(&capture->mutex);
        SVoiceCaptureController *controller = (__bridge SVoiceCaptureController *)capture->controller;
        const bool success = enumerate_devices(controller, out_catalog);
        pthread_mutex_unlock(&capture->mutex);
        return success ? SATURDAY_CAPTURE_OK : SATURDAY_CAPTURE_UNAVAILABLE;
    }
}

int32_t saturday_voice_capture_start(SaturdayVoiceCapture *capture,
                                     const uint8_t handle[SATURDAY_CAPTURE_HANDLE_BYTES],
                                     uint64_t operation_id,
                                     SaturdayCaptureEventCallback callback,
                                     void *callback_context) {
    @autoreleasepool {
        if (capture == NULL || handle == NULL || operation_id == 0 || callback == NULL || callback_context == NULL) {
            return SATURDAY_CAPTURE_FAILED;
        }
        pthread_mutex_lock(&capture->mutex);
        SVoiceCaptureController *controller = (__bridge SVoiceCaptureController *)capture->controller;
        if (controller.activeSession != nil) {
            pthread_mutex_unlock(&capture->mutex);
            return SATURDAY_CAPTURE_BUSY;
        }
        SCaptureDevice *entry = selected_device(controller, handle);
        if (entry == nil) {
            pthread_mutex_unlock(&capture->mutex);
            return SATURDAY_CAPTURE_DEVICE_LOST;
        }
        uint32_t channels = 0;
        AudioDeviceID exactID = kAudioObjectUnknown;
        if (!find_device_for_uid(entry.uid, &exactID, &channels) || exactID != entry.deviceID) {
            pthread_mutex_unlock(&capture->mutex);
            return SATURDAY_CAPTURE_DEVICE_LOST;
        }
        SVoiceCaptureSession *session = [[SVoiceCaptureSession alloc] initWithDevice:exactID
                                                                                 uid:entry.uid
                                                                         operationID:operation_id
                                                                            callback:callback
                                                                             context:callback_context];
        if (session == nil) {
            pthread_mutex_unlock(&capture->mutex);
            return SATURDAY_CAPTURE_UNAVAILABLE;
        }
        controller.activeSession = session;
        const int32_t status = [session begin];
        if (status != SATURDAY_CAPTURE_OK) {
            controller.activeSession = nil;
        }
        pthread_mutex_unlock(&capture->mutex);
        return map_error(status);
    }
}

int32_t saturday_voice_capture_finish(SaturdayVoiceCapture *capture,
                                      uint64_t operation_id,
                                      int32_t *out_read_fd,
                                      uint32_t *out_sample_count,
                                      uint8_t *out_duration_capped) {
    @autoreleasepool {
        if (capture == NULL || operation_id == 0 || out_read_fd == NULL ||
            out_sample_count == NULL || out_duration_capped == NULL) {
            return SATURDAY_CAPTURE_FAILED;
        }
        pthread_mutex_lock(&capture->mutex);
        SVoiceCaptureController *controller = (__bridge SVoiceCaptureController *)capture->controller;
        SVoiceCaptureSession *session = controller.activeSession;
        if (session == nil || session.operationID != operation_id) {
            pthread_mutex_unlock(&capture->mutex);
            return SATURDAY_CAPTURE_DEVICE_LOST;
        }
        [session requestFinish];
        const int32_t joined = [session joinManager];
        if (joined != SATURDAY_CAPTURE_OK) {
            pthread_mutex_unlock(&capture->mutex);
            return map_error(joined);
        }
        const int32_t status = [session makePCMReader:out_read_fd sampleCount:out_sample_count durationCapped:out_duration_capped];
        if (status == SATURDAY_CAPTURE_OK) {
            // The manager is joined and the reader FD now belongs to the caller;
            // the PCM writer retains the session until it finishes.
            controller.activeSession = nil;
        }
        pthread_mutex_unlock(&capture->mutex);
        return map_error(status);
    }
}

int32_t saturday_voice_capture_cancel(SaturdayVoiceCapture *capture, uint64_t operation_id) {
    @autoreleasepool {
        if (capture == NULL) {
            return SATURDAY_CAPTURE_FAILED;
        }
        pthread_mutex_lock(&capture->mutex);
        SVoiceCaptureController *controller = (__bridge SVoiceCaptureController *)capture->controller;
        SVoiceCaptureSession *session = controller.activeSession;
        if (session == nil || (operation_id != 0 && session.operationID != operation_id)) {
            pthread_mutex_unlock(&capture->mutex);
            return SATURDAY_CAPTURE_OK;
        }
        [session requestCancel];
        const int32_t managerStatus = [session joinManager];
        const bool audioDisposed = dispose_audio_unit(session) && session->_unit == NULL &&
            atomic_load_explicit(&session->_callbacksInFlight, memory_order_acquire) == 0;
        const int32_t writerStatus = [session joinWriter];
        if (!audioDisposed || session->_managerThreadStarted || writerStatus == SATURDAY_CAPTURE_FAILED) {
            pthread_mutex_unlock(&capture->mutex);
            return SATURDAY_CAPTURE_FAILED;
        }
        if (session->_readerFD >= 0) {
            close(session->_readerFD);
            session->_readerFD = -1;
        }
        if (session->_writerFD >= 0) {
            close(session->_writerFD);
            session->_writerFD = -1;
        }
        [session zeroAudioBuffers];
        controller.activeSession = nil;
        pthread_mutex_unlock(&capture->mutex);
        (void)managerStatus;
        return SATURDAY_CAPTURE_OK;
    }
}

int32_t saturday_audio_converter_create(uint32_t sample_rate,
                                        uint32_t channels,
                                        uint32_t max_input_frames,
                                        SaturdayAudioConverter **out_converter) {
    @autoreleasepool {
        if (out_converter == NULL) {
            return SATURDAY_CAPTURE_FAILED;
        }
        SAudioConverterBox *box = nil;
        if (!make_converter(sample_rate, channels, max_input_frames, &box)) {
            return SATURDAY_CAPTURE_UNSUPPORTED_FORMAT;
        }
        SaturdayAudioConverter *converter = (SaturdayAudioConverter *)calloc(1, sizeof(SaturdayAudioConverter));
        if (converter == NULL) {
            return SATURDAY_CAPTURE_UNAVAILABLE;
        }
        converter->object = (__bridge_retained void *)box;
        *out_converter = converter;
        return SATURDAY_CAPTURE_OK;
    }
}

int32_t saturday_audio_converter_convert(SaturdayAudioConverter *converter,
                                         const float *input,
                                         uint32_t input_frames,
                                         float *output,
                                         uint32_t output_capacity,
                                         uint32_t *out_output_frames) {
    @autoreleasepool {
        if (converter == NULL || converter->object == NULL) {
            return SATURDAY_CAPTURE_INVALID_AUDIO;
        }
        SAudioConverterBox *box = (__bridge SAudioConverterBox *)converter->object;
        return converter_convert(box, input, input_frames, output, output_capacity, out_output_frames);
    }
}

int32_t saturday_audio_converter_finish(SaturdayAudioConverter *converter,
                                        float *output,
                                        uint32_t output_capacity,
                                        uint32_t *out_output_frames) {
    @autoreleasepool {
        if (converter == NULL || converter->object == NULL) {
            return SATURDAY_CAPTURE_INVALID_AUDIO;
        }
        SAudioConverterBox *box = (__bridge SAudioConverterBox *)converter->object;
        return converter_finish(box, output, output_capacity, out_output_frames);
    }
}

void saturday_audio_converter_destroy(SaturdayAudioConverter *converter) {
    if (converter == NULL) {
        return;
    }
    if (converter->object != NULL) {
        CFRelease((CFTypeRef)converter->object);
    }
    secure_zero_memory(converter, sizeof(*converter));
    free(converter);
}
