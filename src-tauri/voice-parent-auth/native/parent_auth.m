#import <CoreFoundation/CoreFoundation.h>
#import <Security/Security.h>
#import <libproc.h>
#include <sys/proc_info.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>

typedef const void *saturday_parent_auth_cf_data_ref;

static const int32_t SUCCESS = 0;
static const int32_t FAILURE = -1;
static const int32_t INVALID = -2;
static const int32_t BAD_SIGNATURE = -3;

int32_t saturday_parent_auth_copy_pid_unique_id(int32_t pid, uint64_t *unique_id_out) {
    if (pid <= 0 || unique_id_out == NULL) return INVALID;
    struct proc_bsdinfo info;
    memset(&info, 0, sizeof(info));
    int count = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, sizeof(info));
    if (count != sizeof(info) || info.pbi_pid != (uint32_t)pid
        || info.pbi_start_tvsec == 0 || info.pbi_start_tvusec >= 1000000) return FAILURE;
    /*
     * The SDK's supported BSD process-info flavor exposes the process start
     * timestamp, not the removed PROC_PIDUNIQIDENTIFIERINFO flavor. Encode
     * its full microsecond-resolution value as the incarnation token; callers
     * still compare it against a fresh query for the same live PID.
     */
    *unique_id_out = info.pbi_start_tvsec * UINT64_C(1000000) + info.pbi_start_tvusec;
    return SUCCESS;
}


int32_t saturday_parent_auth_verify_signature(const uint8_t *public_bytes, size_t public_len,
    const uint8_t *message_bytes, size_t message_len, const uint8_t *signature_bytes,
    size_t signature_len) {
    if (public_bytes == NULL || public_len != 65 || public_bytes[0] != 0x04
        || message_bytes == NULL || message_len == 0 || message_len > 65536
        || signature_bytes == NULL || signature_len < 8 || signature_len > 256) return INVALID;
    CFDataRef public_data = CFDataCreate(kCFAllocatorDefault, public_bytes, (CFIndex)public_len);
    CFDataRef message = CFDataCreate(kCFAllocatorDefault, message_bytes, (CFIndex)message_len);
    CFDataRef signature = CFDataCreate(kCFAllocatorDefault, signature_bytes, (CFIndex)signature_len);
    if (public_data == NULL || message == NULL || signature == NULL) {
        if (public_data != NULL) CFRelease(public_data);
        if (message != NULL) CFRelease(message);
        if (signature != NULL) CFRelease(signature);
        return FAILURE;
    }
    int32_t bits = 256;
    CFNumberRef size = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &bits);
    const void *keys[] = { kSecAttrKeyType, kSecAttrKeyClass, kSecAttrKeySizeInBits };
    const void *values[] = { kSecAttrKeyTypeECSECPrimeRandom, kSecAttrKeyClassPublic, size };
    CFDictionaryRef attributes = size == NULL ? NULL : CFDictionaryCreate(kCFAllocatorDefault,
        keys, values, 3, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    if (size != NULL) CFRelease(size);
    SecKeyRef key = attributes == NULL ? NULL : SecKeyCreateWithData(public_data, attributes, NULL);
    if (attributes != NULL) CFRelease(attributes);
    if (key == NULL) {
        CFRelease(public_data); CFRelease(message); CFRelease(signature);
        return FAILURE;
    }
    CFErrorRef error = NULL;
    Boolean valid = SecKeyVerifySignature(key, kSecKeyAlgorithmECDSASignatureMessageX962SHA256,
        message, signature, &error);
    int32_t result = error == NULL && valid ? SUCCESS : BAD_SIGNATURE;
    if (error != NULL) CFRelease(error);
    CFRelease(key); CFRelease(public_data); CFRelease(message); CFRelease(signature);
    return result;
}

static CFDataRef copy_requirement(SecCodeRef code) {
    SecRequirementRef requirement = NULL;
    if (SecCodeCopyDesignatedRequirement(code, kSecCSDefaultFlags, &requirement) != errSecSuccess
        || requirement == NULL) return NULL;
    CFDataRef data = NULL;
    OSStatus status = SecRequirementCopyData(requirement, kSecCSDefaultFlags, &data);
    CFRelease(requirement);
    return status == errSecSuccess ? data : NULL;
}

static CFDataRef copy_signing_string(SecCodeRef code, CFStringRef key) {
    CFDictionaryRef information = NULL;
    if (SecCodeCopySigningInformation(code, kSecCSSigningInformation, &information) != errSecSuccess
        || information == NULL) {
        if (information != NULL) CFRelease(information);
        return NULL;
    }
    CFTypeRef value = CFDictionaryGetValue(information, key);
    CFDataRef data = value != NULL && CFGetTypeID(value) == CFStringGetTypeID()
        ? CFStringCreateExternalRepresentation(kCFAllocatorDefault, (CFStringRef)value, kCFStringEncodingUTF8, 0)
        : NULL;
    CFRelease(information);
    return data;
}

int32_t saturday_parent_auth_copy_live_identity(int32_t pid,
    saturday_parent_auth_cf_data_ref *team_out,
    saturday_parent_auth_cf_data_ref *identifier_out,
    saturday_parent_auth_cf_data_ref *requirement_out) {
    if (pid <= 0 || team_out == NULL || identifier_out == NULL || requirement_out == NULL) return INVALID;
    *team_out = NULL; *identifier_out = NULL; *requirement_out = NULL;
    int32_t target_pid = pid;
    CFNumberRef number = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &target_pid);
    if (number == NULL) return FAILURE;
    const void *keys[] = { kSecGuestAttributePid };
    const void *values[] = { number };
    CFDictionaryRef attributes = CFDictionaryCreate(kCFAllocatorDefault, keys, values, 1,
        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFRelease(number);
    if (attributes == NULL) return FAILURE;
    SecCodeRef code = NULL;
    OSStatus status = SecCodeCopyGuestWithAttributes(NULL, attributes, kSecCSDefaultFlags, &code);
    CFRelease(attributes);
    if (status != errSecSuccess || code == NULL) {
        if (code != NULL) CFRelease(code);
        return FAILURE;
    }
    status = SecCodeCheckValidity(code, kSecCSStrictValidate, NULL);
    CFDataRef team = status == errSecSuccess ? copy_signing_string(code, kSecCodeInfoTeamIdentifier) : NULL;
    CFDataRef identifier = status == errSecSuccess ? copy_signing_string(code, kSecCodeInfoIdentifier) : NULL;
    CFDataRef requirement = status == errSecSuccess ? copy_requirement(code) : NULL;
    CFRelease(code);
    if (team == NULL || identifier == NULL || requirement == NULL) {
        if (team != NULL) CFRelease(team);
        if (identifier != NULL) CFRelease(identifier);
        if (requirement != NULL) CFRelease(requirement);
        return FAILURE;
    }
    *team_out = team; *identifier_out = identifier; *requirement_out = requirement;
    return SUCCESS;
}

int32_t saturday_parent_auth_copy_static_identity(const char *path_utf8,
    saturday_parent_auth_cf_data_ref *team_out,
    saturday_parent_auth_cf_data_ref *identifier_out,
    saturday_parent_auth_cf_data_ref *requirement_out) {
    if (path_utf8 == NULL || team_out == NULL || identifier_out == NULL || requirement_out == NULL) return INVALID;
    *team_out = NULL; *identifier_out = NULL; *requirement_out = NULL;
    size_t length = strnlen(path_utf8, 4097);
    if (length == 0 || length > 4096) return INVALID;
    CFStringRef path = CFStringCreateWithBytes(kCFAllocatorDefault, (const UInt8 *)path_utf8,
        (CFIndex)length, kCFStringEncodingUTF8, false);
    if (path == NULL) return INVALID;
    CFURLRef url = CFURLCreateWithFileSystemPath(kCFAllocatorDefault, path, kCFURLPOSIXPathStyle, false);
    CFRelease(path);
    if (url == NULL) return FAILURE;
    SecStaticCodeRef code = NULL;
    OSStatus status = SecStaticCodeCreateWithPath(url, kSecCSDefaultFlags, &code);
    CFRelease(url);
    if (status != errSecSuccess || code == NULL) {
        if (code != NULL) CFRelease(code);
        return FAILURE;
    }
    status = SecStaticCodeCheckValidity(code, kSecCSStrictValidate, NULL);
    CFDataRef team = status == errSecSuccess ? copy_signing_string((SecCodeRef)code, kSecCodeInfoTeamIdentifier) : NULL;
    CFDataRef identifier = status == errSecSuccess ? copy_signing_string((SecCodeRef)code, kSecCodeInfoIdentifier) : NULL;
    CFDataRef requirement = status == errSecSuccess ? copy_requirement((SecCodeRef)code) : NULL;
    CFRelease(code);
    if (team == NULL || identifier == NULL || requirement == NULL) {
        if (team != NULL) CFRelease(team);
        if (identifier != NULL) CFRelease(identifier);
        if (requirement != NULL) CFRelease(requirement);
        return FAILURE;
    }
    *team_out = team; *identifier_out = identifier; *requirement_out = requirement;
    return SUCCESS;
}

int32_t saturday_parent_auth_copy_pid_path(int32_t pid, char *output, size_t capacity) {
    if (pid <= 0 || output == NULL || capacity == 0 || capacity > 4096) return INVALID;
    int length = proc_pidpath((pid_t)pid, output, (uint32_t)capacity);
    if (length <= 1 || (size_t)length >= capacity) return FAILURE;
    output[length] = '\0';
    return SUCCESS;
}

void saturday_parent_auth_cf_release(saturday_parent_auth_cf_data_ref value) {
    if (value != NULL) CFRelease(value);
}
