#import <dispatch/dispatch.h>
#import <AppKit/AppKit.h>
#import <CoreFoundation/CoreFoundation.h>
#import <Foundation/Foundation.h>
#import <LocalAuthentication/LocalAuthentication.h>
#import <Security/Security.h>
#include <limits.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>

/* This application tag is intentionally unrelated to the Stage 1 owner key. */
static const uint8_t SATURDAY_VOICE_KEY_TAG[] = "dev.rohtolos.saturday.voice.owner-key.v1";
enum {
    SATURDAY_VOICE_STATE_MAX_BYTES = 8192,
    SATURDAY_VOICE_STATE_SERIAL_BYTES = 8,
};
typedef const void *saturday_voice_cf_data_ref;

enum {
    SATURDAY_VOICE_SUCCESS = 0,
    SATURDAY_VOICE_UNAVAILABLE = -1,
    SATURDAY_VOICE_KEY_NOT_FOUND = -2,
    SATURDAY_VOICE_CANCELLED = -3,
    SATURDAY_VOICE_AUTHENTICATION_FAILED = -4,
    SATURDAY_VOICE_INVALID_INPUT = -5,
    SATURDAY_VOICE_SYSTEM = -6,
    SATURDAY_VOICE_BAD_SIGNATURE = -7,
};

static int32_t map_status(OSStatus status) {
    switch (status) {
        case errSecSuccess: return SATURDAY_VOICE_SUCCESS;
        case errSecItemNotFound: return SATURDAY_VOICE_KEY_NOT_FOUND;
        case errSecUserCanceled: return SATURDAY_VOICE_CANCELLED;
        case errSecAuthFailed:
        case errSecInteractionNotAllowed: return SATURDAY_VOICE_AUTHENTICATION_FAILED;
        case errSecNotAvailable:
        case errSecUnimplemented:
        case errSecParam: return SATURDAY_VOICE_UNAVAILABLE;
        case errSecVerifyFailed: return SATURDAY_VOICE_BAD_SIGNATURE;
        default: return SATURDAY_VOICE_SYSTEM;
    }
}

static int32_t map_error(CFErrorRef error) {
    return error == NULL ? SATURDAY_VOICE_SYSTEM : map_status((OSStatus)CFErrorGetCode(error));
}

static CFDataRef make_data(const uint8_t *bytes, size_t length) {
    if (bytes == NULL || length == 0 || length > (size_t)LONG_MAX) return NULL;
    return CFDataCreate(kCFAllocatorDefault, bytes, (CFIndex)length);
}

static Boolean current_code_is_valid(void) {
    SecCodeRef self = NULL;
    OSStatus status = SecCodeCopySelf(kSecCSDefaultFlags, &self);
    if (status != errSecSuccess || self == NULL) {
        if (self != NULL) CFRelease(self);
        return false;
    }
    status = SecCodeCheckValidity(self, kSecCSStrictValidate, NULL);
    CFRelease(self);
    return status == errSecSuccess;
}

static uint64_t authorization_state_serial(const uint8_t *bytes) {
    uint64_t serial = 0;
    for (size_t index = 0; index < SATURDAY_VOICE_STATE_SERIAL_BYTES; ++index) {
        serial = (serial << 8) | bytes[index];
    }
    return serial;
}

static CFMutableDictionaryRef authorization_state_query(
    CFStringRef service, CFStringRef account, Boolean return_data
) {
    if (service == NULL || account == NULL) return NULL;
    CFMutableDictionaryRef query = CFDictionaryCreateMutable(kCFAllocatorDefault, 5,
        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    if (query == NULL) return NULL;
    CFDictionarySetValue(query, kSecClass, kSecClassGenericPassword);
    CFDictionarySetValue(query, kSecAttrService, service);
    CFDictionarySetValue(query, kSecAttrAccount, account);
    if (return_data) CFDictionarySetValue(query, kSecReturnData, kCFBooleanTrue);
    CFDictionarySetValue(query, kSecMatchLimit, kSecMatchLimitOne);
    return query;
}

static int32_t authorization_state_copy_for(
    CFStringRef service, CFStringRef account, uint8_t *output, size_t capacity,
    size_t *length_out, uint64_t *serial_out
) {
    if (output == NULL || length_out == NULL || serial_out == NULL
        || capacity < SATURDAY_VOICE_STATE_MAX_BYTES) return SATURDAY_VOICE_INVALID_INPUT;
    *length_out = 0;
    *serial_out = 0;
    if (!current_code_is_valid()) return SATURDAY_VOICE_UNAVAILABLE;
    CFMutableDictionaryRef query = authorization_state_query(service, account, true);
    if (query == NULL) return SATURDAY_VOICE_SYSTEM;
    CFTypeRef result = NULL;
    OSStatus status = SecItemCopyMatching(query, &result);
    CFRelease(query);
    if (status != errSecSuccess) {
        if (result != NULL) CFRelease(result);
        return map_status(status);
    }
    if (result == NULL || CFGetTypeID(result) != CFDataGetTypeID()) {
        if (result != NULL) CFRelease(result);
        return SATURDAY_VOICE_SYSTEM;
    }
    CFDataRef data = (CFDataRef)result;
    CFIndex data_length = CFDataGetLength(data);
    const uint8_t *stored = CFDataGetBytePtr(data);
    Boolean valid = stored != NULL
        && data_length > (CFIndex)SATURDAY_VOICE_STATE_SERIAL_BYTES
        && data_length <= (CFIndex)(SATURDAY_VOICE_STATE_SERIAL_BYTES + SATURDAY_VOICE_STATE_MAX_BYTES);
    uint64_t serial = valid ? authorization_state_serial(stored) : 0;
    size_t json_length = valid ? (size_t)data_length - SATURDAY_VOICE_STATE_SERIAL_BYTES : 0;
    valid = valid && serial > 0 && json_length <= capacity;
    if (valid) {
        memcpy(output, stored + SATURDAY_VOICE_STATE_SERIAL_BYTES, json_length);
        *length_out = json_length;
        *serial_out = serial;
    }
    CFRelease(data);
    return valid ? SATURDAY_VOICE_SUCCESS : SATURDAY_VOICE_INVALID_INPUT;
}

int32_t saturday_voice_authorization_state_copy(uint8_t *output, size_t capacity,
    size_t *length_out, uint64_t *serial_out) {
    return authorization_state_copy_for(
        CFSTR("dev.rohtolos.saturday.voice"),
        CFSTR("authorization-state.v1"),
        output, capacity, length_out, serial_out
    );
}

static int32_t authorization_state_store_for(
    CFStringRef service, CFStringRef account, uint64_t serial,
    const uint8_t *json_bytes, size_t json_length
) {
    if (serial == 0 || json_bytes == NULL || json_length == 0
        || json_length > SATURDAY_VOICE_STATE_MAX_BYTES) return SATURDAY_VOICE_INVALID_INPUT;
    uint8_t encoded[SATURDAY_VOICE_STATE_SERIAL_BYTES + SATURDAY_VOICE_STATE_MAX_BYTES];
    uint64_t remaining = serial;
    for (size_t index = 0; index < SATURDAY_VOICE_STATE_SERIAL_BYTES; ++index) {
        encoded[SATURDAY_VOICE_STATE_SERIAL_BYTES - index - 1] = (uint8_t)(remaining & 0xff);
        remaining >>= 8;
    }
    memcpy(encoded + SATURDAY_VOICE_STATE_SERIAL_BYTES, json_bytes, json_length);
    CFDataRef value = CFDataCreate(kCFAllocatorDefault, encoded,
        (CFIndex)(SATURDAY_VOICE_STATE_SERIAL_BYTES + json_length));
    if (value == NULL) return SATURDAY_VOICE_SYSTEM;
    if (!current_code_is_valid()) {
        CFRelease(value);
        return SATURDAY_VOICE_UNAVAILABLE;
    }
    CFMutableDictionaryRef query = authorization_state_query(service, account, true);
    if (query == NULL) { CFRelease(value); return SATURDAY_VOICE_SYSTEM; }
    CFTypeRef existing = NULL;
    OSStatus status = SecItemCopyMatching(query, &existing);
    if (status == errSecItemNotFound) {
        CFDictionaryRemoveValue(query, kSecReturnData);
        CFDictionaryRemoveValue(query, kSecMatchLimit);
        CFDictionarySetValue(query, kSecValueData, value);
        CFDictionarySetValue(query, kSecAttrAccessible, kSecAttrAccessibleWhenUnlockedThisDeviceOnly);
        CFDictionarySetValue(query, kSecAttrSynchronizable, kCFBooleanFalse);
        status = SecItemAdd(query, NULL);
    } else if (status == errSecSuccess && existing != NULL
        && CFGetTypeID(existing) == CFDataGetTypeID()) {
        CFDataRef old_data = (CFDataRef)existing;
        CFIndex old_length = CFDataGetLength(old_data);
        const uint8_t *old_bytes = CFDataGetBytePtr(old_data);
        if (old_bytes == NULL || old_length <= (CFIndex)SATURDAY_VOICE_STATE_SERIAL_BYTES
            || old_length > (CFIndex)(SATURDAY_VOICE_STATE_SERIAL_BYTES + SATURDAY_VOICE_STATE_MAX_BYTES)
            || authorization_state_serial(old_bytes) == 0
            || serial <= authorization_state_serial(old_bytes)) {
            status = errSecParam;
        } else {
            CFDictionaryRemoveValue(query, kSecReturnData);
            CFDictionaryRemoveValue(query, kSecMatchLimit);
            const void *keys[] = { kSecValueData };
            const void *values[] = { value };
            CFDictionaryRef update = CFDictionaryCreate(kCFAllocatorDefault, keys, values, 1,
                &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
            status = update == NULL ? errSecAllocate : SecItemUpdate(query, update);
            if (update != NULL) CFRelease(update);
        }
    }
    if (existing != NULL) CFRelease(existing);
    CFRelease(query);
    CFRelease(value);
    return map_status(status);
}

int32_t saturday_voice_authorization_state_store(uint64_t serial,
    const uint8_t *json_bytes, size_t json_length) {
    return authorization_state_store_for(
        CFSTR("dev.rohtolos.saturday.voice"),
        CFSTR("authorization-state.v1"),
        serial, json_bytes, json_length
    );
}

int32_t saturday_voice_keychain_app_scope_self_test(void) {
    if (!current_code_is_valid()) return SATURDAY_VOICE_UNAVAILABLE;
    CFUUIDRef uuid = CFUUIDCreate(kCFAllocatorDefault);
    if (uuid == NULL) return SATURDAY_VOICE_SYSTEM;
    CFStringRef account = CFUUIDCreateString(kCFAllocatorDefault, uuid);
    CFRelease(uuid);
    if (account == NULL) return SATURDAY_VOICE_SYSTEM;

    static const uint8_t expected[] = "{\"schema\":\"saturday.keychain-scope-self-test.v1\"}";
    uint8_t actual[SATURDAY_VOICE_STATE_MAX_BYTES];
    size_t actual_length = 0;
    uint64_t actual_serial = 0;
    CFStringRef service = CFSTR("dev.rohtolos.saturday.voice.scope-self-test.v1");
    int32_t result = authorization_state_store_for(
        service, account, 1, expected, sizeof(expected) - 1
    );
    if (result == SATURDAY_VOICE_SUCCESS) {
        result = authorization_state_copy_for(
            service, account, actual, sizeof(actual), &actual_length, &actual_serial
        );
        if (result == SATURDAY_VOICE_SUCCESS
            && (actual_serial != 1 || actual_length != sizeof(expected) - 1
                || memcmp(actual, expected, sizeof(expected) - 1) != 0)) {
            result = SATURDAY_VOICE_SYSTEM;
        }
    }

    CFMutableDictionaryRef query = authorization_state_query(service, account, false);
    if (query == NULL) {
        result = SATURDAY_VOICE_SYSTEM;
    } else {
        OSStatus delete_status = SecItemDelete(query);
        if (delete_status != errSecSuccess && delete_status != errSecItemNotFound) {
            result = SATURDAY_VOICE_SYSTEM;
        }
        CFRelease(query);
    }
    CFRelease(account);
    return result;
}


static NSString *string_from_utf8(const char *value, size_t maximum) {
    if (value == NULL) return nil;
    size_t length = strnlen(value, maximum + 1);
    if (length == 0 || length > maximum) return nil;
    return [[NSString alloc] initWithBytes:value length:length encoding:NSUTF8StringEncoding];
}

static LAContext *new_context(const char *prompt_utf8) {
    NSString *prompt = string_from_utf8(prompt_utf8, 256);
    if (prompt == nil) return nil;
    LAContext *context = [[LAContext alloc] init];
    context.localizedReason = prompt;
    context.localizedFallbackTitle = @"";
    context.localizedCancelTitle = @"Cancel";
    return context;
}

static CFDictionaryRef private_key_query(CFDataRef tag, LAContext *context, Boolean return_reference) {
    if (tag == NULL || CFDataGetLength(tag) != (CFIndex)(sizeof(SATURDAY_VOICE_KEY_TAG) - 1)
        || memcmp(CFDataGetBytePtr(tag), SATURDAY_VOICE_KEY_TAG, sizeof(SATURDAY_VOICE_KEY_TAG) - 1) != 0
        || !current_code_is_valid()) {
        return NULL;
    }
    const void *keys[8];
    const void *values[8];
    CFIndex count = 0;
    keys[count] = kSecClass; values[count++] = kSecClassKey;
    keys[count] = kSecAttrKeyClass; values[count++] = kSecAttrKeyClassPrivate;
    keys[count] = kSecAttrKeyType; values[count++] = kSecAttrKeyTypeECSECPrimeRandom;
    keys[count] = kSecAttrTokenID; values[count++] = kSecAttrTokenIDSecureEnclave;
    keys[count] = kSecAttrApplicationTag; values[count++] = tag;
    if (context != nil) {
        keys[count] = kSecUseAuthenticationContext; values[count++] = (__bridge const void *)context;
    }
    if (return_reference) {
        keys[count] = kSecReturnRef; values[count++] = kCFBooleanTrue;
        keys[count] = kSecMatchLimit; values[count++] = kSecMatchLimitOne;
    }
    /* No explicit access group confines this query to the signed app's default group. */
    return CFDictionaryCreate(kCFAllocatorDefault, keys, values, count,
        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
}

static int validate_private_key(SecKeyRef key, CFDataRef tag) {
    if (key == NULL || tag == NULL) return 0;
    CFDictionaryRef attributes = SecKeyCopyAttributes(key);
    if (attributes == NULL) return 0;
    CFTypeRef token = CFDictionaryGetValue(attributes, kSecAttrTokenID);
    CFTypeRef key_class = CFDictionaryGetValue(attributes, kSecAttrKeyClass);
    CFTypeRef key_type = CFDictionaryGetValue(attributes, kSecAttrKeyType);
    CFTypeRef key_size = CFDictionaryGetValue(attributes, kSecAttrKeySizeInBits);
    CFTypeRef app_tag = CFDictionaryGetValue(attributes, kSecAttrApplicationTag);
    int valid = token != NULL && CFEqual(token, kSecAttrTokenIDSecureEnclave)
        && key_class != NULL && CFEqual(key_class, kSecAttrKeyClassPrivate)
        && key_type != NULL && CFEqual(key_type, kSecAttrKeyTypeECSECPrimeRandom)
        && key_size != NULL && CFGetTypeID(key_size) == CFNumberGetTypeID()
        && app_tag != NULL && CFEqual(app_tag, tag);
    int32_t bits = 0;
    if (valid) valid = CFNumberGetValue((CFNumberRef)key_size, kCFNumberSInt32Type, &bits) && bits == 256;
    CFRelease(attributes);
    return valid;
}


static int32_t copy_private_key(CFDataRef tag, LAContext *context, SecKeyRef *key_out) {
    if (key_out == NULL) return SATURDAY_VOICE_INVALID_INPUT;
    *key_out = NULL;
    CFDictionaryRef query = private_key_query(tag, context, true);
    if (query == NULL) return SATURDAY_VOICE_INVALID_INPUT;
    CFTypeRef result = NULL;
    OSStatus status = SecItemCopyMatching(query, &result);
    CFRelease(query);
    if (status != errSecSuccess) {
        if (result != NULL) CFRelease(result);
        return map_status(status);
    }
    if (result == NULL || CFGetTypeID(result) != SecKeyGetTypeID()
        || !validate_private_key((SecKeyRef)result, tag)) {
        if (result != NULL) CFRelease(result);
        return SATURDAY_VOICE_UNAVAILABLE;
    }
    *key_out = (SecKeyRef)result;
    return SATURDAY_VOICE_SUCCESS;
}

static int32_t create_private_key(CFDataRef tag, SecKeyRef *key_out) {
    if (key_out == NULL) return SATURDAY_VOICE_INVALID_INPUT;
    *key_out = NULL;
    if (!current_code_is_valid()) return SATURDAY_VOICE_UNAVAILABLE;
    CFErrorRef error = NULL;
    SecAccessControlRef access = SecAccessControlCreateWithFlags(kCFAllocatorDefault,
        kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        kSecAccessControlPrivateKeyUsage | kSecAccessControlBiometryCurrentSet,
        &error);
    if (access == NULL) {
        int32_t result = map_error(error);
        if (error != NULL) CFRelease(error);
        return result;
    }
    if (error != NULL) { CFRelease(error); error = NULL; }

    const void *private_keys[] = {
        kSecAttrIsPermanent, kSecAttrApplicationTag, kSecAttrAccessControl
    };
    const void *private_values[] = { kCFBooleanTrue, tag, access };
    CFDictionaryRef private_attributes = CFDictionaryCreate(kCFAllocatorDefault, private_keys,
        private_values, 3, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    if (private_attributes == NULL) { CFRelease(access); return SATURDAY_VOICE_SYSTEM; }
    int32_t bits = 256;
    CFNumberRef size = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &bits);
    if (size == NULL) { CFRelease(private_attributes); CFRelease(access); return SATURDAY_VOICE_SYSTEM; }
    const void *keys[] = { kSecAttrKeyType, kSecAttrKeySizeInBits, kSecAttrTokenID, kSecPrivateKeyAttrs };
    const void *values[] = { kSecAttrKeyTypeECSECPrimeRandom, size, kSecAttrTokenIDSecureEnclave, private_attributes };
    CFDictionaryRef attributes = CFDictionaryCreate(kCFAllocatorDefault, keys, values, 4,
        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFRelease(size);
    CFRelease(private_attributes);
    CFRelease(access);
    if (attributes == NULL) return SATURDAY_VOICE_SYSTEM;
    SecKeyRef key = SecKeyCreateRandomKey(attributes, &error);
    CFRelease(attributes);
    if (key == NULL) {
        int32_t result = map_error(error);
        if (error != NULL) CFRelease(error);
        return result;
    }
    if (error != NULL) CFRelease(error);
    if (!validate_private_key(key, tag)) { CFRelease(key); return SATURDAY_VOICE_UNAVAILABLE; }
    *key_out = key;
    return SATURDAY_VOICE_SUCCESS;
}

static int32_t copy_public_key_data(SecKeyRef private_key, CFDataRef *data_out) {
    if (data_out == NULL) return SATURDAY_VOICE_INVALID_INPUT;
    *data_out = NULL;
    SecKeyRef public_key = SecKeyCopyPublicKey(private_key);
    if (public_key == NULL) return SATURDAY_VOICE_SYSTEM;
    CFErrorRef error = NULL;
    CFDataRef data = SecKeyCopyExternalRepresentation(public_key, &error);
    CFRelease(public_key);
    if (data == NULL) {
        int32_t result = map_error(error);
        if (error != NULL) CFRelease(error);
        return result;
    }
    if (error != NULL) CFRelease(error);
    if (CFDataGetLength(data) != 65 || CFDataGetBytePtr(data)[0] != 0x04) {
        CFRelease(data);
        return SATURDAY_VOICE_UNAVAILABLE;
    }
    *data_out = data;
    return SATURDAY_VOICE_SUCCESS;
}

static int32_t sign_data(SecKeyRef private_key, CFDataRef message, CFDataRef *signature_out) {
    if (signature_out == NULL) return SATURDAY_VOICE_INVALID_INPUT;
    *signature_out = NULL;
    CFErrorRef error = NULL;
    CFDataRef signature = SecKeyCreateSignature(private_key,
        kSecKeyAlgorithmECDSASignatureMessageX962SHA256, message, &error);
    if (signature == NULL) {
        int32_t result = map_error(error);
        if (error != NULL) CFRelease(error);
        return result;
    }
    if (error != NULL) CFRelease(error);
    if (CFDataGetLength(signature) < 8 || CFDataGetLength(signature) > 256
        || CFDataGetBytePtr(signature)[0] != 0x30) {
        CFRelease(signature);
        return SATURDAY_VOICE_BAD_SIGNATURE;
    }
    *signature_out = signature;
    return SATURDAY_VOICE_SUCCESS;
}

int32_t saturday_voice_key_authorize_session(const uint8_t *tag_bytes, size_t tag_len,
    const char *prompt_utf8, void **context_out) {
    if (context_out == NULL || tag_bytes == NULL || tag_len != sizeof(SATURDAY_VOICE_KEY_TAG) - 1
        || memcmp(tag_bytes, SATURDAY_VOICE_KEY_TAG, tag_len) != 0) return SATURDAY_VOICE_INVALID_INPUT;
    *context_out = NULL;
    NSString *prompt = string_from_utf8(prompt_utf8, 256);
    LAContext *context = new_context(prompt_utf8);
    if (prompt == nil || context == nil) return SATURDAY_VOICE_INVALID_INPUT;
    CFErrorRef error = NULL;
    SecAccessControlRef access = SecAccessControlCreateWithFlags(kCFAllocatorDefault,
        kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        kSecAccessControlPrivateKeyUsage | kSecAccessControlBiometryCurrentSet,
        &error);
    if (access == NULL) {
        int32_t result = map_error(error);
        if (error != NULL) CFRelease(error);
        return result;
    }
    if (error != NULL) { CFRelease(error); error = NULL; }
    dispatch_semaphore_t completed = dispatch_semaphore_create(0);
    __block BOOL allowed = NO;
    __block NSError *evaluation_error = nil;
    [context evaluateAccessControl:access
        operation:LAAccessControlOperationUseKeySign
        localizedReason:prompt
        reply:^(BOOL success, NSError *failure) {
            allowed = success;
            evaluation_error = failure;
            dispatch_semaphore_signal(completed);
        }];
    CFRelease(access);
    if (dispatch_semaphore_wait(completed, dispatch_time(DISPATCH_TIME_NOW, 120 * NSEC_PER_SEC)) != 0) {
        [context invalidate];
        return SATURDAY_VOICE_AUTHENTICATION_FAILED;
    }
    if (!allowed || evaluation_error != nil) {
        [context invalidate];
        return evaluation_error != nil && [evaluation_error.domain isEqualToString:LAErrorDomain]
            && evaluation_error.code == LAErrorUserCancel
            ? SATURDAY_VOICE_CANCELLED : SATURDAY_VOICE_AUTHENTICATION_FAILED;
    }
    context.interactionNotAllowed = YES;
    *context_out = (__bridge_retained void *)context;
    return SATURDAY_VOICE_SUCCESS;
}

void saturday_voice_key_context_invalidate_release(void *context_value) {
    if (context_value == NULL) return;
    LAContext *context = CFBridgingRelease(context_value);
    [context invalidate];
}

int32_t saturday_voice_key_sign_authorized(const uint8_t *tag_bytes, size_t tag_len,
    const uint8_t *message_bytes, size_t message_len, void *context_value,
    saturday_voice_cf_data_ref *public_out, saturday_voice_cf_data_ref *signature_out) {
    if (public_out == NULL || signature_out == NULL || context_value == NULL
        || tag_bytes == NULL || message_bytes == NULL || message_len == 0
        || message_len > 65536) return SATURDAY_VOICE_INVALID_INPUT;
    *public_out = NULL; *signature_out = NULL;
    CFDataRef tag = make_data(tag_bytes, tag_len);
    CFDataRef message = make_data(message_bytes, message_len);
    LAContext *context = (__bridge LAContext *)context_value;
    if (tag == NULL || message == NULL || context.interactionNotAllowed != YES) {
        if (tag != NULL) CFRelease(tag);
        if (message != NULL) CFRelease(message);
        return SATURDAY_VOICE_AUTHENTICATION_FAILED;
    }
    SecKeyRef key = NULL;
    int32_t result = copy_private_key(tag, context, &key);
    if (result == SATURDAY_VOICE_SUCCESS) result = copy_public_key_data(key, (CFDataRef *)public_out);
    if (result == SATURDAY_VOICE_SUCCESS) result = sign_data(key, message, (CFDataRef *)signature_out);
    if (key != NULL) CFRelease(key);
    CFRelease(message);
    CFRelease(tag);
    if (result != SATURDAY_VOICE_SUCCESS) {
        if (*public_out != NULL) CFRelease(*public_out);
        if (*signature_out != NULL) CFRelease(*signature_out);
        *public_out = NULL; *signature_out = NULL;
    }
    return result;
}

int32_t saturday_voice_key_provision(const uint8_t *tag_bytes, size_t tag_len,
    const uint8_t *challenge, size_t challenge_len, const char *prompt,
    saturday_voice_cf_data_ref *public_out, saturday_voice_cf_data_ref *signature_out) {
    if (public_out == NULL || signature_out == NULL) return SATURDAY_VOICE_INVALID_INPUT;
    *public_out = NULL; *signature_out = NULL;
    if (tag_bytes == NULL || tag_len == 0 || tag_len > 128 || challenge == NULL
        || challenge_len == 0 || challenge_len > 4096) return SATURDAY_VOICE_INVALID_INPUT;
    CFDataRef tag = make_data(tag_bytes, tag_len);
    CFDataRef message = make_data(challenge, challenge_len);
    LAContext *context = new_context(prompt);
    if (tag == NULL || message == NULL || context == nil) {
        if (tag != NULL) CFRelease(tag);
        if (message != NULL) CFRelease(message);
        return SATURDAY_VOICE_INVALID_INPUT;
    }
    SecKeyRef key = NULL;
    int32_t result = copy_private_key(tag, context, &key);
    if (result == SATURDAY_VOICE_KEY_NOT_FOUND) result = create_private_key(tag, &key);
    if (result == SATURDAY_VOICE_SUCCESS) result = copy_public_key_data(key, (CFDataRef *)public_out);
    if (result == SATURDAY_VOICE_SUCCESS) result = sign_data(key, message, (CFDataRef *)signature_out);
    if (key != NULL) CFRelease(key);
    if (message != NULL) CFRelease(message);
    if (tag != NULL) CFRelease(tag);
    if (result != SATURDAY_VOICE_SUCCESS) {
        if (*public_out != NULL) CFRelease(*public_out);
        if (*signature_out != NULL) CFRelease(*signature_out);
        *public_out = NULL; *signature_out = NULL;
    }
    return result;
}

int32_t saturday_voice_key_copy_public(const uint8_t *tag_bytes, size_t tag_len,
    saturday_voice_cf_data_ref *public_out) {
    if (public_out == NULL) return SATURDAY_VOICE_INVALID_INPUT;
    *public_out = NULL;
    if (tag_bytes == NULL || tag_len == 0 || tag_len > 128) return SATURDAY_VOICE_INVALID_INPUT;
    CFDataRef tag = make_data(tag_bytes, tag_len);
    if (tag == NULL) return SATURDAY_VOICE_INVALID_INPUT;
    SecKeyRef key = NULL;
    int32_t result = copy_private_key(tag, nil, &key);
    if (result == SATURDAY_VOICE_SUCCESS) result = copy_public_key_data(key, (CFDataRef *)public_out);
    if (key != NULL) CFRelease(key);
    CFRelease(tag);
    return result;
}

int32_t saturday_voice_key_sign(const uint8_t *tag_bytes, size_t tag_len,
    const uint8_t *message_bytes, size_t message_len, const char *prompt,
    saturday_voice_cf_data_ref *public_out, saturday_voice_cf_data_ref *signature_out) {
    if (public_out == NULL || signature_out == NULL) return SATURDAY_VOICE_INVALID_INPUT;
    *public_out = NULL; *signature_out = NULL;
    if (tag_bytes == NULL || tag_len == 0 || tag_len > 128 || message_bytes == NULL
        || message_len == 0 || message_len > 65536) return SATURDAY_VOICE_INVALID_INPUT;
    CFDataRef tag = make_data(tag_bytes, tag_len);
    CFDataRef message = make_data(message_bytes, message_len);
    LAContext *context = new_context(prompt);
    if (tag == NULL || message == NULL || context == nil) {
        if (tag != NULL) CFRelease(tag);
        if (message != NULL) CFRelease(message);
        return SATURDAY_VOICE_INVALID_INPUT;
    }
    SecKeyRef key = NULL;
    int32_t result = copy_private_key(tag, context, &key);
    if (result == SATURDAY_VOICE_SUCCESS) result = copy_public_key_data(key, (CFDataRef *)public_out);
    if (result == SATURDAY_VOICE_SUCCESS) result = sign_data(key, message, (CFDataRef *)signature_out);
    if (key != NULL) CFRelease(key);
    CFRelease(message);
    CFRelease(tag);
    if (result != SATURDAY_VOICE_SUCCESS) {
        if (*public_out != NULL) CFRelease(*public_out);
        if (*signature_out != NULL) CFRelease(*signature_out);
        *public_out = NULL; *signature_out = NULL;
    }
    return result;
}

int32_t saturday_voice_key_verify(const uint8_t *public_bytes, size_t public_len,
    const uint8_t *message_bytes, size_t message_len, const uint8_t *signature_bytes,
    size_t signature_len, int32_t *valid_out) {
    if (valid_out == NULL) return SATURDAY_VOICE_INVALID_INPUT;
    *valid_out = 0;
    if (public_bytes == NULL || public_len != 65 || public_bytes[0] != 0x04
        || message_bytes == NULL || message_len == 0 || message_len > 65536
        || signature_bytes == NULL || signature_len < 8 || signature_len > 256) return SATURDAY_VOICE_INVALID_INPUT;
    CFDataRef public_data = make_data(public_bytes, public_len);
    CFDataRef message = make_data(message_bytes, message_len);
    CFDataRef signature = make_data(signature_bytes, signature_len);
    if (public_data == NULL || message == NULL || signature == NULL) {
        if (public_data != NULL) CFRelease(public_data);
        if (message != NULL) CFRelease(message);
        if (signature != NULL) CFRelease(signature);
        return SATURDAY_VOICE_INVALID_INPUT;
    }
    int32_t bits = 256;
    CFNumberRef key_size = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &bits);
    const void *keys[] = { kSecAttrKeyType, kSecAttrKeyClass, kSecAttrKeySizeInBits };
    const void *values[] = { kSecAttrKeyTypeECSECPrimeRandom, kSecAttrKeyClassPublic, key_size };
    CFDictionaryRef attributes = key_size == NULL ? NULL : CFDictionaryCreate(kCFAllocatorDefault,
        keys, values, 3, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    if (key_size != NULL) CFRelease(key_size);
    if (attributes == NULL) {
        CFRelease(public_data); CFRelease(message); CFRelease(signature);
        return SATURDAY_VOICE_SYSTEM;
    }
    CFErrorRef error = NULL;
    SecKeyRef public_key = SecKeyCreateWithData(public_data, attributes, &error);
    CFRelease(attributes);
    if (public_key == NULL) {
        int32_t result = map_error(error);
        if (error != NULL) CFRelease(error);
        CFRelease(public_data); CFRelease(message); CFRelease(signature);
        return result;
    }
    if (error != NULL) CFRelease(error);
    error = NULL;
    Boolean valid = SecKeyVerifySignature(public_key,
        kSecKeyAlgorithmECDSASignatureMessageX962SHA256, message, signature, &error);
    if (error != NULL) {
        int32_t result = map_error(error);
        CFRelease(error); CFRelease(public_key); CFRelease(public_data); CFRelease(message); CFRelease(signature);
        if (result == SATURDAY_VOICE_BAD_SIGNATURE) { *valid_out = 0; return SATURDAY_VOICE_SUCCESS; }
        return result;
    }
    *valid_out = valid ? 1 : 0;
    CFRelease(public_key); CFRelease(public_data); CFRelease(message); CFRelease(signature);
    return SATURDAY_VOICE_SUCCESS;
}

int32_t saturday_voice_present_consent(
    int32_t purpose,
    const uint8_t *body_en_bytes, size_t body_en_len,
    const uint8_t *body_tr_bytes, size_t body_tr_len
) {
    if (![NSThread isMainThread] || purpose < 0 || purpose > 3
        || body_tr_bytes == NULL || body_tr_len == 0 || body_tr_len > 16384) {
        return SATURDAY_VOICE_INVALID_INPUT;
    }
    NSWindow *window = NSApp.keyWindow;
    if (NSApp == nil || window == nil || !window.isVisible || !window.isKeyWindow) {
        return SATURDAY_VOICE_UNAVAILABLE;
    }
    NSString *body_en = [[NSString alloc] initWithBytes:body_en_bytes
        length:body_en_len encoding:NSUTF8StringEncoding];
    NSString *body_tr = [[NSString alloc] initWithBytes:body_tr_bytes
        length:body_tr_len encoding:NSUTF8StringEncoding];
    if (body_en == nil || body_tr == nil) {
        return SATURDAY_VOICE_INVALID_INPUT;
    }
    NSString *language = [[NSLocale currentLocale] objectForKey:NSLocaleLanguageCode];
    BOOL turkish = [language isEqualToString:@"tr"];
    NSAlert *alert = [[NSAlert alloc] init];
    alert.alertStyle = NSAlertStyleWarning;
    alert.messageText = purpose == 3
        ? (turkish ? @"Bu ses oturumu için yerel modelleri ve Touch ID yetkisini onaylıyor musunuz?" : @"Authorize local models and Touch ID for this voice session?")
        : (purpose == 2
            ? (turkish ? @"Yerel ses yetkisini güvenli biçimde kurulsun mu?" : @"Set up local voice trust?")
            : (purpose == 1
                ? (turkish ? @"Qwen yerel modelinin kullanımını onaylıyor musunuz?" : @"Approve local Qwen model use?")
                : (turkish ? @"Yerel ses modellerinin kurulmasını onaylıyor musunuz?" : @"Approve installing local voice models?")));
    alert.informativeText = turkish ? body_tr : body_en;
    /* Cancel is deliberately the default/first action. */
    [alert addButtonWithTitle:turkish ? @"Vazgeç" : @"Cancel"];
    [alert addButtonWithTitle:purpose == 3
        ? (turkish ? @"Bu Oturum İçin Onayla" : @"Authorize This Session")
        : (purpose == 2
            ? (turkish ? @"Güvenli Kur ve Touch ID ile Onayla" : @"Set up and authorize with Touch ID")
            : (purpose == 1
                ? (turkish ? @"Qwen modelini Touch ID ile yetkilendir" : @"Authorize Qwen with Touch ID")
                : (turkish ? @"Devam et ve Touch ID ile onayla" : @"Continue and authorize with Touch ID")))];
    NSModalResponse response = [alert runModal];
    return response == NSAlertSecondButtonReturn ? SATURDAY_VOICE_SUCCESS : SATURDAY_VOICE_CANCELLED;
}

void saturday_voice_cf_release(saturday_voice_cf_data_ref value) {
    if (value != NULL) CFRelease(value);
}
static CFDataRef copy_requirement_data(SecCodeRef code) {
    SecRequirementRef requirement = NULL;
    OSStatus status = SecCodeCopyDesignatedRequirement(code, kSecCSDefaultFlags, &requirement);
    if (status != errSecSuccess || requirement == NULL) {
        if (requirement != NULL) CFRelease(requirement);
        return NULL;
    }
    CFDataRef data = NULL;
    status = SecRequirementCopyData(requirement, kSecCSDefaultFlags, &data);
    CFRelease(requirement);
    return status == errSecSuccess ? data : NULL;
}

static CFDataRef copy_team_identifier(SecCodeRef code) {
    CFDictionaryRef information = NULL;
    if (SecCodeCopySigningInformation(code, kSecCSSigningInformation, &information) != errSecSuccess
        || information == NULL) {
        if (information != NULL) CFRelease(information);
        return NULL;
    }
    CFTypeRef team = CFDictionaryGetValue(information, kSecCodeInfoTeamIdentifier);
    CFDataRef data = NULL;
    if (team != NULL && CFGetTypeID(team) == CFStringGetTypeID()) {
        data = CFStringCreateExternalRepresentation(kCFAllocatorDefault,
            (CFStringRef)team, kCFStringEncodingUTF8, 0);
    }
    CFRelease(information);
    if (data != NULL && (CFDataGetLength(data) == 0 || CFDataGetLength(data) > 256)) {
        CFRelease(data);
        data = NULL;
    }
    return data;
}

static CFDataRef copy_bundle_identifier(SecCodeRef code) {
    CFDictionaryRef information = NULL;
    if (SecCodeCopySigningInformation(code, kSecCSSigningInformation, &information) != errSecSuccess
        || information == NULL) {
        if (information != NULL) CFRelease(information);
        return NULL;
    }
    CFTypeRef identifier = CFDictionaryGetValue(information, kSecCodeInfoIdentifier);
    CFDataRef data = NULL;
    if (identifier != NULL && CFGetTypeID(identifier) == CFStringGetTypeID()) {
        data = CFStringCreateExternalRepresentation(kCFAllocatorDefault,
            (CFStringRef)identifier, kCFStringEncodingUTF8, 0);
    }
    CFRelease(information);
    if (data != NULL && (CFDataGetLength(data) == 0 || CFDataGetLength(data) > 256)) {
        CFRelease(data);
        data = NULL;
    }
    return data;
}

int32_t saturday_voice_copy_code_identity(const char *path_utf8,
    saturday_voice_cf_data_ref *team_out,
    saturday_voice_cf_data_ref *requirement_out,
    saturday_voice_cf_data_ref *bundle_identifier_out) {
    if (team_out == NULL || requirement_out == NULL || bundle_identifier_out == NULL) return SATURDAY_VOICE_INVALID_INPUT;
    *team_out = NULL;
    *requirement_out = NULL;
    *bundle_identifier_out = NULL;
    SecCodeRef code = NULL;
    SecStaticCodeRef static_code = NULL;
    if (path_utf8 == NULL) {
        OSStatus status = SecCodeCopySelf(kSecCSDefaultFlags, &code);
        if (status != errSecSuccess || code == NULL) return map_status(status);
        status = SecCodeCheckValidity(code, kSecCSStrictValidate, NULL);
        if (status != errSecSuccess) {
            CFRelease(code);
            return map_status(status);
        }
    } else {
        NSString *path = string_from_utf8(path_utf8, 4096);
        if (path == nil) return SATURDAY_VOICE_INVALID_INPUT;
        CFURLRef url = CFURLCreateWithFileSystemPath(kCFAllocatorDefault,
            (CFStringRef)path, kCFURLPOSIXPathStyle, false);
        if (url == NULL) return SATURDAY_VOICE_INVALID_INPUT;
        OSStatus status = SecStaticCodeCreateWithPath(url, kSecCSDefaultFlags, &static_code);
        CFRelease(url);
        if (status != errSecSuccess || static_code == NULL) {
            if (static_code != NULL) CFRelease(static_code);
            return map_status(status);
        }
        status = SecStaticCodeCheckValidity(static_code, kSecCSStrictValidate, NULL);
        if (status != errSecSuccess) {
            CFRelease(static_code);
            return map_status(status);
        }
        code = (SecCodeRef)static_code;
    }
    CFDataRef team = copy_team_identifier(code);
    CFDataRef requirement = copy_requirement_data(code);
    CFDataRef bundle_identifier = copy_bundle_identifier(code);
    CFRelease(code);
    if (team == NULL || requirement == NULL || bundle_identifier == NULL) {
        if (team != NULL) CFRelease(team);
        if (requirement != NULL) CFRelease(requirement);
        if (bundle_identifier != NULL) CFRelease(bundle_identifier);
        return SATURDAY_VOICE_UNAVAILABLE;
    }
    *team_out = team;
    *requirement_out = requirement;
    *bundle_identifier_out = bundle_identifier;
    return SATURDAY_VOICE_SUCCESS;
}
