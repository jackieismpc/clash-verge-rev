#include "clash_verge_native_core.h"
#include <dlfcn.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>
static void *runtime;
static pthread_once_t initialized = PTHREAD_ONCE_INIT;
static void initialize(void) {
    const char *path = getenv("CVN_REAL_LIBRARY_PATH");
    if (path) runtime = dlopen(path, RTLD_NOW | RTLD_LOCAL);
}
static void *symbol(const char *name) { pthread_once(&initialized, initialize); return runtime ? dlsym(runtime, name) : NULL; }
static CvnBuffer failed(void) { return (CvnBuffer){NULL, 0, 0, 5}; }
uint32_t cvn_abi_version(void) { CvnVersionFn call = (CvnVersionFn)symbol("cvn_abi_version"); return call ? call() : 0; }
CvnHandle *cvn_create(const uint8_t *data, uint64_t length) { CvnCreateFn call = (CvnCreateFn)symbol("cvn_create"); return call ? call(data, length) : NULL; }
CvnBuffer cvn_last_error(void) { CvnErrorFn call = (CvnErrorFn)symbol("cvn_last_error"); return call ? call() : failed(); }
int32_t cvn_submit(CvnHandle *handle, const uint8_t *data, uint64_t length) { CvnInputFn call = (CvnInputFn)symbol("cvn_submit"); return call ? call(handle, data, length) : 5; }
int32_t cvn_cancel(CvnHandle *handle, const uint8_t *data, uint64_t length) { CvnInputFn call = (CvnInputFn)symbol("cvn_cancel"); return call ? call(handle, data, length) : 5; }
CvnBuffer cvn_snapshot(CvnHandle *handle) { CvnReadFn call = (CvnReadFn)symbol("cvn_snapshot"); return call ? call(handle) : failed(); }
int32_t cvn_free_buffer(CvnBuffer buffer) { CvnReleaseFn call = (CvnReleaseFn)symbol("cvn_free_buffer"); return call ? call(buffer) : 5; }
int32_t cvn_stop(CvnHandle *handle) { CvnCloseFn call = (CvnCloseFn)symbol("cvn_stop"); return call ? call(handle) : 5; }
int32_t cvn_destroy(CvnHandle *handle) { CvnCloseFn call = (CvnCloseFn)symbol("cvn_destroy"); return call ? call(handle) : 5; }
CvnBuffer cvn_next_event(CvnHandle *handle) {
    CvnReadFn call = (CvnReadFn)symbol("cvn_next_event");
    if (!call) return failed();
    const char marker[] = "\"kind\":\"state\"";
    for (;;) {
        CvnBuffer buffer = call(handle);
        if (buffer.status != 0) return buffer;
        int discard = 0;
        for (uint64_t i = 0; i + sizeof(marker) - 1 <= buffer.length; ++i) {
            if (!memcmp(buffer.data + i, marker, sizeof(marker) - 1)) { discard = 1; break; }
        }
        if (!discard) return buffer;
        cvn_free_buffer(buffer);
    }
}
