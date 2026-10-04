#include <CoreFoundation/CoreFoundation.h>
#include <Security/Security.h>
#include <sys/types.h>

// Result codes mirrored by src/platform/code_identity.rs.
enum {
    SATURDAY_IDENTITY_MATCH = 1,
    SATURDAY_IDENTITY_MISMATCH = 0,
    SATURDAY_IDENTITY_CURRENT_TEAM_UNAVAILABLE = -2,
    SATURDAY_IDENTITY_PEER_UNAVAILABLE = -3,
};

static int copy_valid_team(SecCodeRef code, CFStringRef *team_out) {
    if (code == NULL || team_out == NULL) {
        return 0;
    }

    OSStatus status = SecCodeCheckValidity(code, kSecCSStrictValidate, NULL);
    if (status != errSecSuccess) {
        return 0;
    }

    CFDictionaryRef information = NULL;
    status = SecCodeCopySigningInformation(
        code,
        kSecCSSigningInformation,
        &information
    );
    if (status != errSecSuccess || information == NULL) {
        return 0;
    }

    CFTypeRef value = CFDictionaryGetValue(
        information,
        kSecCodeInfoTeamIdentifier
    );
    if (value == NULL || CFGetTypeID(value) != CFStringGetTypeID()) {
        CFRelease(information);
        return 0;
    }

    CFStringRef team = (CFStringRef)value;
    if (CFStringGetLength(team) == 0) {
        CFRelease(information);
        return 0;
    }

    CFRetain(team);
    CFRelease(information);
    *team_out = team;
    return 1;
}

int saturday_process_matches_current_team(pid_t peer_pid) {
    if (peer_pid <= 0) {
        return SATURDAY_IDENTITY_PEER_UNAVAILABLE;
    }

    SecCodeRef current_code = NULL;
    OSStatus status = SecCodeCopySelf(kSecCSDefaultFlags, &current_code);
    if (status != errSecSuccess || current_code == NULL) {
        return SATURDAY_IDENTITY_CURRENT_TEAM_UNAVAILABLE;
    }

    CFStringRef current_team = NULL;
    if (!copy_valid_team(current_code, &current_team)) {
        CFRelease(current_code);
        return SATURDAY_IDENTITY_CURRENT_TEAM_UNAVAILABLE;
    }

    CFNumberRef pid_number = CFNumberCreate(
        kCFAllocatorDefault,
        kCFNumberIntType,
        &peer_pid
    );
    if (pid_number == NULL) {
        CFRelease(current_team);
        CFRelease(current_code);
        return SATURDAY_IDENTITY_PEER_UNAVAILABLE;
    }

    const void *keys[] = {kSecGuestAttributePid};
    const void *values[] = {pid_number};
    CFDictionaryRef attributes = CFDictionaryCreate(
        kCFAllocatorDefault,
        keys,
        values,
        1,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks
    );
    CFRelease(pid_number);
    if (attributes == NULL) {
        CFRelease(current_team);
        CFRelease(current_code);
        return SATURDAY_IDENTITY_PEER_UNAVAILABLE;
    }

    SecCodeRef peer_code = NULL;
    status = SecCodeCopyGuestWithAttributes(
        NULL,
        attributes,
        kSecCSDefaultFlags,
        &peer_code
    );
    CFRelease(attributes);
    if (status != errSecSuccess || peer_code == NULL) {
        CFRelease(current_team);
        CFRelease(current_code);
        return SATURDAY_IDENTITY_PEER_UNAVAILABLE;
    }

    CFStringRef peer_team = NULL;
    if (!copy_valid_team(peer_code, &peer_team)) {
        CFRelease(peer_code);
        CFRelease(current_team);
        CFRelease(current_code);
        return SATURDAY_IDENTITY_PEER_UNAVAILABLE;
    }

    Boolean matches = CFEqual(current_team, peer_team);
    CFRelease(peer_team);
    CFRelease(peer_code);
    CFRelease(current_team);
    CFRelease(current_code);
    return matches ? SATURDAY_IDENTITY_MATCH : SATURDAY_IDENTITY_MISMATCH;
}
