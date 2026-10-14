#ifndef CLASH_VERGE_NATIVE_CORE_H
#define CLASH_VERGE_NATIVE_CORE_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct CvnHandle CvnHandle;
typedef struct { const uint8_t *data; uint64_t length; uint64_t allocation_id; int32_t status; } CvnBuffer;
/* ABI 2, UTF-8 JSON schema 1. Input spans must be readable for their exact length;
 * no NUL terminator is needed. Callers never dereference handles or modify buffers.
 * Status: 0=success, 1=invalid, 2=capacity/closing, 3=stopped, 4=no event, 5=internal.
 * Config/request/envelope JSON schemas remain version 1. Concurrent handle calls
 * are safe. Stop cancels; destroy initiates cleanup and returns 2 until cleanup is
 * joined, then 0. Do not unload the library until all handles return destroy=0 and
 * all buffers are freed. Close/free are idempotent. Data stays valid until freed.
 * Input: config <=64 KiB, request <=64 MiB, request ID <=256 bytes.
 * Returned buffers carry exact lengths and allocation IDs, bounded to 128 MiB in
 * total and 64 outstanding allocations. last_error is thread-local UTF-8 text.
 */
CvnHandle *cvn_create(const uint8_t *, uint64_t);
CvnBuffer cvn_last_error(void);
int32_t cvn_submit(CvnHandle *, const uint8_t *, uint64_t);
int32_t cvn_cancel(CvnHandle *, const uint8_t *, uint64_t);
CvnBuffer cvn_snapshot(CvnHandle *);
CvnBuffer cvn_next_event(CvnHandle *);
int32_t cvn_free_buffer(CvnBuffer);
int32_t cvn_stop(CvnHandle *);
int32_t cvn_destroy(CvnHandle *);
uint32_t cvn_abi_version(void);
typedef CvnHandle *(*CvnCreateFn)(const uint8_t *, uint64_t);
typedef CvnBuffer (*CvnReadFn)(CvnHandle *);
typedef CvnBuffer (*CvnErrorFn)(void);
typedef int32_t (*CvnInputFn)(CvnHandle *, const uint8_t *, uint64_t);
typedef int32_t (*CvnReleaseFn)(CvnBuffer);
typedef int32_t (*CvnCloseFn)(CvnHandle *);
typedef uint32_t (*CvnVersionFn)(void);
#ifdef __cplusplus
}
#endif
#endif
