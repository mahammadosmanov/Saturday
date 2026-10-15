#import "voice_assets.h"

#import <Foundation/Foundation.h>
#import <dispatch/dispatch.h>

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static NSString *const kWhisperURL = @"https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.bin";
static NSString *const kQwenURL = @"https://huggingface.co/Qwen/Qwen3-4B-GGUF/resolve/bc640142c66e1fdd12af0bd68f40445458f3869b/Qwen3-4B-Q4_K_M.gguf";
static NSArray<NSString *> *saturday_asset_pinned_origins(void) {
    return @[
        @"https://huggingface.co",
        @"https://cdn-lfs.huggingface.co",
        @"https://cas-bridge.xethub.hf.co",
        @"https://us.aws.cdn.hf.co"
    ];
}

struct SaturdayAssetTransferHandle {
    void *object;
};

@interface SaturdayAssetDownload : NSObject <NSURLSessionDataDelegate, NSURLSessionTaskDelegate>
@property(nonatomic, strong) NSCondition *condition;
@property(nonatomic, strong) NSURLSession *session;
@property(nonatomic, strong) NSURLSessionDataTask *task;
@property(nonatomic, copy) NSArray<NSString *> *allowedOrigins;
@property(nonatomic) int writerFD;
@property(nonatomic) uint64_t expectedBytes;
@property(nonatomic) uint64_t receivedBytes;
@property(nonatomic) uint32_t result;
@property(nonatomic) NSUInteger redirects;
@property(nonatomic) BOOL responseAccepted;
@property(nonatomic) BOOL cancellationRequested;
@property(nonatomic) BOOL complete;
@end

@implementation SaturdayAssetDownload

- (instancetype)initWithWriterFD:(int)writerFD
                   expectedBytes:(uint64_t)expectedBytes
                   timeoutMillis:(uint32_t)timeoutMillis
                  allowedOrigins:(NSArray<NSString *> *)allowedOrigins {
    self = [super init];
    if (self != nil) {
        _condition = [[NSCondition alloc] init];
        _writerFD = writerFD;
        _expectedBytes = expectedBytes;
        _result = SATURDAY_ASSET_TRANSFER_PENDING;
        _allowedOrigins = [allowedOrigins copy];

        NSURLSessionConfiguration *configuration = [NSURLSessionConfiguration ephemeralSessionConfiguration];
        configuration.URLCache = nil;
        configuration.HTTPCookieStorage = nil;
        configuration.URLCredentialStorage = nil;
        configuration.HTTPShouldSetCookies = NO;
        configuration.requestCachePolicy = NSURLRequestReloadIgnoringLocalCacheData;
        configuration.HTTPMaximumConnectionsPerHost = 1;
        configuration.timeoutIntervalForRequest = (NSTimeInterval)timeoutMillis / 1000.0;
        configuration.timeoutIntervalForResource = (NSTimeInterval)timeoutMillis / 1000.0;
        configuration.HTTPAdditionalHeaders = @{@"Accept-Encoding": @"identity"};
        if (@available(macOS 10.15, *)) {
            configuration.waitsForConnectivity = NO;
        }

        NSOperationQueue *delegateQueue = [[NSOperationQueue alloc] init];
        delegateQueue.name = @"app.saturday.voice.asset-transfer";
        delegateQueue.maxConcurrentOperationCount = 1;
        delegateQueue.qualityOfService = NSQualityOfServiceUtility;
        _session = [NSURLSession sessionWithConfiguration:configuration
                                                 delegate:self
                                            delegateQueue:delegateQueue];
    }
    return self;
}

- (BOOL)isAllowedOriginForURL:(NSURL *)url {
    NSURLComponents *components = [NSURLComponents componentsWithURL:url resolvingAgainstBaseURL:NO];
    if (components == nil || components.scheme.length == 0 || components.host.length == 0 ||
        components.user != nil || components.password != nil || components.fragment != nil ||
        ![components.scheme.lowercaseString isEqualToString:@"https"] ||
        (components.port != nil && components.port.integerValue != 443)) {
        return NO;
    }
    NSString *origin = [NSString stringWithFormat:@"https://%@", components.host.lowercaseString];
    return [self.allowedOrigins containsObject:origin];
}

- (void)rejectWithResult:(SaturdayAssetTransferResult)result {
    NSURLSessionDataTask *task = nil;
    [self.condition lock];
    if (!self.complete && self.result == SATURDAY_ASSET_TRANSFER_PENDING) {
        self.result = result;
        task = self.task;
    }
    [self.condition unlock];
    [task cancel];
}

- (void)URLSession:(NSURLSession *)session
              task:(NSURLSessionTask *)task
willPerformHTTPRedirection:(NSHTTPURLResponse *)response
        newRequest:(NSURLRequest *)request
 completionHandler:(void (^)(NSURLRequest * _Nullable))completionHandler {
    (void)session;
    (void)task;
    BOOL permitted = request.URL != nil && [self isAllowedOriginForURL:request.URL];
    [self.condition lock];
    if (self.cancellationRequested || self.complete || self.redirects >= 5) {
        permitted = NO;
    }
    if (permitted) {
        self.redirects += 1;
    } else if (!self.complete && self.result == SATURDAY_ASSET_TRANSFER_PENDING) {
        self.result = SATURDAY_ASSET_TRANSFER_REDIRECT_REJECTED;
    }
    [self.condition unlock];
    if (permitted) {
        completionHandler(request);
    } else {
        completionHandler(nil);
        [self.task cancel];
    }
}

- (void)URLSession:(NSURLSession *)session
          dataTask:(NSURLSessionDataTask *)dataTask
 didReceiveResponse:(NSURLResponse *)response
  completionHandler:(void (^)(NSURLSessionResponseDisposition))completionHandler {
    (void)session;
    (void)dataTask;
    BOOL accepted = NO;
    if ([response isKindOfClass:[NSHTTPURLResponse class]] &&
        [self isAllowedOriginForURL:response.URL]) {
        NSHTTPURLResponse *http = (NSHTTPURLResponse *)response;
        id lengthHeader = nil;
        id encodingHeader = nil;
        for (id key in http.allHeaderFields) {
            if ([[key description] caseInsensitiveCompare:@"Content-Length"] == NSOrderedSame) {
                lengthHeader = http.allHeaderFields[key];
            } else if ([[key description] caseInsensitiveCompare:@"Content-Encoding"] == NSOrderedSame) {
                encodingHeader = http.allHeaderFields[key];
            }
        }
        NSString *lengthText = [lengthHeader isKindOfClass:[NSString class]]
            ? (NSString *)lengthHeader
            : ([lengthHeader isKindOfClass:[NSNumber class]] ? [(NSNumber *)lengthHeader stringValue] : nil);
        BOOL decimalLength = lengthText.length > 0;
        uint64_t headerBytes = 0;
        if (decimalLength) {
            for (NSUInteger index = 0; index < lengthText.length; index++) {
                unichar scalar = [lengthText characterAtIndex:index];
                if (scalar < '0' || scalar > '9') {
                    decimalLength = NO;
                    break;
                }
                uint64_t digit = (uint64_t)(scalar - '0');
                if (headerBytes > (UINT64_MAX - digit) / 10) {
                    decimalLength = NO;
                    break;
                }
                headerBytes = headerBytes * 10 + digit;
            }
        }
        NSString *encoding = [encodingHeader isKindOfClass:[NSString class]]
            ? [(NSString *)encodingHeader lowercaseString]
            : nil;
        BOOL identityEncoding = encoding == nil || [encoding isEqualToString:@"identity"];
        accepted = http.statusCode == 200 && decimalLength &&
                   headerBytes == self.expectedBytes &&
                   response.expectedContentLength == (int64_t)self.expectedBytes &&
                   identityEncoding;
    }
    [self.condition lock];
    if (self.cancellationRequested || self.complete) {
        accepted = NO;
    }
    self.responseAccepted = accepted;
    if (!accepted && !self.complete && self.result == SATURDAY_ASSET_TRANSFER_PENDING) {
        self.result = SATURDAY_ASSET_TRANSFER_RESPONSE_REJECTED;
    }
    [self.condition unlock];
    completionHandler(accepted ? NSURLSessionResponseAllow : NSURLSessionResponseCancel);
    if (!accepted) {
        [self.task cancel];
    }
}

- (void)URLSession:(NSURLSession *)session dataTask:(NSURLSessionDataTask *)dataTask didReceiveData:(NSData *)data {
    (void)session;
    (void)dataTask;
    BOOL cancelTask = NO;
    [self.condition lock];
    if (self.cancellationRequested || self.complete || !self.responseAccepted ||
        (uint64_t)data.length > self.expectedBytes - self.receivedBytes) {
        if (!self.complete && self.result == SATURDAY_ASSET_TRANSFER_PENDING) {
            self.result = self.cancellationRequested
                ? SATURDAY_ASSET_TRANSFER_CANCELLED
                : SATURDAY_ASSET_TRANSFER_SIZE_REJECTED;
        }
        cancelTask = YES;
    } else {
        const uint8_t *bytes = data.bytes;
        size_t remaining = data.length;
        while (remaining > 0) {
            ssize_t written = write(self.writerFD, bytes, remaining);
            if (written < 0 && errno == EINTR) {
                continue;
            }
            if (written <= 0) {
                self.result = SATURDAY_ASSET_TRANSFER_STORAGE_ERROR;
                cancelTask = YES;
                break;
            }
            bytes += (size_t)written;
            remaining -= (size_t)written;
        }
        if (!cancelTask) {
            self.receivedBytes += (uint64_t)data.length;
        }
    }
    [self.condition unlock];
    if (cancelTask) {
        [self.task cancel];
    }
}

- (void)URLSession:(NSURLSession *)session task:(NSURLSessionTask *)task didCompleteWithError:(NSError *)error {
    (void)session;
    (void)task;
    [self.condition lock];
    int writerFD = self.writerFD;
    self.writerFD = -1;
    if (writerFD >= 0 && close(writerFD) != 0 &&
        self.result == SATURDAY_ASSET_TRANSFER_PENDING) {
        self.result = SATURDAY_ASSET_TRANSFER_STORAGE_ERROR;
    }
    if (self.result == SATURDAY_ASSET_TRANSFER_PENDING) {
        if (error != nil) {
            self.result = SATURDAY_ASSET_TRANSFER_NETWORK_ERROR;
        } else if (!self.responseAccepted || self.receivedBytes != self.expectedBytes) {
            self.result = SATURDAY_ASSET_TRANSFER_SIZE_REJECTED;
        } else {
            self.result = SATURDAY_ASSET_TRANSFER_OK;
        }
    }
    self.complete = YES;
    [self.condition broadcast];
    [self.condition unlock];
    [self.session finishTasksAndInvalidate];
}

- (void)cancelAndWait {
    NSURLSessionDataTask *task = nil;
    [self.condition lock];
    if (!self.complete) {
        self.cancellationRequested = YES;
        if (self.result == SATURDAY_ASSET_TRANSFER_PENDING) {
            self.result = SATURDAY_ASSET_TRANSFER_CANCELLED;
        }
        task = self.task;
    }
    [self.condition unlock];
    [task cancel];

    [self.condition lock];
    while (!self.complete) {
        [self.condition wait];
    }
    [self.condition unlock];
}

- (void)closeWriterBeforeTask {
    [self.condition lock];
    if (self.writerFD >= 0) {
        close(self.writerFD);
        self.writerFD = -1;
    }
    self.result = SATURDAY_ASSET_TRANSFER_STORAGE_ERROR;
    self.complete = YES;
    [self.condition broadcast];
    [self.condition unlock];
    [self.session invalidateAndCancel];
}

- (void)copyBytes:(uint64_t *)bytes result:(uint32_t *)result complete:(uint32_t *)complete {
    [self.condition lock];
    if (bytes != NULL) {
        *bytes = self.receivedBytes;
    }
    if (result != NULL) {
        *result = self.result;
    }
    if (complete != NULL) {
        *complete = self.complete ? 1u : 0u;
    }
    [self.condition unlock];
}

@end

static BOOL saturday_asset_url_is_pinned(NSString *url, uint64_t expectedBytes) {
    if ([url isEqualToString:kWhisperURL]) {
        return expectedBytes == UINT64_C(147951465);
    }
    if ([url isEqualToString:kQwenURL]) {
        return expectedBytes == UINT64_C(2497280256);
    }
    return NO;
}

SaturdayAssetTransferResult saturday_asset_transfer_start(
    const char *url,
    const char *const *allowed_origins,
    size_t allowed_origin_count,
    int destination_fd,
    uint64_t expected_bytes,
    uint32_t timeout_milliseconds,
    SaturdayAssetTransferHandle **out_handle) {
    if (out_handle != NULL) {
        *out_handle = NULL;
    }
    if (url == NULL || allowed_origins == NULL || out_handle == NULL || destination_fd < 0 ||
        expected_bytes == 0 || expected_bytes > UINT64_C(4294967296) ||
        timeout_milliseconds == 0 || timeout_milliseconds > 900000) {
        return SATURDAY_ASSET_TRANSFER_INVALID_ARGUMENT;
    }

    NSArray<NSString *> *pinnedOrigins = saturday_asset_pinned_origins();
    if (allowed_origin_count != pinnedOrigins.count) {
        return SATURDAY_ASSET_TRANSFER_INVALID_ARGUMENT;
    }
    NSMutableArray<NSString *> *origins = [[NSMutableArray alloc] initWithCapacity:allowed_origin_count];
    for (size_t index = 0; index < allowed_origin_count; index++) {
        if (allowed_origins[index] == NULL || strnlen(allowed_origins[index], 257) > 256) {
            return SATURDAY_ASSET_TRANSFER_INVALID_ARGUMENT;
        }
        NSString *origin = [[NSString alloc] initWithUTF8String:allowed_origins[index]];
        if (origin == nil || ![origin isEqualToString:pinnedOrigins[index]]) {
            return SATURDAY_ASSET_TRANSFER_INVALID_ARGUMENT;
        }
        [origins addObject:origin];
    }

    size_t urlLength = strnlen(url, 2049);
    if (urlLength == 0 || urlLength > 2048) {
        return SATURDAY_ASSET_TRANSFER_INVALID_ARGUMENT;
    }
    NSString *urlString = [[NSString alloc] initWithBytes:url length:urlLength encoding:NSUTF8StringEncoding];
    if (urlString == nil || !saturday_asset_url_is_pinned(urlString, expected_bytes)) {
        return SATURDAY_ASSET_TRANSFER_INVALID_ARGUMENT;
    }
    NSURL *requestURL = [NSURL URLWithString:urlString];
    if (requestURL == nil) {
        return SATURDAY_ASSET_TRANSFER_INVALID_ARGUMENT;
    }
    struct stat status;
    int flags = fcntl(destination_fd, F_GETFL);
    if (flags < 0 || (flags & O_ACCMODE) != O_RDWR || (flags & O_APPEND) != 0 ||
        fstat(destination_fd, &status) != 0 || !S_ISREG(status.st_mode) ||
        status.st_uid != geteuid() || status.st_nlink != 1 || status.st_size != 0 ||
        (status.st_mode & 0777) != 0600) {
        return SATURDAY_ASSET_TRANSFER_INVALID_ARGUMENT;
    }
    int writerFD = fcntl(destination_fd, F_DUPFD_CLOEXEC, 0);
    if (writerFD < 0) {
        return SATURDAY_ASSET_TRANSFER_STORAGE_ERROR;
    }
    SaturdayAssetDownload *download = [[SaturdayAssetDownload alloc]
        initWithWriterFD:writerFD
        expectedBytes:expected_bytes
        timeoutMillis:timeout_milliseconds
        allowedOrigins:origins];
    if (download == nil) {
        close(writerFD);
        return SATURDAY_ASSET_TRANSFER_NETWORK_ERROR;
    }
    NSMutableURLRequest *request = [NSMutableURLRequest requestWithURL:requestURL
                                                            cachePolicy:NSURLRequestReloadIgnoringLocalCacheData
                                                        timeoutInterval:(NSTimeInterval)timeout_milliseconds / 1000.0];
    request.HTTPMethod = @"GET";
    request.HTTPShouldHandleCookies = NO;
    [request setValue:@"identity" forHTTPHeaderField:@"Accept-Encoding"];
    download.task = [download.session dataTaskWithRequest:request];
    if (download.task == nil) {
        [download closeWriterBeforeTask];
        return SATURDAY_ASSET_TRANSFER_NETWORK_ERROR;
    }
    SaturdayAssetTransferHandle *handle = calloc(1, sizeof(*handle));
    if (handle == NULL) {
        [download closeWriterBeforeTask];
        return SATURDAY_ASSET_TRANSFER_STORAGE_ERROR;
    }
    handle->object = (__bridge_retained void *)download;
    *out_handle = handle;
    [download.task resume];
    return SATURDAY_ASSET_TRANSFER_PENDING;
}

int saturday_asset_transfer_poll(
    SaturdayAssetTransferHandle *handle,
    uint64_t *out_bytes_received,
    uint32_t *out_result,
    uint32_t *out_complete) {
    if (handle == NULL || handle->object == NULL || out_bytes_received == NULL ||
        out_result == NULL || out_complete == NULL) {
        return -1;
    }
    SaturdayAssetDownload *download = (__bridge SaturdayAssetDownload *)handle->object;
    [download copyBytes:out_bytes_received result:out_result complete:out_complete];
    return 0;
}

int saturday_asset_transfer_cancel_and_wait(
    SaturdayAssetTransferHandle *handle,
    uint64_t *out_bytes_received,
    uint32_t *out_result) {
    if (handle == NULL || handle->object == NULL || out_bytes_received == NULL || out_result == NULL) {
        return -1;
    }
    SaturdayAssetDownload *download = (__bridge SaturdayAssetDownload *)handle->object;
    [download cancelAndWait];
    [download copyBytes:out_bytes_received result:out_result complete:NULL];
    return 0;
}

void saturday_asset_transfer_destroy(SaturdayAssetTransferHandle *handle) {
    if (handle == NULL) {
        return;
    }
    if (handle->object != NULL) {
        SaturdayAssetDownload *download = (__bridge_transfer SaturdayAssetDownload *)handle->object;
        [download cancelAndWait];
        [download.session invalidateAndCancel];
        handle->object = NULL;
    }
    free(handle);
}
