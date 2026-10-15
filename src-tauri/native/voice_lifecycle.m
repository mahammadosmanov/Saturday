#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
#include "voice_lifecycle.h"

static NSArray<id> *saturday_voice_lifecycle_observers;

int saturday_voice_lifecycle_start(SaturdayVoiceLifecycleCallback callback) {
    if (callback == NULL) {
        return 1;
    }
    static dispatch_once_t once_token;
    __block BOOL installed = NO;
    dispatch_once(&once_token, ^{
        NSNotificationCenter *workspace_center = NSWorkspace.sharedWorkspace.notificationCenter;
        NSMutableArray<id> *observers = [NSMutableArray array];
        NSArray<NSNotificationName> *workspace_names = @[
            NSWorkspaceWillSleepNotification,
            NSWorkspaceScreensDidSleepNotification,
            NSWorkspaceSessionDidResignActiveNotification,
        ];
        for (NSNotificationName name in workspace_names) {
            id observer = [workspace_center addObserverForName:name
                                                        object:nil
                                                         queue:NSOperationQueue.mainQueue
                                                    usingBlock:^(__unused NSNotification *notification) {
                callback();
            }];
            if (observer == nil) {
                return;
            }
            [observers addObject:observer];
        }
        NSNotificationCenter *application_center = NSNotificationCenter.defaultCenter;
        NSArray<NSNotificationName> *application_names = @[
            NSApplicationDidResignActiveNotification,
            NSApplicationDidHideNotification,
            NSWindowDidMiniaturizeNotification,
        ];
        for (NSNotificationName name in application_names) {
            id observer = [application_center addObserverForName:name
                                                          object:nil
                                                           queue:NSOperationQueue.mainQueue
                                                      usingBlock:^(__unused NSNotification *notification) {
                callback();
            }];
            if (observer == nil) {
                return;
            }
            [observers addObject:observer];
        }
        id lock_observer = [NSDistributedNotificationCenter.defaultCenter
            addObserverForName:@"com.apple.screenIsLocked"
                        object:nil
                         queue:NSOperationQueue.mainQueue
                    usingBlock:^(__unused NSNotification *notification) {
            callback();
        }];
        if (lock_observer == nil) {
            return;
        }
        [observers addObject:lock_observer];
        saturday_voice_lifecycle_observers = [observers copy];
        installed = YES;
    });
    return installed ? 0 : 2;
}
