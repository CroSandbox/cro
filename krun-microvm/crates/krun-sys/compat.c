// Weak compatibility fallback implementations for libkrun FFI functions
// that may not be present in older versions of libkrun.dylib (e.g. Homebrew 1.19.4).

#include <stddef.h>
#include <stdbool.h>

extern int krun_add_disk3(
    unsigned int ctx_id,
    const char *block_id,
    const char *disk_path,
    unsigned int disk_format,
    bool read_only,
    bool direct_io,
    unsigned int sync_mode
);

extern int krun_add_virtiofs3(
    unsigned int ctx_id,
    const char *tag,
    const char *path,
    unsigned long long shm_size,
    bool read_only
);

__attribute__((weak))
int krun_add_disk4(
    unsigned int ctx_id,
    const char *block_id,
    const char *disk_path,
    unsigned int disk_format,
    bool read_only,
    bool direct_io,
    unsigned int sync_mode,
    bool parallel_reads,
    size_t read_pool_size
) {
    (void)parallel_reads;
    (void)read_pool_size;
    return krun_add_disk3(ctx_id, block_id, disk_path, disk_format, read_only, direct_io, sync_mode);
}

__attribute__((weak))
int krun_add_virtiofs4(
    unsigned int ctx_id,
    const char *tag,
    const char *path,
    unsigned long long shm_size,
    bool read_only,
    unsigned int cache_policy,
    unsigned int permission_semantics
) {
    (void)cache_policy;
    (void)permission_semantics;
    return krun_add_virtiofs3(ctx_id, tag, path, shm_size, read_only);
}

__attribute__((weak))
int krun_set_balloon_free_page_reporting_mode(unsigned int ctx_id, unsigned int mode) {
    (void)ctx_id;
    (void)mode;
    return 0;
}

__attribute__((weak))
int krun_has_feature(unsigned long long feature) {
    (void)feature;
    return 1;
}

__attribute__((weak))
int krun_get_max_vcpus(void) {
    return 32;
}

