#ifndef SATURDAY_VOICE_LIFECYCLE_H
#define SATURDAY_VOICE_LIFECYCLE_H

#ifdef __cplusplus
extern "C" {
#endif

typedef void (*SaturdayVoiceLifecycleCallback)(void);
int saturday_voice_lifecycle_start(SaturdayVoiceLifecycleCallback callback);

#ifdef __cplusplus
}
#endif

#endif
