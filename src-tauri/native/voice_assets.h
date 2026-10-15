#ifndef SATURDAY_VOICE_ASSETS_H
#define SATURDAY_VOICE_ASSETS_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct SaturdayAssetTransferHandle SaturdayAssetTransferHandle;

typedef enum SaturdayAssetTransferResult {
    SATURDAY_ASSET_TRANSFER_PENDING = 0,
    SATURDAY_ASSET_TRANSFER_OK = 1,
    SATURDAY_ASSET_TRANSFER_CANCELLED = 2,
    SATURDAY_ASSET_TRANSFER_TIMEOUT = 3,
    SATURDAY_ASSET_TRANSFER_NETWORK_ERROR = 4,
    SATURDAY_ASSET_TRANSFER_RESPONSE_REJECTED = 5,
    SATURDAY_ASSET_TRANSFER_SIZE_REJECTED = 6,
    SATURDAY_ASSET_TRANSFER_STORAGE_ERROR = 7,
    SATURDAY_ASSET_TRANSFER_REDIRECT_REJECTED = 8,
    SATURDAY_ASSET_TRANSFER_INVALID_ARGUMENT = 9
} SaturdayAssetTransferResult;

SaturdayAssetTransferResult saturday_asset_transfer_start(
    const char *url,
    const char *const *allowed_origins,
    size_t allowed_origin_count,
    int destination_fd,
    uint64_t expected_bytes,
    uint32_t timeout_milliseconds,
    SaturdayAssetTransferHandle **out_handle);

int saturday_asset_transfer_poll(
    SaturdayAssetTransferHandle *handle,
    uint64_t *out_bytes_received,
    uint32_t *out_result,
    uint32_t *out_complete);

int saturday_asset_transfer_cancel_and_wait(
    SaturdayAssetTransferHandle *handle,
    uint64_t *out_bytes_received,
    uint32_t *out_result);

void saturday_asset_transfer_destroy(SaturdayAssetTransferHandle *handle);

#ifdef __cplusplus
}
#endif

#endif
