#include <CoreFoundation/CoreFoundation.h>
#include <Security/Security.h>
#include <CommonCrypto/CommonDigest.h>
#include <stdint.h>
#include <string.h>
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
static int hash_cfstring(CFStringRef value, uint8_t digest[CC_SHA256_DIGEST_LENGTH]) {
    if (value == NULL || digest == NULL) {
        return 0;
    }
    CFDataRef bytes = CFStringCreateExternalRepresentation(
        kCFAllocatorDefault, value, kCFStringEncodingUTF8, 0
    );
    if (bytes == NULL) {
        return 0;
    }
    CFIndex length = CFDataGetLength(bytes);
    int valid = length > 0 && length <= UINT32_MAX
        && CC_SHA256(CFDataGetBytePtr(bytes), (CC_LONG)length, digest) != NULL;
    CFRelease(bytes);
    return valid;
}

int saturday_process_matches_identity(
    pid_t peer_pid,
    const char *expected_identifier,
    const uint8_t expected_team_sha256[CC_SHA256_DIGEST_LENGTH],
    const uint8_t expected_requirement_sha256[CC_SHA256_DIGEST_LENGTH]
) {
    if (peer_pid <= 0 || expected_identifier == NULL || expected_identifier[0] == '\0'
        || expected_team_sha256 == NULL || expected_requirement_sha256 == NULL) {
        return SATURDAY_IDENTITY_PEER_UNAVAILABLE;
    }
    CFNumberRef pid_number = CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &peer_pid);
    if (pid_number == NULL) {
        return SATURDAY_IDENTITY_PEER_UNAVAILABLE;
    }
    const void *keys[] = {kSecGuestAttributePid};
    const void *values[] = {pid_number};
    CFDictionaryRef attributes = CFDictionaryCreate(
        kCFAllocatorDefault, keys, values, 1,
        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks
    );
    CFRelease(pid_number);
    if (attributes == NULL) {
        return SATURDAY_IDENTITY_PEER_UNAVAILABLE;
    }
    SecCodeRef peer_code = NULL;
    OSStatus status = SecCodeCopyGuestWithAttributes(
        NULL, attributes, kSecCSDefaultFlags, &peer_code
    );
    CFRelease(attributes);
    if (status != errSecSuccess || peer_code == NULL
        || SecCodeCheckValidity(peer_code, kSecCSStrictValidate, NULL) != errSecSuccess) {
        if (peer_code != NULL) CFRelease(peer_code);
        return SATURDAY_IDENTITY_PEER_UNAVAILABLE;
    }
    CFDictionaryRef information = NULL;
    status = SecCodeCopySigningInformation(peer_code, kSecCSSigningInformation, &information);
    if (status != errSecSuccess || information == NULL) {
        if (information != NULL) CFRelease(information);
        CFRelease(peer_code);
        return SATURDAY_IDENTITY_PEER_UNAVAILABLE;
    }
    CFTypeRef identifier_value = CFDictionaryGetValue(information, kSecCodeInfoIdentifier);
    CFTypeRef team_value = CFDictionaryGetValue(information, kSecCodeInfoTeamIdentifier);
    if (identifier_value == NULL || CFGetTypeID(identifier_value) != CFStringGetTypeID()
        || team_value == NULL || CFGetTypeID(team_value) != CFStringGetTypeID()) {
        CFRelease(information);
        CFRelease(peer_code);
        return SATURDAY_IDENTITY_PEER_UNAVAILABLE;
    }
    CFStringRef expected_identifier_value = CFStringCreateWithCString(
        kCFAllocatorDefault, expected_identifier, kCFStringEncodingUTF8
    );
    uint8_t team_digest[CC_SHA256_DIGEST_LENGTH];
    uint8_t requirement_digest[CC_SHA256_DIGEST_LENGTH];
    SecRequirementRef requirement = NULL;
    CFDataRef requirement_data = NULL;
    int has_team = hash_cfstring((CFStringRef)team_value, team_digest);
    status = SecCodeCopyDesignatedRequirement(peer_code, kSecCSDefaultFlags, &requirement);
    if (status == errSecSuccess && requirement != NULL) {
        status = SecRequirementCopyData(requirement, kSecCSDefaultFlags, &requirement_data);
    }
    int has_requirement = status == errSecSuccess && requirement_data != NULL
        && CFDataGetLength(requirement_data) > 0
        && CFDataGetLength(requirement_data) <= UINT32_MAX
        && CC_SHA256(
            CFDataGetBytePtr(requirement_data),
            (CC_LONG)CFDataGetLength(requirement_data),
            requirement_digest
        ) != NULL;
    Boolean matches = expected_identifier_value != NULL
        && CFEqual(identifier_value, expected_identifier_value)
        && has_team
        && memcmp(team_digest, expected_team_sha256, CC_SHA256_DIGEST_LENGTH) == 0
        && has_requirement
        && memcmp(requirement_digest, expected_requirement_sha256, CC_SHA256_DIGEST_LENGTH) == 0;
    if (requirement_data != NULL) CFRelease(requirement_data);
    if (requirement != NULL) CFRelease(requirement);
    if (expected_identifier_value != NULL) CFRelease(expected_identifier_value);
    CFRelease(information);
    CFRelease(peer_code);
    return matches ? SATURDAY_IDENTITY_MATCH : SATURDAY_IDENTITY_MISMATCH;
}
