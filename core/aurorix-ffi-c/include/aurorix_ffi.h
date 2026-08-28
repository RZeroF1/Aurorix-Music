#ifndef AURORIX_FFI_H
#define AURORIX_FFI_H

#include <stdint.h>

#if defined(_WIN32)
#define AURORIX_API __declspec(dllimport)
#else
#define AURORIX_API
#endif

#ifdef __cplusplus
extern "C" {
#endif

#define AURORIX_STATUS_OK 0
#define AURORIX_STATUS_INVALID_ARGUMENT 1
#define AURORIX_STATUS_INVALID_HANDLE 2
#define AURORIX_STATUS_INCOMPATIBLE_VERSION 3
#define AURORIX_STATUS_SHUTDOWN 4
#define AURORIX_STATUS_ALREADY_CANCELLED 5
#define AURORIX_STATUS_CANCELLED 6
#define AURORIX_STATUS_CALLBACK_REJECTED 7
#define AURORIX_STATUS_PANIC 8
#define AURORIX_STATUS_SHUTDOWN_INCOMPLETE 9
#define AURORIX_STATUS_REENTRANT_RELEASE 10

#define AURORIX_OUTCOME_COMPLETED 0
#define AURORIX_OUTCOME_CANCELLED_BEFORE_COMMIT 1
#define AURORIX_OUTCOME_CANCELLED_OUTCOME_UNKNOWN 2

typedef struct AurorixByteSliceV1 {
    const uint8_t* ptr;
    uint64_t len;
} AurorixByteSliceV1;

typedef struct AurorixBufferV1 {
    uint8_t* ptr;
    uint64_t len;
} AurorixBufferV1;

typedef struct AurorixClientConfigV1 {
    AurorixByteSliceV1 data_dir;
    uint32_t shutdown_timeout_ms;
} AurorixClientConfigV1;

typedef struct AurorixErrorV1 {
    int32_t code;
    AurorixBufferV1 message;
} AurorixErrorV1;

typedef struct AurorixClientHandle AurorixClientHandle;
typedef struct AurorixOperationHandle AurorixOperationHandle;
typedef struct AurorixSubscriptionHandle AurorixSubscriptionHandle;

typedef void (*AurorixCompletionV1)(
    void* context,
    int32_t status,
    int32_t outcome,
    AurorixByteSliceV1 response);

typedef void (*AurorixEventSinkV1)(
    void* context,
    uint64_t event_sequence,
    AurorixByteSliceV1 event);

AURORIX_API AurorixClientHandle* aurorix_client_create_v1(
    const AurorixClientConfigV1* config,
    AurorixErrorV1* error);

AURORIX_API int32_t aurorix_client_command_v1(
    AurorixClientHandle* client,
    AurorixByteSliceV1 request,
    AurorixCompletionV1 callback,
    void* context,
    AurorixOperationHandle** operation);

AURORIX_API int32_t aurorix_client_query_v1(
    AurorixClientHandle* client,
    AurorixByteSliceV1 request,
    AurorixCompletionV1 callback,
    void* context,
    AurorixOperationHandle** operation);

AURORIX_API int32_t aurorix_client_subscribe_v1(
    AurorixClientHandle* client,
    AurorixByteSliceV1 request,
    AurorixEventSinkV1 callback,
    void* context,
    AurorixSubscriptionHandle** subscription,
    uint64_t* observed_sequence);

AURORIX_API int32_t aurorix_operation_cancel_v1(AurorixOperationHandle* operation);
AURORIX_API int32_t aurorix_operation_release_v1(AurorixOperationHandle* operation);
AURORIX_API int32_t aurorix_subscription_cancel_v1(AurorixSubscriptionHandle* subscription);
AURORIX_API int32_t aurorix_subscription_release_v1(AurorixSubscriptionHandle* subscription);
AURORIX_API void aurorix_buffer_free_v1(AurorixBufferV1 buffer);
AURORIX_API int32_t aurorix_client_shutdown_v1(AurorixClientHandle* client);
AURORIX_API int32_t aurorix_client_release_v1(AurorixClientHandle* client);

#ifdef __cplusplus
}
#endif

#endif
