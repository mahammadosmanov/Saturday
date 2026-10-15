#import "voice_speech.h"

#import <AVFoundation/AVFoundation.h>
#import <CoreAudio/CoreAudio.h>
#import <Foundation/Foundation.h>
#import <dispatch/dispatch.h>

#include <stdlib.h>
#include <string.h>
#include <stdatomic.h>

static const uint32_t kSpeechErrorNone = 0;
static const uint32_t kSpeechErrorUnavailable = 1;
static const uint32_t kSpeechErrorVoiceUnavailable = 2;
static const uint32_t kSpeechErrorWrongLocale = 3;
static const uint32_t kSpeechErrorRouteLost = 4;
static const uint32_t kSpeechErrorInternal = 5;

@interface SaturdaySpeechItem : NSObject
@property(nonatomic, copy) NSString *sessionID;
@property(nonatomic, copy) NSString *turnID;
@property(nonatomic, copy) NSString *voiceID;
@property(nonatomic, copy) NSString *locale;
@property(nonatomic, copy) NSString *text;
@property(nonatomic, strong) AVSpeechUtterance *utterance;
@property(nonatomic) uint64_t generation;
@property(nonatomic) uint64_t operationID;
@property(nonatomic) uint64_t utteranceID;
@property(nonatomic) uint32_t language;
@property(nonatomic) BOOL started;
@end

@implementation SaturdaySpeechItem
@end

@interface SaturdaySpeechObject : NSObject <AVSpeechSynthesizerDelegate>
@property(nonatomic, strong) AVSpeechSynthesizer *synthesizer;
@property(nonatomic, strong) NSMutableArray<SaturdaySpeechItem *> *pending;
@property(nonatomic, strong) SaturdaySpeechItem *active;
@property(nonatomic) SaturdaySpeechCallback callback;
@property(nonatomic) void *userData;
@property(nonatomic) SaturdaySpeechHandle *nativeHandle;
@property(nonatomic) AudioDeviceID observedOutputID;
@property(nonatomic, copy) AudioObjectPropertyListenerBlock defaultOutputListener;
@property(nonatomic, copy) AudioObjectPropertyListenerBlock outputAliveListener;
@property(nonatomic) BOOL routeListenersInstalled;
@end

@implementation SaturdaySpeechObject

- (instancetype)initWithCallback:(SaturdaySpeechCallback)callback userData:(void *)userData {
    self = [super init];
    if (self != nil) {
        _callback = callback;
        _userData = userData;
        _pending = [[NSMutableArray alloc] initWithCapacity:2];
        _synthesizer = [[AVSpeechSynthesizer alloc] init];
        _synthesizer.delegate = self;
    }
    return self;
}

@end

@interface SaturdaySpeechStopWaiter : NSObject
@property(nonatomic, strong) dispatch_semaphore_t semaphore;
@property(nonatomic) NSUInteger attemptsRemaining;
@property(nonatomic) SaturdaySpeechResult result;
@property(nonatomic) BOOL completed;
@end

@implementation SaturdaySpeechStopWaiter
@end


struct SaturdaySpeechHandle {
    void *object;
    atomic_uint credits;
    atomic_uint stopped;
};
static void saturday_release_credits(SaturdaySpeechObject *object, uint32_t count) {
    if (object.nativeHandle != NULL && count > 0) {
        atomic_fetch_add_explicit(&object.nativeHandle->credits, count, memory_order_release);
    }
}

static void saturday_on_main(dispatch_block_t block) {
    if ([NSThread isMainThread]) {
        block();
    } else {
        dispatch_sync(dispatch_get_main_queue(), block);
    }
}

static BOOL saturday_refresh_stopped_state(SaturdaySpeechObject *object) {
    if (object == nil || object.nativeHandle == NULL) {
        return NO;
    }
    BOOL stopped = object.active == nil &&
                   !object.synthesizer.isSpeaking &&
                   !object.synthesizer.isPaused;
    atomic_store_explicit(&object.nativeHandle->stopped, stopped ? 1u : 0u, memory_order_release);
    return stopped;
}

static void saturday_complete_stop_waiter(
    SaturdaySpeechStopWaiter *waiter,
    SaturdaySpeechResult result) {
    if (waiter == nil || waiter.completed) {
        return;
    }
    waiter.completed = YES;
    waiter.result = result;
    dispatch_semaphore_signal(waiter.semaphore);
}

static void saturday_schedule_stop_check(
    SaturdaySpeechObject *object,
    SaturdaySpeechStopWaiter *waiter) {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_MSEC),
                   dispatch_get_main_queue(), ^{
        if (saturday_refresh_stopped_state(object)) {
            saturday_complete_stop_waiter(waiter, SATURDAY_SPEECH_OK);
            return;
        }
        if (waiter.attemptsRemaining <= 1) {
            saturday_complete_stop_waiter(waiter, SATURDAY_SPEECH_STOP_UNVERIFIED);
            return;
        }
        waiter.attemptsRemaining -= 1;
        saturday_schedule_stop_check(object, waiter);
    });
}

static SaturdaySpeechStopWaiter *saturday_new_stop_waiter(void) {
    SaturdaySpeechStopWaiter *waiter = [[SaturdaySpeechStopWaiter alloc] init];
    waiter.semaphore = dispatch_semaphore_create(0);
    waiter.attemptsRemaining = 50;
    waiter.result = SATURDAY_SPEECH_STOP_UNVERIFIED;
    return waiter;
}

static NSString *saturday_string(const uint8_t *bytes, size_t length, size_t maximum) {
    if (bytes == NULL || length == 0 || length > maximum || memchr(bytes, 0, length) != NULL) {
        return nil;
    }
    NSData *data = [NSData dataWithBytes:bytes length:length];
    return [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
}

static BOOL saturday_copy_utf8(NSString *value, char *output, size_t capacity, uint32_t *out_length) {
    if (value == nil || output == NULL || capacity == 0) {
        return NO;
    }
    NSData *data = [value dataUsingEncoding:NSUTF8StringEncoding allowLossyConversion:NO];
    if (data == nil || data.length >= capacity) {
        return NO;
    }
    memset(output, 0, capacity);
    memcpy(output, data.bytes, data.length);
    if (out_length != NULL) {
        *out_length = (uint32_t)data.length;
    }
    return YES;
}

static NSUInteger saturday_scalar_count(NSString *value) {
    NSUInteger count = 0;
    NSUInteger index = 0;
    while (index < value.length) {
        unichar current = [value characterAtIndex:index];
        if (CFStringIsSurrogateHighCharacter(current) && index + 1 < value.length &&
            CFStringIsSurrogateLowCharacter([value characterAtIndex:index + 1])) {
            index += 2;
        } else {
            index += 1;
        }
        count += 1;
    }
    return count;
}

static BOOL saturday_has_forbidden_text_scalar(NSString *value) {
    for (NSUInteger index = 0; index < value.length; index += 1) {
        unichar current = [value characterAtIndex:index];
        if (CFStringIsSurrogateHighCharacter(current) && index + 1 < value.length &&
            CFStringIsSurrogateLowCharacter([value characterAtIndex:index + 1])) {
            index += 1;
            continue;
        }
        if ((current < 0x20 && current != '\n' && current != '\t') ||
            (current >= 0x7F && current <= 0x9F)) {
            return YES;
        }
        if (current == 0x00AD || current == 0x034F || current == 0x061C ||
            (current >= 0x200B && current <= 0x200F) ||
            (current >= 0x202A && current <= 0x202E) ||
            (current >= 0x2060 && current <= 0x206F) || current == 0xFEFF) {
            return YES;
        }
    }
    return NO;
}

static BOOL saturday_owner_is_valid(
    const uint8_t *sessionID,
    size_t sessionIDLength,
    const uint8_t *turnID,
    size_t turnIDLength) {
    return sessionID != NULL && turnID != NULL && sessionIDLength > 0 && turnIDLength > 0 &&
           sessionIDLength <= SATURDAY_SPEECH_MAX_OWNER_ID_BYTES &&
           turnIDLength <= SATURDAY_SPEECH_MAX_OWNER_ID_BYTES &&
           memchr(sessionID, 0, sessionIDLength) == NULL && memchr(turnID, 0, turnIDLength) == NULL &&
           [[NSString alloc] initWithBytes:sessionID length:sessionIDLength encoding:NSUTF8StringEncoding] != nil &&
           [[NSString alloc] initWithBytes:turnID length:turnIDLength encoding:NSUTF8StringEncoding] != nil;
}

static void saturday_emit(
    SaturdaySpeechObject *object,
    SaturdaySpeechItem *item,
    uint32_t kind,
    uint32_t errorCode) {
    if (object.callback == NULL || item == nil) {
        return;
    }
    SaturdaySpeechEvent event = {0};
    event.abi_version = SATURDAY_SPEECH_ABI_VERSION;
    event.struct_size = sizeof(event);
    event.kind = kind;
    event.error_code = errorCode;
    event.generation = item.generation;
    event.operation_id = item.operationID;
    event.utterance_id = item.utteranceID;
    if (!saturday_copy_utf8(item.sessionID, event.session_id, sizeof(event.session_id), &event.session_id_length) ||
        !saturday_copy_utf8(item.turnID, event.turn_id, sizeof(event.turn_id), &event.turn_id_length)) {
        return;
    }
    object.callback(object.userData, &event);
}

static AudioObjectPropertyAddress saturday_default_output_address(void) {
    AudioObjectPropertyAddress address = {
        kAudioHardwarePropertyDefaultOutputDevice,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain
    };
    return address;
}

static AudioObjectPropertyAddress saturday_output_alive_address(void) {
    AudioObjectPropertyAddress address = {
        kAudioDevicePropertyDeviceIsAlive,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain
    };
    return address;
}

static BOOL saturday_get_default_output(AudioDeviceID *out_device) {
    AudioObjectPropertyAddress address = saturday_default_output_address();
    UInt32 dataSize = sizeof(AudioDeviceID);
    AudioDeviceID device = kAudioObjectUnknown;
    OSStatus status = AudioObjectGetPropertyData(
        kAudioObjectSystemObject, &address, 0, NULL, &dataSize, &device);
    if (status != noErr || dataSize != sizeof(AudioDeviceID) || device == kAudioObjectUnknown) {
        return NO;
    }
    *out_device = device;
    return YES;
}

static BOOL saturday_output_is_alive(AudioDeviceID device) {
    AudioObjectPropertyAddress address = saturday_output_alive_address();
    UInt32 dataSize = sizeof(UInt32);
    UInt32 alive = 0;
    OSStatus status = AudioObjectGetPropertyData(device, &address, 0, NULL, &dataSize, &alive);
    return status == noErr && dataSize == sizeof(UInt32) && alive == 1;
}

static void saturday_remove_route_listeners(SaturdaySpeechObject *object) {
    if (!object.routeListenersInstalled) {
        return;
    }
    AudioObjectPropertyAddress defaultAddress = saturday_default_output_address();
    AudioObjectRemovePropertyListenerBlock(
        kAudioObjectSystemObject,
        &defaultAddress,
        dispatch_get_main_queue(),
        object.defaultOutputListener);
    if (object.observedOutputID != kAudioObjectUnknown && object.outputAliveListener != nil) {
        AudioObjectPropertyAddress aliveAddress = saturday_output_alive_address();
        AudioObjectRemovePropertyListenerBlock(
            object.observedOutputID,
            &aliveAddress,
            dispatch_get_main_queue(),
            object.outputAliveListener);
    }
    object.defaultOutputListener = nil;
    object.outputAliveListener = nil;
    object.observedOutputID = kAudioObjectUnknown;
    object.routeListenersInstalled = NO;
}

static void saturday_start_next(SaturdaySpeechObject *object);

static void saturday_route_lost(SaturdaySpeechObject *object) {
    if (object == nil) {
        return;
    }
    NSMutableArray<SaturdaySpeechItem *> *failed = [[NSMutableArray alloc] initWithCapacity:3];
    if (object.active != nil) {
        [failed addObject:object.active];
        object.active = nil;
    }
    [failed addObjectsFromArray:object.pending];
    [object.pending removeAllObjects];
    saturday_release_credits(object, (uint32_t)failed.count);
    [object.synthesizer stopSpeakingAtBoundary:AVSpeechBoundaryImmediate];
    saturday_remove_route_listeners(object);
    if (!saturday_refresh_stopped_state(object)) {
        saturday_schedule_stop_check(object, saturday_new_stop_waiter());
    }
    for (SaturdaySpeechItem *item in failed) {
        saturday_emit(object, item, SATURDAY_SPEECH_EVENT_ROUTE_LOST, kSpeechErrorRouteLost);
    }
}

static BOOL saturday_install_route_listeners(SaturdaySpeechObject *object, AudioDeviceID device) {
    AudioObjectPropertyAddress defaultAddress = saturday_default_output_address();
    AudioObjectPropertyAddress aliveAddress = saturday_output_alive_address();
    __weak SaturdaySpeechObject *weakObject = object;
    object.defaultOutputListener = ^(UInt32 count, const AudioObjectPropertyAddress *addresses) {
        (void)count;
        (void)addresses;
        SaturdaySpeechObject *strongObject = weakObject;
        if (strongObject == nil || strongObject.active == nil) {
            return;
        }
        AudioDeviceID current = kAudioObjectUnknown;
        if (!saturday_get_default_output(&current) || current != strongObject.observedOutputID ||
            !saturday_output_is_alive(current)) {
            saturday_route_lost(strongObject);
        }
    };
    object.outputAliveListener = ^(UInt32 count, const AudioObjectPropertyAddress *addresses) {
        (void)count;
        (void)addresses;
        SaturdaySpeechObject *strongObject = weakObject;
        if (strongObject == nil || strongObject.active == nil ||
            !saturday_output_is_alive(strongObject.observedOutputID)) {
            saturday_route_lost(strongObject);
        }
    };
    OSStatus defaultStatus = AudioObjectAddPropertyListenerBlock(
        kAudioObjectSystemObject,
        &defaultAddress,
        dispatch_get_main_queue(),
        object.defaultOutputListener);
    if (defaultStatus != noErr) {
        object.defaultOutputListener = nil;
        object.outputAliveListener = nil;
        return NO;
    }
    OSStatus aliveStatus = AudioObjectAddPropertyListenerBlock(
        device,
        &aliveAddress,
        dispatch_get_main_queue(),
        object.outputAliveListener);
    if (aliveStatus != noErr) {
        AudioObjectRemovePropertyListenerBlock(
            kAudioObjectSystemObject,
            &defaultAddress,
            dispatch_get_main_queue(),
            object.defaultOutputListener);
        object.defaultOutputListener = nil;
        object.outputAliveListener = nil;
        return NO;
    }
    object.observedOutputID = device;
    object.routeListenersInstalled = YES;
    return YES;
}

static uint32_t saturday_language_for_locale(NSString *locale) {
    NSString *language = [[[locale componentsSeparatedByString:@"-"] firstObject] lowercaseString];
    if ([language isEqualToString:@"tr"]) {
        return SATURDAY_SPEECH_LANGUAGE_TR;
    }
    if ([language isEqualToString:@"en"]) {
        return SATURDAY_SPEECH_LANGUAGE_EN;
    }
    return 0;
}

static BOOL saturday_known_pre14_voice(NSString *identifier, uint32_t language) {
    static NSSet<NSString *> *knownTurkish;
    static NSSet<NSString *> *knownEnglish;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        knownTurkish = [NSSet setWithObject:@"com.apple.voice.super-compact.tr-TR.Yelda"];
        knownEnglish = [NSSet setWithArray:@[
            @"com.apple.voice.compact.en-US.Samantha",
            @"com.apple.voice.super-compact.en-US.Samantha"
        ]];
    });
    return language == SATURDAY_SPEECH_LANGUAGE_TR ? [knownTurkish containsObject:identifier]
         : language == SATURDAY_SPEECH_LANGUAGE_EN ? [knownEnglish containsObject:identifier]
                                                   : NO;
}

static BOOL saturday_voice_is_supported(AVSpeechSynthesisVoice *voice, uint32_t *out_language) {
    if (voice == nil || voice.identifier.length == 0 || voice.language.length == 0) {
        return NO;
    }
    uint32_t language = saturday_language_for_locale(voice.language);
    if (language == 0) {
        return NO;
    }
    // Avoid Clang's availability runtime helper: the Rust final link does not include it.
    // NSProcessInfo is available at the macOS 12 deployment target.
    NSOperatingSystemVersion osVersion = [NSProcessInfo processInfo].operatingSystemVersion;
    if (osVersion.majorVersion >= 14) {
        AVSpeechSynthesisVoiceTraits traits = voice.voiceTraits;
        if ((traits & AVSpeechSynthesisVoiceTraitIsPersonalVoice) != 0 ||
            (traits & AVSpeechSynthesisVoiceTraitIsNoveltyVoice) != 0) {
            return NO;
        }
    } else if (!saturday_known_pre14_voice(voice.identifier, language)) {
        return NO;
    }
    NSData *identifier = [voice.identifier dataUsingEncoding:NSUTF8StringEncoding allowLossyConversion:NO];
    NSData *name = [voice.name dataUsingEncoding:NSUTF8StringEncoding allowLossyConversion:NO];
    NSData *locale = [voice.language dataUsingEncoding:NSUTF8StringEncoding allowLossyConversion:NO];
    if (identifier == nil || identifier.length == 0 || identifier.length > SATURDAY_SPEECH_MAX_VOICE_ID_BYTES ||
        name == nil || name.length == 0 || name.length > SATURDAY_SPEECH_MAX_DISPLAY_NAME_BYTES ||
        locale == nil || locale.length == 0 || locale.length > SATURDAY_SPEECH_MAX_LOCALE_BYTES) {
        return NO;
    }
    if (out_language != NULL) {
        *out_language = language;
    }
    return YES;
}

static NSComparisonResult saturday_compare_voice(AVSpeechSynthesisVoice *left, AVSpeechSynthesisVoice *right) {
    return [left.identifier compare:right.identifier options:NSLiteralSearch];
}

static uint64_t saturday_catalog_revision(NSArray<AVSpeechSynthesisVoice *> *voices) {
    uint64_t hash = UINT64_C(14695981039346656037);
    const uint64_t prime = UINT64_C(1099511628211);
    for (AVSpeechSynthesisVoice *voice in voices) {
        NSArray<NSString *> *fields = @[voice.identifier, voice.name, voice.language];
        for (NSString *field in fields) {
            NSData *data = [field dataUsingEncoding:NSUTF8StringEncoding];
            const uint8_t *bytes = data.bytes;
            for (NSUInteger index = 0; index < data.length; index += 1) {
                hash ^= bytes[index];
                hash *= prime;
            }
            hash ^= 0;
            hash *= prime;
        }
    }
    return hash == 0 ? 1 : hash;
}

static NSArray<AVSpeechSynthesisVoice *> *saturday_supported_voices(SaturdaySpeechResult *out_result) {
    NSMutableArray<AVSpeechSynthesisVoice *> *eligible = [[NSMutableArray alloc] initWithCapacity:SATURDAY_SPEECH_MAX_VOICES];
    NSMutableSet<NSString *> *identifiers = [[NSMutableSet alloc] initWithCapacity:SATURDAY_SPEECH_MAX_VOICES];
    for (AVSpeechSynthesisVoice *voice in [AVSpeechSynthesisVoice speechVoices]) {
        uint32_t language = 0;
        if (!saturday_voice_is_supported(voice, &language) || [identifiers containsObject:voice.identifier]) {
            continue;
        }
        [identifiers addObject:voice.identifier];
        if (eligible.count == SATURDAY_SPEECH_MAX_VOICES) {
            *out_result = SATURDAY_SPEECH_BUFFER_TOO_SMALL;
            return nil;
        }
        [eligible addObject:voice];
    }
    [eligible sortUsingComparator:^NSComparisonResult(AVSpeechSynthesisVoice *left, AVSpeechSynthesisVoice *right) {
        return saturday_compare_voice(left, right);
    }];
    *out_result = SATURDAY_SPEECH_OK;
    return eligible;
}

static BOOL saturday_item_matches_owner(
    SaturdaySpeechItem *item,
    NSString *sessionID,
    NSString *turnID,
    uint64_t generation) {
    return [item.sessionID isEqualToString:sessionID] && [item.turnID isEqualToString:turnID] &&
           item.generation == generation;
}

static uint32_t saturday_start_item(SaturdaySpeechObject *object, SaturdaySpeechItem *item) {
    AudioDeviceID device = kAudioObjectUnknown;
    if (!saturday_get_default_output(&device) || !saturday_output_is_alive(device)) {
        return kSpeechErrorRouteLost;
    }
    if (object.observedOutputID == kAudioObjectUnknown) {
        if (!saturday_install_route_listeners(object, device)) {
            return kSpeechErrorUnavailable;
        }
    } else if (object.observedOutputID != device) {
        return kSpeechErrorRouteLost;
    }

    AVSpeechSynthesisVoice *voice = [AVSpeechSynthesisVoice voiceWithIdentifier:item.voiceID];
    if (voice == nil || ![voice.identifier isEqualToString:item.voiceID]) {
        return kSpeechErrorVoiceUnavailable;
    }
    uint32_t resolvedLanguage = 0;
    if (!saturday_voice_is_supported(voice, &resolvedLanguage) ||
        resolvedLanguage != item.language || ![voice.language isEqualToString:item.locale]) {
        return kSpeechErrorWrongLocale;
    }

    AVSpeechUtterance *utterance = [[AVSpeechUtterance alloc] initWithString:item.text];
    if (utterance == nil) {
        return kSpeechErrorInternal;
    }
    utterance.voice = voice;
    utterance.prefersAssistiveTechnologySettings = NO;
    item.utterance = utterance;
    item.started = NO;
    object.active = item;
    saturday_refresh_stopped_state(object);
    @try {
        [object.synthesizer speakUtterance:utterance];
    } @catch (__unused NSException *exception) {
        object.active = nil;
        item.utterance = nil;
        saturday_refresh_stopped_state(object);
        return kSpeechErrorInternal;
    }
    return kSpeechErrorNone;
}

static void saturday_start_next(SaturdaySpeechObject *object) {
    while (object.active == nil && object.pending.count > 0) {
        SaturdaySpeechItem *next = object.pending[0];
        [object.pending removeObjectAtIndex:0];
        uint32_t error = saturday_start_item(object, next);
        if (error == kSpeechErrorNone) {
            return;
        }
        saturday_release_credits(object, 1);
        saturday_emit(
            object,
            next,
            error == kSpeechErrorRouteLost ? SATURDAY_SPEECH_EVENT_ROUTE_LOST : SATURDAY_SPEECH_EVENT_FAILED,
            error);
    }
    if (object.active == nil && object.pending.count == 0) {
        saturday_remove_route_listeners(object);
        saturday_refresh_stopped_state(object);
    }
}

static uint32_t saturday_validate_item_text(NSString *text, size_t utf8Length) {
    if (text == nil || utf8Length == 0 || utf8Length > SATURDAY_SPEECH_MAX_SEGMENT_BYTES ||
        saturday_scalar_count(text) > SATURDAY_SPEECH_MAX_SEGMENT_SCALARS ||
        saturday_has_forbidden_text_scalar(text)) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }
    return kSpeechErrorNone;
}

static BOOL saturday_has_owner(SaturdaySpeechObject *object) {
    return object.active != nil || object.pending.count > 0;
}

static SaturdaySpeechItem *saturday_first_item(SaturdaySpeechObject *object) {
    if (object.active != nil) {
        return object.active;
    }
    return object.pending.firstObject;
}

static BOOL saturday_same_queue_owner(
    SaturdaySpeechObject *object,
    NSString *sessionID,
    NSString *turnID,
    uint64_t generation) {
    if (!saturday_has_owner(object)) {
        return YES;
    }
    return saturday_item_matches_owner(saturday_first_item(object), sessionID, turnID, generation);
}

static void saturday_remove_matching_items(
    SaturdaySpeechObject *object,
    NSString *sessionID,
    NSString *turnID,
    uint64_t generation,
    NSMutableArray<SaturdaySpeechItem *> *removed) {
    if (object.active != nil &&
        saturday_item_matches_owner(object.active, sessionID, turnID, generation)) {
        [removed addObject:object.active];
        object.active = nil;
    }
    NSIndexSet *indexes = [object.pending indexesOfObjectsPassingTest:^BOOL(SaturdaySpeechItem *item, NSUInteger index, BOOL *stop) {
        (void)index;
        (void)stop;
        return saturday_item_matches_owner(item, sessionID, turnID, generation);
    }];
    if (indexes.count > 0) {
        [removed addObjectsFromArray:[object.pending objectsAtIndexes:indexes]];
        [object.pending removeObjectsAtIndexes:indexes];
    }
}

static void saturday_cancel_owner_on_main(
    SaturdaySpeechObject *object,
    NSString *sessionID,
    NSString *turnID,
    uint64_t generation,
    SaturdaySpeechStopWaiter *waiter) {
    NSMutableArray<SaturdaySpeechItem *> *removed = [[NSMutableArray alloc] initWithCapacity:3];
    saturday_remove_matching_items(object, sessionID, turnID, generation, removed);
    if (removed.count == 0) {
        saturday_complete_stop_waiter(waiter, SATURDAY_SPEECH_OK);
        return;
    }
    saturday_release_credits(object, (uint32_t)removed.count);
    BOOL hadActive = NO;
    for (SaturdaySpeechItem *item in removed) {
        if (item.utterance != nil) {
            hadActive = YES;
        }
    }
    if (hadActive) {
        [object.synthesizer stopSpeakingAtBoundary:AVSpeechBoundaryImmediate];
    }
    for (SaturdaySpeechItem *item in removed) {
        item.utterance = nil;
        saturday_emit(object, item, SATURDAY_SPEECH_EVENT_CANCELLED, kSpeechErrorNone);
    }
    if (object.active == nil && object.pending.count == 0) {
        saturday_remove_route_listeners(object);
    }
    if (!hadActive || saturday_refresh_stopped_state(object)) {
        saturday_complete_stop_waiter(waiter, SATURDAY_SPEECH_OK);
        return;
    }
    waiter.attemptsRemaining = 50;
    saturday_schedule_stop_check(object, waiter);
}

static void saturday_finish_destroy(
    SaturdaySpeechObject *object,
    SaturdaySpeechStopWaiter *waiter) {
    if (!saturday_refresh_stopped_state(object)) {
        saturday_complete_stop_waiter(waiter, SATURDAY_SPEECH_STOP_UNVERIFIED);
        return;
    }
    object.synthesizer.delegate = nil;
    object.callback = NULL;
    object.userData = NULL;
    object.nativeHandle = NULL;
    saturday_complete_stop_waiter(waiter, SATURDAY_SPEECH_OK);
}

static void saturday_schedule_destroy_check(
    SaturdaySpeechObject *object,
    SaturdaySpeechStopWaiter *waiter) {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_MSEC), dispatch_get_main_queue(), ^{
        if (saturday_refresh_stopped_state(object)) {
            saturday_finish_destroy(object, waiter);
        } else if (waiter.attemptsRemaining > 1) {
            waiter.attemptsRemaining -= 1;
            saturday_schedule_destroy_check(object, waiter);
        } else {
            saturday_complete_stop_waiter(waiter, SATURDAY_SPEECH_STOP_UNVERIFIED);
        }
    });
}

static void saturday_destroy_on_main(
    SaturdaySpeechObject *object,
    SaturdaySpeechStopWaiter *waiter) {
    NSMutableArray<SaturdaySpeechItem *> *removed = [[NSMutableArray alloc] initWithCapacity:3];
    if (object.active != nil) {
        [removed addObject:object.active];
        object.active = nil;
    }
    [removed addObjectsFromArray:object.pending];
    [object.pending removeAllObjects];
    saturday_release_credits(object, (uint32_t)removed.count);
    [object.synthesizer stopSpeakingAtBoundary:AVSpeechBoundaryImmediate];
    for (SaturdaySpeechItem *item in removed) {
        item.utterance = nil;
        saturday_emit(object, item, SATURDAY_SPEECH_EVENT_CANCELLED, kSpeechErrorNone);
    }
    saturday_remove_route_listeners(object);
    if (saturday_refresh_stopped_state(object)) {
        saturday_finish_destroy(object, waiter);
        return;
    }
    waiter.attemptsRemaining = 50;
    saturday_schedule_destroy_check(object, waiter);
}

SaturdaySpeechResult saturday_speech_create(
    SaturdaySpeechCallback callback,
    void *user_data,
    SaturdaySpeechHandle **out_handle) {
    if (callback == NULL || out_handle == NULL) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }
    __block SaturdaySpeechObject *object = nil;
    saturday_on_main(^{
        object = [[SaturdaySpeechObject alloc] initWithCallback:callback userData:user_data];
    });
    if (object == nil) {
        return SATURDAY_SPEECH_UNAVAILABLE;
    }
    SaturdaySpeechHandle *handle = calloc(1, sizeof(*handle));
    if (handle == NULL) {
        return SATURDAY_SPEECH_UNAVAILABLE;
    }
    handle->object = (__bridge_retained void *)object;
    atomic_init(&handle->credits, SATURDAY_SPEECH_QUEUE_CAPACITY);
    atomic_init(&handle->stopped, 1);
    saturday_on_main(^{
        object.nativeHandle = handle;
        saturday_refresh_stopped_state(object);
    });
    *out_handle = handle;
    return SATURDAY_SPEECH_OK;
}

SaturdaySpeechResult saturday_speech_enumerate_voices(
    SaturdaySpeechVoiceRecord *records,
    uint32_t capacity,
    uint32_t *out_count,
    uint64_t *out_revision) {
    if (out_count == NULL || out_revision == NULL || (capacity > 0 && records == NULL)) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }
    __block NSArray<AVSpeechSynthesisVoice *> *voices = nil;
    __block SaturdaySpeechResult result = SATURDAY_SPEECH_OK;
    saturday_on_main(^{
        voices = saturday_supported_voices(&result);
    });
    if (result != SATURDAY_SPEECH_OK || voices == nil) {
        *out_count = 0;
        *out_revision = 0;
        return result;
    }
    *out_count = (uint32_t)voices.count;
    *out_revision = saturday_catalog_revision(voices);
    if (capacity < voices.count) {
        return SATURDAY_SPEECH_BUFFER_TOO_SMALL;
    }
    for (NSUInteger index = 0; index < voices.count; index += 1) {
        AVSpeechSynthesisVoice *voice = voices[index];
        uint32_t language = 0;
        if (!saturday_voice_is_supported(voice, &language)) {
            return SATURDAY_SPEECH_INTERNAL_ERROR;
        }
        SaturdaySpeechVoiceRecord record = {0};
        record.abi_version = SATURDAY_SPEECH_ABI_VERSION;
        record.struct_size = sizeof(record);
        record.language = language;
        if (!saturday_copy_utf8(voice.identifier, record.identifier, sizeof(record.identifier), NULL) ||
            !saturday_copy_utf8(voice.name, record.display_name, sizeof(record.display_name), NULL) ||
            !saturday_copy_utf8(voice.language, record.locale, sizeof(record.locale), NULL)) {
            return SATURDAY_SPEECH_INTERNAL_ERROR;
        }
        records[index] = record;
    }
    return SATURDAY_SPEECH_OK;
}

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
    uint64_t utterance_id) {
    if (handle == NULL || handle->object == NULL || owner == NULL || utterance_id == 0 ||
        owner->generation == 0 || owner->operation_id == 0 ||
        !saturday_owner_is_valid(owner->session_id, owner->session_id_length, owner->turn_id, owner->turn_id_length) ||
        (language != SATURDAY_SPEECH_LANGUAGE_TR && language != SATURDAY_SPEECH_LANGUAGE_EN)) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }
    NSString *sessionID = saturday_string(owner->session_id, owner->session_id_length, SATURDAY_SPEECH_MAX_OWNER_ID_BYTES);
    NSString *turnID = saturday_string(owner->turn_id, owner->turn_id_length, SATURDAY_SPEECH_MAX_OWNER_ID_BYTES);
    NSString *voiceID = saturday_string(voice_id, voice_id_length, SATURDAY_SPEECH_MAX_VOICE_ID_BYTES);
    NSString *localeString = saturday_string(locale, locale_length, SATURDAY_SPEECH_MAX_LOCALE_BYTES);
    NSString *textString = saturday_string(text, text_length, SATURDAY_SPEECH_MAX_SEGMENT_BYTES);
    if (sessionID == nil || turnID == nil || voiceID == nil || localeString == nil ||
        saturday_language_for_locale(localeString) != language ||
        saturday_validate_item_text(textString, text_length) != kSpeechErrorNone) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }

    __block SaturdaySpeechResult result = SATURDAY_SPEECH_OK;
    SaturdaySpeechObject *object = (__bridge SaturdaySpeechObject *)handle->object;
    saturday_on_main(^{
        if (!saturday_same_queue_owner(object, sessionID, turnID, owner->generation)) {
            result = SATURDAY_SPEECH_INVALID_ARGUMENT;
            return;
        }
        if (object.pending.count + (object.active == nil ? 0 : 1) >= SATURDAY_SPEECH_QUEUE_CAPACITY) {
            result = SATURDAY_SPEECH_QUEUE_FULL;
            return;
        }
        if (object.active != nil &&
            (object.active.operationID == owner->operation_id || object.active.utteranceID == utterance_id)) {
            result = SATURDAY_SPEECH_INVALID_ARGUMENT;
            return;
        }
        if (atomic_load_explicit(&handle->credits, memory_order_acquire) == 0) {
            result = SATURDAY_SPEECH_QUEUE_FULL;
            return;
        }
        for (SaturdaySpeechItem *queued in object.pending) {
            if (queued.operationID == owner->operation_id || queued.utteranceID == utterance_id) {
                result = SATURDAY_SPEECH_INVALID_ARGUMENT;
                return;
            }
        }
        SaturdaySpeechItem *item = [[SaturdaySpeechItem alloc] init];
        item.sessionID = sessionID;
        item.turnID = turnID;
        item.generation = owner->generation;
        item.operationID = owner->operation_id;
        item.language = language;
        item.voiceID = voiceID;
        item.locale = localeString;
        atomic_fetch_sub_explicit(&handle->credits, 1, memory_order_acq_rel);
        item.text = textString;
        item.utteranceID = utterance_id;
        if (object.active == nil) {
            uint32_t startError = saturday_start_item(object, item);
            if (startError != kSpeechErrorNone) {
                saturday_remove_route_listeners(object);
                atomic_fetch_add_explicit(&handle->credits, 1, memory_order_release);
                switch (startError) {
                    case kSpeechErrorVoiceUnavailable:
                        result = SATURDAY_SPEECH_VOICE_UNAVAILABLE;
                        break;
                    case kSpeechErrorWrongLocale:
                        result = SATURDAY_SPEECH_WRONG_LOCALE;
                        break;
                    case kSpeechErrorRouteLost:
                        result = SATURDAY_SPEECH_ROUTE_LOST;
                        break;
                    default:
                        result = SATURDAY_SPEECH_UNAVAILABLE;
                        break;
                }
                return;
            }
        } else {
            [object.pending addObject:item];
        }
    });
    return result;
}

SaturdaySpeechResult saturday_speech_available_credits(
    SaturdaySpeechHandle *handle,
    uint32_t *out_credits) {
    if (handle == NULL || handle->object == NULL || out_credits == NULL) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }
    uint32_t credits = atomic_load_explicit(&handle->credits, memory_order_acquire);
    if (credits > SATURDAY_SPEECH_QUEUE_CAPACITY) {
        return SATURDAY_SPEECH_INTERNAL_ERROR;
    }
    *out_credits = credits;
    return SATURDAY_SPEECH_OK;
}

SaturdaySpeechResult saturday_speech_cancel(
    SaturdaySpeechHandle *handle,
    const uint8_t *session_id,
    size_t session_id_length,
    const uint8_t *turn_id,
    size_t turn_id_length,
    uint64_t generation) {
    if (handle == NULL || handle->object == NULL || generation == 0 ||
        !saturday_owner_is_valid(session_id, session_id_length, turn_id, turn_id_length)) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }
    NSString *sessionID = saturday_string(session_id, session_id_length, SATURDAY_SPEECH_MAX_OWNER_ID_BYTES);
    NSString *turnID = saturday_string(turn_id, turn_id_length, SATURDAY_SPEECH_MAX_OWNER_ID_BYTES);
    if (sessionID == nil || turnID == nil) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }
    SaturdaySpeechStopWaiter *waiter = saturday_new_stop_waiter();
    SaturdaySpeechObject *object = (__bridge SaturdaySpeechObject *)handle->object;
    if ([NSThread isMainThread]) {
        saturday_cancel_owner_on_main(object, sessionID, turnID, generation, waiter);
        return waiter.completed ? waiter.result : SATURDAY_SPEECH_STOP_UNVERIFIED;
    }
    dispatch_async(dispatch_get_main_queue(), ^{
        saturday_cancel_owner_on_main(object, sessionID, turnID, generation, waiter);
    });
    dispatch_time_t deadline = dispatch_time(DISPATCH_TIME_NOW, 250 * NSEC_PER_MSEC);
    if (dispatch_semaphore_wait(waiter.semaphore, deadline) != 0) {
        return SATURDAY_SPEECH_STOP_UNVERIFIED;
    }
    return waiter.result;
}

SaturdaySpeechResult saturday_speech_is_stopped(
    SaturdaySpeechHandle *handle,
    uint32_t *out_is_stopped) {
    if (handle == NULL || handle->object == NULL || out_is_stopped == NULL) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }
    uint32_t stopped = atomic_load_explicit(&handle->stopped, memory_order_acquire);
    if (stopped > 1) {
        return SATURDAY_SPEECH_INTERNAL_ERROR;
    }
    *out_is_stopped = stopped;
    return SATURDAY_SPEECH_OK;
}

SaturdaySpeechResult saturday_speech_destroy(SaturdaySpeechHandle *handle) {
    if (handle == NULL || handle->object == NULL) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }
    SaturdaySpeechObject *object = (__bridge SaturdaySpeechObject *)handle->object;
    SaturdaySpeechStopWaiter *waiter = saturday_new_stop_waiter();
    if ([NSThread isMainThread]) {
        saturday_destroy_on_main(object, waiter);
        if (!waiter.completed || waiter.result != SATURDAY_SPEECH_OK) {
            return waiter.result;
        }
    } else {
        dispatch_async(dispatch_get_main_queue(), ^{
            saturday_destroy_on_main(object, waiter);
        });
        dispatch_time_t deadline = dispatch_time(DISPATCH_TIME_NOW, 250 * NSEC_PER_MSEC);
        if (dispatch_semaphore_wait(waiter.semaphore, deadline) != 0 ||
            waiter.result != SATURDAY_SPEECH_OK) {
            return SATURDAY_SPEECH_STOP_UNVERIFIED;
        }
    }
    CFBridgingRelease(handle->object);
    free(handle);
    return SATURDAY_SPEECH_OK;
}

SaturdaySpeechResult saturday_speech_normalize_nfc(
    const uint8_t *input,
    size_t input_length,
    uint8_t *output,
    size_t output_capacity,
    size_t *out_output_length) {
    if (input == NULL || input_length > 8192 || (output_capacity > 0 && output == NULL) ||
        out_output_length == NULL || memchr(input, 0, input_length) != NULL) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }
    NSString *source = [[NSString alloc] initWithBytes:input length:input_length encoding:NSUTF8StringEncoding];
    if (source == nil) {
        return SATURDAY_SPEECH_INVALID_ARGUMENT;
    }
    NSString *normalized = [source precomposedStringWithCanonicalMapping];
    NSData *data = [normalized dataUsingEncoding:NSUTF8StringEncoding allowLossyConversion:NO];
    if (data == nil) {
        return SATURDAY_SPEECH_INTERNAL_ERROR;
    }
    *out_output_length = data.length;
    if (data.length > output_capacity) {
        return SATURDAY_SPEECH_BUFFER_TOO_SMALL;
    }
    if (data.length > 0) {
        memcpy(output, data.bytes, data.length);
    }
    return SATURDAY_SPEECH_OK;
}

@implementation SaturdaySpeechObject (Delegate)

- (void)speechSynthesizer:(AVSpeechSynthesizer *)synthesizer didStartSpeechUtterance:(AVSpeechUtterance *)utterance {
    if (synthesizer != self.synthesizer || self.active == nil || self.active.utterance != utterance || self.active.started) {
        return;
    }
    self.active.started = YES;
    saturday_emit(self, self.active, SATURDAY_SPEECH_EVENT_DID_START, kSpeechErrorNone);
}

- (void)speechSynthesizer:(AVSpeechSynthesizer *)synthesizer didFinishSpeechUtterance:(AVSpeechUtterance *)utterance {
    if (synthesizer != self.synthesizer || self.active == nil || self.active.utterance != utterance) {
        return;
    }
    SaturdaySpeechItem *finished = self.active;
    self.active = nil;
    saturday_release_credits(self, 1);
    finished.utterance = nil;
    if (finished.started) {
        saturday_emit(self, finished, SATURDAY_SPEECH_EVENT_DID_FINISH, kSpeechErrorNone);
    } else {
        saturday_emit(self, finished, SATURDAY_SPEECH_EVENT_FAILED, kSpeechErrorInternal);
    }
    saturday_start_next(self);
}

- (void)speechSynthesizer:(AVSpeechSynthesizer *)synthesizer didCancelSpeechUtterance:(AVSpeechUtterance *)utterance {
    (void)synthesizer;
    (void)utterance;
    // Cancellation is completed from the immediate-stop verification path, not this optional delegate.
}

@end
