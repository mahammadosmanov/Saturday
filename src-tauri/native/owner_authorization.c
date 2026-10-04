#include <CoreFoundation/CoreFoundation.h>
#include <LocalAuthentication/LocalAuthentication.h>
#include <Security/Security.h>
#include <limits.h>
#include <stdint.h>
#include <stddef.h>

typedef const void *saturday_cf_data_ref;

// Stable result codes mirrored by src/platform/owner_authorization.rs.
enum {
    SATURDAY_OWNER_SUCCESS = 0,
    SATURDAY_OWNER_SECURE_ENCLAVE_UNAVAILABLE = -1,
    SATURDAY_OWNER_KEY_NOT_FOUND = -2,
    SATURDAY_OWNER_DUPLICATE_KEY = -3,
    SATURDAY_OWNER_USER_CANCELLED = -4,
    SATURDAY_OWNER_AUTHENTICATION_FAILED = -5,
    SATURDAY_OWNER_INVALID_PUBLIC_KEY = -6,
    SATURDAY_OWNER_INVALID_SIGNATURE = -7,
    SATURDAY_OWNER_SYSTEM = -8,
};

static int32_t map_status(OSStatus status) {
    switch (status) {
        case errSecSuccess:
            return SATURDAY_OWNER_SUCCESS;
        case errSecItemNotFound:
            return SATURDAY_OWNER_KEY_NOT_FOUND;
        case errSecDuplicateItem:
            return SATURDAY_OWNER_DUPLICATE_KEY;
        case errSecUserCanceled:
            return SATURDAY_OWNER_USER_CANCELLED;
        case errSecAuthFailed:
        case errSecInteractionNotAllowed:
            return SATURDAY_OWNER_AUTHENTICATION_FAILED;
        case errSecNotAvailable:
        case errSecUnimplemented:
        case errSecParam:
            return SATURDAY_OWNER_SECURE_ENCLAVE_UNAVAILABLE;
        case errSecVerifyFailed:
            return SATURDAY_OWNER_INVALID_SIGNATURE;
        default:
            return SATURDAY_OWNER_SYSTEM;
    }
}

static int32_t map_error(CFErrorRef error) {
    if (error == NULL) {
        return SATURDAY_OWNER_SYSTEM;
    }
    return map_status((OSStatus)CFErrorGetCode(error));
}

static CFDataRef create_data(const uint8_t *bytes, size_t length) {
    if (bytes == NULL || length == 0 || length > (size_t)LONG_MAX) {
        return NULL;
    }
    return CFDataCreate(kCFAllocatorDefault, bytes, (CFIndex)length);
}

static LAContext *create_authentication_context(const char *prompt_utf8) {
    if (prompt_utf8 == NULL || prompt_utf8[0] == '\0') {
        return nil;
    }
    NSString *prompt = [[NSString alloc] initWithUTF8String:prompt_utf8];
    if (prompt == nil) {
        return nil;
    }
    LAContext *context = [[LAContext alloc] init];
    context.localizedReason = prompt;
    [prompt release];
    return context;
}

static CFDictionaryRef create_exact_key_query(
    CFDataRef tag,
    LAContext *authentication_context,
    Boolean return_reference
) {
    if (tag == NULL) {
        return NULL;
    }

    const void *keys[8];
    const void *values[8];
    CFIndex count = 0;
    keys[count] = kSecClass;
    values[count++] = kSecClassKey;
    keys[count] = kSecAttrKeyClass;
    values[count++] = kSecAttrKeyClassPrivate;
    keys[count] = kSecAttrKeyType;
    values[count++] = kSecAttrKeyTypeECSECPrimeRandom;
    keys[count] = kSecAttrTokenID;
    values[count++] = kSecAttrTokenIDSecureEnclave;
    keys[count] = kSecAttrApplicationTag;
    values[count++] = tag;
    if (authentication_context != nil) {
        keys[count] = kSecUseAuthenticationContext;
        values[count++] = authentication_context;
    }
    if (return_reference) {
        keys[count] = kSecReturnRef;
        values[count++] = kCFBooleanTrue;
        keys[count] = kSecMatchLimit;
        values[count++] = kSecMatchLimitOne;
    }
    return CFDictionaryCreate(
        kCFAllocatorDefault,
        keys,
        values,
        count,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks
    );
}

static int validate_secure_enclave_private_key(SecKeyRef private_key) {
    if (private_key == NULL) {
        return 0;
    }
    CFDictionaryRef attributes = SecKeyCopyAttributes(private_key);
    if (attributes == NULL) {
        return 0;
    }
    CFTypeRef token = CFDictionaryGetValue(attributes, kSecAttrTokenID);
    CFTypeRef key_class = CFDictionaryGetValue(attributes, kSecAttrKeyClass);
    CFTypeRef key_type = CFDictionaryGetValue(attributes, kSecAttrKeyType);
    int valid = token != NULL
        && CFEqual(token, kSecAttrTokenIDSecureEnclave)
        && key_class != NULL
        && CFEqual(key_class, kSecAttrKeyClassPrivate)
        && key_type != NULL
        && CFEqual(key_type, kSecAttrKeyTypeECSECPrimeRandom);
    CFRelease(attributes);
    return valid;
}

static int32_t copy_exact_private_key(
    CFDataRef tag,
    LAContext *authentication_context,
    SecKeyRef *private_key_out
) {
    if (private_key_out == NULL) {
        return SATURDAY_OWNER_SYSTEM;
    }
    *private_key_out = NULL;
    CFDictionaryRef query = create_exact_key_query(
        tag,
        authentication_context,
        true
    );
    if (query == NULL) {
        return SATURDAY_OWNER_SYSTEM;
    }
    CFTypeRef result = NULL;
    OSStatus status = SecItemCopyMatching(query, &result);
    CFRelease(query);
    if (status != errSecSuccess) {
        if (result != NULL) {
            CFRelease(result);
        }
        return map_status(status);
    }
    if (result == NULL || CFGetTypeID(result) != SecKeyGetTypeID()) {
        if (result != NULL) {
            CFRelease(result);
        }
        return SATURDAY_OWNER_SYSTEM;
    }
    SecKeyRef private_key = (SecKeyRef)result;
    if (!validate_secure_enclave_private_key(private_key)) {
        CFRelease(private_key);
        return SATURDAY_OWNER_SECURE_ENCLAVE_UNAVAILABLE;
    }
    *private_key_out = private_key;
    return SATURDAY_OWNER_SUCCESS;
}

static int32_t create_secure_enclave_private_key(
    CFDataRef tag,
    SecKeyRef *private_key_out
) {
    if (private_key_out == NULL) {
        return SATURDAY_OWNER_SYSTEM;
    }
    *private_key_out = NULL;

    CFErrorRef access_error = NULL;
    SecAccessControlRef access_control = SecAccessControlCreateWithFlags(
        kCFAllocatorDefault,
        kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        kSecAccessControlPrivateKeyUsage | kSecAccessControlUserPresence,
        &access_error
    );
    if (access_control == NULL) {
        int32_t result = map_error(access_error);
        if (access_error != NULL) {
            CFRelease(access_error);
        }
        return result;
    }
    if (access_error != NULL) {
        CFRelease(access_error);
    }

    const void *private_keys[] = {
        kSecAttrIsPermanent,
        kSecAttrApplicationTag,
        kSecAttrAccessControl,
    };
    const void *private_values[] = {
        kCFBooleanTrue,
        tag,
        access_control,
    };
    CFDictionaryRef private_attributes = CFDictionaryCreate(
        kCFAllocatorDefault,
        private_keys,
        private_values,
        3,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks
    );
    if (private_attributes == NULL) {
        CFRelease(access_control);
        return SATURDAY_OWNER_SYSTEM;
    }

    int32_t key_size = 256;
    CFNumberRef key_size_number = CFNumberCreate(
        kCFAllocatorDefault,
        kCFNumberSInt32Type,
        &key_size
    );
    if (key_size_number == NULL) {
        CFRelease(private_attributes);
        CFRelease(access_control);
        return SATURDAY_OWNER_SYSTEM;
    }

    const void *attribute_keys[] = {
        kSecAttrKeyType,
        kSecAttrKeySizeInBits,
        kSecAttrTokenID,
        kSecPrivateKeyAttrs,
    };
    const void *attribute_values[] = {
        kSecAttrKeyTypeECSECPrimeRandom,
        key_size_number,
        kSecAttrTokenIDSecureEnclave,
        private_attributes,
    };
    CFDictionaryRef attributes = CFDictionaryCreate(
        kCFAllocatorDefault,
        attribute_keys,
        attribute_values,
        4,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks
    );
    if (attributes == NULL) {
        CFRelease(key_size_number);
        CFRelease(private_attributes);
        CFRelease(access_control);
        return SATURDAY_OWNER_SYSTEM;
    }

    CFErrorRef create_error = NULL;
    SecKeyRef private_key = SecKeyCreateRandomKey(attributes, &create_error);
    CFRelease(attributes);
    CFRelease(key_size_number);
    CFRelease(private_attributes);
    CFRelease(access_control);
    if (private_key == NULL) {
        int32_t result = map_error(create_error);
        if (create_error != NULL) {
            CFRelease(create_error);
        }
        return result;
    }
    if (create_error != NULL) {
        CFRelease(create_error);
    }
    if (!validate_secure_enclave_private_key(private_key)) {
        CFRelease(private_key);
        return SATURDAY_OWNER_SECURE_ENCLAVE_UNAVAILABLE;
    }
    *private_key_out = private_key;
    return SATURDAY_OWNER_SUCCESS;
}

static int32_t copy_public_data(SecKeyRef private_key, CFDataRef *public_data_out) {
    if (public_data_out == NULL) {
        return SATURDAY_OWNER_SYSTEM;
    }
    *public_data_out = NULL;
    SecKeyRef public_key = SecKeyCopyPublicKey(private_key);
    if (public_key == NULL) {
        return SATURDAY_OWNER_INVALID_PUBLIC_KEY;
    }
    CFErrorRef error = NULL;
    CFDataRef public_data = SecKeyCopyExternalRepresentation(public_key, &error);
    CFRelease(public_key);
    if (public_data == NULL) {
        int32_t result = map_error(error);
        if (error != NULL) {
            CFRelease(error);
        }
        return result;
    }
    if (error != NULL) {
        CFRelease(error);
    }
    if (CFDataGetLength(public_data) != 65
        || CFDataGetBytePtr(public_data) == NULL
        || CFDataGetBytePtr(public_data)[0] != 0x04) {
        CFRelease(public_data);
        return SATURDAY_OWNER_INVALID_PUBLIC_KEY;
    }
    *public_data_out = public_data;
    return SATURDAY_OWNER_SUCCESS;
}

static int32_t create_verified_proof(
    SecKeyRef private_key,
    CFDataRef message,
    CFDataRef *public_data_out,
    CFDataRef *signature_out
) {
    *public_data_out = NULL;
    *signature_out = NULL;
    CFDataRef public_data = NULL;
    int32_t result = copy_public_data(private_key, &public_data);
    if (result != SATURDAY_OWNER_SUCCESS) {
        return result;
    }
    SecKeyRef public_key = SecKeyCopyPublicKey(private_key);
    if (public_key == NULL) {
        CFRelease(public_data);
        return SATURDAY_OWNER_INVALID_PUBLIC_KEY;
    }
    CFErrorRef sign_error = NULL;
    CFDataRef signature = SecKeyCreateSignature(
        private_key,
        kSecKeyAlgorithmECDSASignatureMessageX962SHA256,
        message,
        &sign_error
    );
    if (signature == NULL) {
        result = map_error(sign_error);
        if (sign_error != NULL) {
            CFRelease(sign_error);
        }
        CFRelease(public_key);
        CFRelease(public_data);
        return result;
    }
    if (sign_error != NULL) {
        CFRelease(sign_error);
    }
    CFErrorRef verify_error = NULL;
    Boolean valid = SecKeyVerifySignature(
        public_key,
        kSecKeyAlgorithmECDSASignatureMessageX962SHA256,
        message,
        signature,
        &verify_error
    );
    CFRelease(public_key);
    if (!valid) {
        if (verify_error != NULL) {
            CFRelease(verify_error);
        }
        CFRelease(signature);
        CFRelease(public_data);
        return SATURDAY_OWNER_INVALID_SIGNATURE;
    }
    if (verify_error != NULL) {
        CFRelease(verify_error);
    }
    *public_data_out = public_data;
    *signature_out = signature;
    return SATURDAY_OWNER_SUCCESS;
}

int32_t saturday_owner_key_provision(
    const uint8_t *tag,
    size_t tag_len,
    const uint8_t *challenge,
    size_t challenge_len,
    saturday_cf_data_ref *public_key_out,
    saturday_cf_data_ref *signature_out
) {
    if (public_key_out == NULL || signature_out == NULL) {
        return SATURDAY_OWNER_SYSTEM;
    }
    *public_key_out = NULL;
    *signature_out = NULL;
    CFDataRef tag_data = create_data(tag, tag_len);
    CFDataRef challenge_data = create_data(challenge, challenge_len);
    if (tag_data == NULL || challenge_data == NULL) {
        if (tag_data != NULL) {
            CFRelease(tag_data);
        }
        if (challenge_data != NULL) {
            CFRelease(challenge_data);
        }
        return SATURDAY_OWNER_SYSTEM;
    }

    SecKeyRef private_key = NULL;
    CFDataRef public_key = NULL;
    CFDataRef signature = NULL;
    int32_t result = copy_exact_private_key(tag_data, NULL, &private_key);
    if (result == SATURDAY_OWNER_KEY_NOT_FOUND) {
        result = create_secure_enclave_private_key(tag_data, &private_key);
    }
    if (result == SATURDAY_OWNER_SUCCESS) {
        result = create_verified_proof(
            private_key,
            challenge_data,
            &public_key,
            &signature
        );
    }
    if (result == SATURDAY_OWNER_SUCCESS) {
        *public_key_out = public_key;
        *signature_out = signature;
    }
    if (private_key != NULL) {
        CFRelease(private_key);
    }
    CFRelease(challenge_data);
    CFRelease(tag_data);
    return result;
}

int32_t saturday_owner_key_copy_public(
    const uint8_t *tag,
    size_t tag_len,
    saturday_cf_data_ref *public_key_out
) {
    if (public_key_out == NULL) {
        return SATURDAY_OWNER_SYSTEM;
    }
    *public_key_out = NULL;
    CFDataRef tag_data = create_data(tag, tag_len);
    if (tag_data == NULL) {
        return SATURDAY_OWNER_SYSTEM;
    }
    SecKeyRef private_key = NULL;
    CFDataRef public_key = NULL;
    int32_t result = copy_exact_private_key(tag_data, NULL, &private_key);
    if (result == SATURDAY_OWNER_SUCCESS) {
        result = copy_public_data(private_key, &public_key);
    }
    if (result == SATURDAY_OWNER_SUCCESS) {
        *public_key_out = public_key;
    }
    if (private_key != NULL) {
        CFRelease(private_key);
    }
    CFRelease(tag_data);
    return result;
}

int32_t saturday_owner_key_sign(
    const uint8_t *tag,
    size_t tag_len,
    const uint8_t *message,
    size_t message_len,
    const char *prompt_utf8,
    saturday_cf_data_ref *public_key_out,
    saturday_cf_data_ref *signature_out
) {
    if (public_key_out == NULL || signature_out == NULL) {
        return SATURDAY_OWNER_SYSTEM;
    }
    *public_key_out = NULL;
    *signature_out = NULL;
    CFDataRef tag_data = create_data(tag, tag_len);
    CFDataRef message_data = create_data(message, message_len);
    LAContext *authentication_context = create_authentication_context(prompt_utf8);
    if (tag_data == NULL || message_data == NULL || authentication_context == nil) {
        if (tag_data != NULL) {
            CFRelease(tag_data);
        }
        if (message_data != NULL) {
            CFRelease(message_data);
        }
        if (authentication_context != nil) {
            [authentication_context release];
        }
        return SATURDAY_OWNER_SYSTEM;
    }
    SecKeyRef private_key = NULL;
    CFDataRef public_key = NULL;
    CFDataRef signature = NULL;
    int32_t result = copy_exact_private_key(
        tag_data,
        authentication_context,
        &private_key
    );
    if (result == SATURDAY_OWNER_SUCCESS) {
        result = create_verified_proof(
            private_key,
            message_data,
            &public_key,
            &signature
        );
    }
    if (result == SATURDAY_OWNER_SUCCESS) {
        *public_key_out = public_key;
        *signature_out = signature;
    }
    if (private_key != NULL) {
        CFRelease(private_key);
    }
    [authentication_context release];
    CFRelease(message_data);
    CFRelease(tag_data);
    return result;
}

int32_t saturday_owner_key_delete_after_proof(
    const uint8_t *tag,
    size_t tag_len,
    const uint8_t *challenge,
    size_t challenge_len,
    const char *prompt_utf8
) {
    CFDataRef tag_data = create_data(tag, tag_len);
    CFDataRef challenge_data = create_data(challenge, challenge_len);
    LAContext *authentication_context = create_authentication_context(prompt_utf8);
    if (tag_data == NULL || challenge_data == NULL || authentication_context == nil) {
        if (tag_data != NULL) {
            CFRelease(tag_data);
        }
        if (challenge_data != NULL) {
            CFRelease(challenge_data);
        }
        if (authentication_context != nil) {
            [authentication_context release];
        }
        return SATURDAY_OWNER_SYSTEM;
    }

    SecKeyRef private_key = NULL;
    int32_t result = copy_exact_private_key(
        tag_data,
        authentication_context,
        &private_key
    );
    SecKeyRef public_key = NULL;
    CFDataRef signature = NULL;
    if (result == SATURDAY_OWNER_SUCCESS) {
        public_key = SecKeyCopyPublicKey(private_key);
        if (public_key == NULL) {
            result = SATURDAY_OWNER_INVALID_PUBLIC_KEY;
        }
    }
    if (result == SATURDAY_OWNER_SUCCESS) {
        CFErrorRef sign_error = NULL;
        signature = SecKeyCreateSignature(
            private_key,
            kSecKeyAlgorithmECDSASignatureMessageX962SHA256,
            challenge_data,
            &sign_error
        );
        if (signature == NULL) {
            result = map_error(sign_error);
        }
        if (sign_error != NULL) {
            CFRelease(sign_error);
        }
    }
    if (result == SATURDAY_OWNER_SUCCESS) {
        CFErrorRef verify_error = NULL;
        Boolean valid = SecKeyVerifySignature(
            public_key,
            kSecKeyAlgorithmECDSASignatureMessageX962SHA256,
            challenge_data,
            signature,
            &verify_error
        );
        if (!valid) {
            result = SATURDAY_OWNER_INVALID_SIGNATURE;
        }
        if (verify_error != NULL) {
            CFRelease(verify_error);
        }
    }
    if (result == SATURDAY_OWNER_SUCCESS) {
        CFDictionaryRef delete_query = create_exact_key_query(tag_data, NULL, false);
        if (delete_query == NULL) {
            result = SATURDAY_OWNER_SYSTEM;
        } else {
            result = map_status(SecItemDelete(delete_query));
            CFRelease(delete_query);
        }
    }
    if (result == SATURDAY_OWNER_SUCCESS) {
        SecKeyRef unexpected_key = NULL;
        int32_t confirm = copy_exact_private_key(tag_data, NULL, &unexpected_key);
        if (unexpected_key != NULL) {
            CFRelease(unexpected_key);
        }
        if (confirm != SATURDAY_OWNER_KEY_NOT_FOUND) {
            result = SATURDAY_OWNER_SYSTEM;
        }
    }

    if (signature != NULL) {
        CFRelease(signature);
    }
    if (public_key != NULL) {
        CFRelease(public_key);
    }
    if (private_key != NULL) {
        CFRelease(private_key);
    }
    [authentication_context release];
    CFRelease(challenge_data);
    CFRelease(tag_data);
    return result;
}

void saturday_owner_cf_release(const void *value) {
    if (value != NULL) {
        CFRelease(value);
    }
}
