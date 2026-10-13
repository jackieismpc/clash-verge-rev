#ifndef CLASH_VERGE_NATIVE_CORE_H
#define CLASH_VERGE_NATIVE_CORE_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct CvnHandle CvnHandle;
/* All strings are UTF-8 JSON, schema version 1. No callback crosses the ABI.
 * Config: {version:1,dataDir:absolute,mihomoPath?:absolute,mixedPort?:u16}.
 * Creation and destruction perform I/O: call on a background executor.
 * Request: {version:1,id:string,command:string,args:object,
 *           coreGeneration?:u64,revision?:u64}.
 * Envelope: {version:1,id:string,kind:result|state|stream,coreGeneration:u64,
 *            revision:u64,ok:bool,data?:any,error?:{code:string,message:string}}.
 * A handle must not be accessed concurrently with destroy. Other calls may be
 * concurrent. JSON returned here is owned by the caller; free exactly once.
 * Submit: 0=accepted, 1=invalid request, 2=queue full/duplicate ID, 3=stopped.
 * Stop requests shutdown without blocking. Destroy waits for owned cleanup.
 * A result is emitted exactly once for each accepted request. Cancellation
 * before commit is an error; after commit the authoritative result wins.
 */
CvnHandle *cvn_create(const char *config_json);
char *cvn_last_error(void);
int32_t cvn_submit(CvnHandle *handle, const char *request_json);
int32_t cvn_cancel(CvnHandle *handle, const char *request_id);
char *cvn_snapshot(CvnHandle *handle);
char *cvn_next_event(CvnHandle *handle);
void cvn_free_string(char *string);
void cvn_stop(CvnHandle *handle);
void cvn_destroy(CvnHandle *handle);
uint32_t cvn_abi_version(void);
#ifdef __cplusplus
}
#endif
#endif
