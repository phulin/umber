#define _GNU_SOURCE

#include <dlfcn.h>
#include <elf.h>
#include <fcntl.h>
#include <link.h>
#include <stdatomic.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <unwind.h>

enum {
    API_COUNT = 2,
    APPLICATION_SEGMENT_CAPACITY = 8,
    BIN_CAPACITY = 32768,
    MAX_PROBES = 24,
    MAX_UNWIND_FRAMES = 48,
};

enum caller_class {
    CALLER_APPLICATION_DIRECT = 1,
    CALLER_APPLICATION_ANCESTOR = 2,
    CALLER_EXTERNAL = 3,
};

struct executable_segment {
    uintptr_t start;
    uintptr_t end;
};

struct caller_bin {
    _Atomic uintptr_t address;
    _Atomic unsigned int caller_class;
    _Atomic uint64_t calls;
    _Atomic uint64_t bytes;
};

struct unwind_search {
    uintptr_t application_address;
    unsigned int visited;
};

typedef void *(*copy_fn)(void *, const void *, size_t);

static copy_fn real_memcpy;
static copy_fn real_memmove;
static struct executable_segment application_segments[APPLICATION_SEGMENT_CAPACITY];
static size_t application_segment_count;
static uintptr_t application_base;
static struct caller_bin bins[API_COUNT][BIN_CAPACITY];
static _Atomic uint64_t total_calls[API_COUNT];
static _Atomic uint64_t total_bytes[API_COUNT];
static _Atomic uint64_t overflow_calls[API_COUNT];
static _Atomic uint64_t overflow_bytes[API_COUNT];
static _Atomic uint64_t collision_probes[API_COUNT];
static _Atomic unsigned int maximum_probe[API_COUNT];
static _Atomic uint64_t suppressed_probe_calls[API_COUNT];
static _Atomic int recording;
static _Thread_local int inside_probe;
static int output_fd = STDERR_FILENO;

static void *fallback_copy(void *destination, const void *source, size_t size, int overlap) {
    volatile unsigned char *to = destination;
    const volatile unsigned char *from = source;
    if (overlap && to > from && to < from + size) {
        for (size_t index = size; index != 0; --index) {
            to[index - 1] = from[index - 1];
        }
    } else {
        for (size_t index = 0; index < size; ++index) {
            to[index] = from[index];
        }
    }
    return destination;
}

static int application_image(struct dl_phdr_info *info, size_t size, void *data) {
    (void)size;
    (void)data;
    if (info->dlpi_name != NULL && info->dlpi_name[0] != '\0') {
        return 0;
    }
    application_base = (uintptr_t)info->dlpi_addr;
    for (ElfW(Half) index = 0; index < info->dlpi_phnum; ++index) {
        const ElfW(Phdr) *header = &info->dlpi_phdr[index];
        if (header->p_type != PT_LOAD || (header->p_flags & PF_X) == 0) {
            continue;
        }
        if (application_segment_count == APPLICATION_SEGMENT_CAPACITY) {
            break;
        }
        struct executable_segment *segment = &application_segments[application_segment_count++];
        segment->start = application_base + (uintptr_t)header->p_vaddr;
        segment->end = segment->start + (uintptr_t)header->p_memsz;
    }
    return 1;
}

static int is_application_address(uintptr_t address) {
    for (size_t index = 0; index < application_segment_count; ++index) {
        if (address >= application_segments[index].start &&
            address < application_segments[index].end) {
            return 1;
        }
    }
    return 0;
}

static _Unwind_Reason_Code find_application_ancestor(
    struct _Unwind_Context *context, void *argument) {
    struct unwind_search *search = argument;
    uintptr_t address = (uintptr_t)_Unwind_GetIP(context);
    ++search->visited;
    if (is_application_address(address)) {
        search->application_address = address;
        return _URC_END_OF_STACK;
    }
    if (search->visited >= MAX_UNWIND_FRAMES) {
        return _URC_END_OF_STACK;
    }
    return _URC_NO_REASON;
}

static uint64_t mix_address(uintptr_t address, unsigned int caller_class) {
    uint64_t value = (uint64_t)address ^ ((uint64_t)caller_class << 56);
    value ^= value >> 30;
    value *= UINT64_C(0xbf58476d1ce4e5b9);
    value ^= value >> 27;
    value *= UINT64_C(0x94d049bb133111eb);
    return value ^ (value >> 31);
}

static void update_maximum_probe(int api, unsigned int probe) {
    unsigned int current = atomic_load_explicit(&maximum_probe[api], memory_order_relaxed);
    while (current < probe &&
           !atomic_compare_exchange_weak_explicit(&maximum_probe[api], &current, probe,
                                                  memory_order_relaxed,
                                                  memory_order_relaxed)) {
    }
}

static void record_bin(
    int api, unsigned int caller_class, uintptr_t address, size_t size) {
    uint64_t hash = mix_address(address, caller_class);
    for (unsigned int probe = 0; probe < MAX_PROBES; ++probe) {
        size_t index = (size_t)((hash + probe) & (BIN_CAPACITY - 1));
        struct caller_bin *bin = &bins[api][index];
        uintptr_t observed = atomic_load_explicit(&bin->address, memory_order_acquire);
        if (observed == 0) {
            uintptr_t empty = 0;
            if (atomic_compare_exchange_strong_explicit(
                    &bin->address, &empty, address, memory_order_acq_rel,
                    memory_order_acquire)) {
                atomic_store_explicit(&bin->caller_class, caller_class, memory_order_release);
                observed = address;
            } else {
                observed = empty;
            }
        }
        if (observed == address &&
            atomic_load_explicit(&bin->caller_class, memory_order_acquire) == caller_class) {
            atomic_fetch_add_explicit(&bin->calls, 1, memory_order_relaxed);
            atomic_fetch_add_explicit(&bin->bytes, size, memory_order_relaxed);
            update_maximum_probe(api, probe);
            return;
        }
        atomic_fetch_add_explicit(&collision_probes[api], 1, memory_order_relaxed);
    }
    atomic_fetch_add_explicit(&overflow_calls[api], 1, memory_order_relaxed);
    atomic_fetch_add_explicit(&overflow_bytes[api], size, memory_order_relaxed);
}

static void record_copy(int api, uintptr_t direct_address, size_t size) {
    atomic_fetch_add_explicit(&total_calls[api], 1, memory_order_relaxed);
    atomic_fetch_add_explicit(&total_bytes[api], size, memory_order_relaxed);

    unsigned int caller_class = CALLER_EXTERNAL;
    uintptr_t address = direct_address;
    if (is_application_address(direct_address)) {
        caller_class = CALLER_APPLICATION_DIRECT;
        address -= application_base;
    } else {
        struct unwind_search search = {0, 0};
        _Unwind_Backtrace(find_application_ancestor, &search);
        if (search.application_address != 0) {
            caller_class = CALLER_APPLICATION_ANCESTOR;
            address = search.application_address - application_base;
        }
    }
    record_bin(api, caller_class, address, size);
}

static void resolve_copy_functions(void) {
    inside_probe = 1;
    real_memcpy = (copy_fn)dlsym(RTLD_NEXT, "memcpy");
    real_memmove = (copy_fn)dlsym(RTLD_NEXT, "memmove");
    inside_probe = 0;
}

__attribute__((constructor)) static void initialize_probe(void) {
    inside_probe = 1;
    dl_iterate_phdr(application_image, NULL);
    const char *output_path = getenv("UMBER_COPY_ATTRIBUTION_OUT");
    if (output_path != NULL && output_path[0] != '\0') {
        int opened = open(output_path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0600);
        if (opened >= 0) {
            output_fd = opened;
        }
    }
    resolve_copy_functions();
    inside_probe = 0;
    atomic_store_explicit(&recording, 1, memory_order_release);
}

void *memcpy(void *destination, const void *source, size_t size) {
    uintptr_t caller = (uintptr_t)__builtin_extract_return_addr(__builtin_return_address(0));
    int nested = inside_probe;
    if (nested) {
        atomic_fetch_add_explicit(&suppressed_probe_calls[0], 1, memory_order_relaxed);
    } else if (atomic_load_explicit(&recording, memory_order_acquire)) {
        inside_probe = 1;
        record_copy(0, caller, size);
        inside_probe = 0;
    }
    if (real_memcpy == NULL) {
        if (nested) {
            return fallback_copy(destination, source, size, 0);
        }
        resolve_copy_functions();
    }
    if (real_memcpy == NULL) {
        return fallback_copy(destination, source, size, 0);
    }
    return real_memcpy(destination, source, size);
}

void *memmove(void *destination, const void *source, size_t size) {
    uintptr_t caller = (uintptr_t)__builtin_extract_return_addr(__builtin_return_address(0));
    int nested = inside_probe;
    if (nested) {
        atomic_fetch_add_explicit(&suppressed_probe_calls[1], 1, memory_order_relaxed);
    } else if (atomic_load_explicit(&recording, memory_order_acquire)) {
        inside_probe = 1;
        record_copy(1, caller, size);
        inside_probe = 0;
    }
    if (real_memmove == NULL) {
        if (nested) {
            return fallback_copy(destination, source, size, 1);
        }
        resolve_copy_functions();
    }
    if (real_memmove == NULL) {
        return fallback_copy(destination, source, size, 1);
    }
    return real_memmove(destination, source, size);
}

static const char *caller_class_name(unsigned int caller_class) {
    switch (caller_class) {
    case CALLER_APPLICATION_DIRECT:
        return "application_direct";
    case CALLER_APPLICATION_ANCESTOR:
        return "application_ancestor";
    default:
        return "external_only";
    }
}

static void report_external_owner(uintptr_t address) {
    Dl_info information;
    if (dladdr((void *)address, &information) != 0 && information.dli_fbase != NULL) {
        const char *module = information.dli_fname == NULL ? "unknown" : information.dli_fname;
        uintptr_t offset = address - (uintptr_t)information.dli_fbase;
        dprintf(output_fd, " module=%s module_offset=0x%lx", module, (unsigned long)offset);
    } else {
        dprintf(output_fd, " module=unknown module_offset=0x0");
    }
}

__attribute__((destructor)) static void report_probe(void) {
    atomic_store_explicit(&recording, 0, memory_order_release);
    inside_probe = 1;
    dprintf(output_fd,
            "COPY_ATTRIBUTION schema=1 application_base=0x%lx bins=%u max_probes=%u "
            "max_unwind_frames=%u\n",
            (unsigned long)application_base, BIN_CAPACITY, MAX_PROBES, MAX_UNWIND_FRAMES);
    for (int api = 0; api < API_COUNT; ++api) {
        const char *name = api == 0 ? "memcpy" : "memmove";
        uint64_t binned_calls = 0;
        uint64_t binned_bytes = 0;
        for (size_t index = 0; index < BIN_CAPACITY; ++index) {
            struct caller_bin *bin = &bins[api][index];
            uint64_t calls = atomic_load_explicit(&bin->calls, memory_order_relaxed);
            if (calls == 0) {
                continue;
            }
            uint64_t bytes = atomic_load_explicit(&bin->bytes, memory_order_relaxed);
            uintptr_t address = atomic_load_explicit(&bin->address, memory_order_relaxed);
            unsigned int caller_class =
                atomic_load_explicit(&bin->caller_class, memory_order_relaxed);
            binned_calls += calls;
            binned_bytes += bytes;
            dprintf(output_fd,
                    "COPY_CALLER api=%s class=%s address=0x%lx calls=%lu bytes=%lu",
                    name, caller_class_name(caller_class), (unsigned long)address,
                    (unsigned long)calls, (unsigned long)bytes);
            if (caller_class == CALLER_EXTERNAL) {
                report_external_owner(address);
            }
            dprintf(output_fd, "\n");
        }
        uint64_t dropped_calls = atomic_load_explicit(&overflow_calls[api], memory_order_relaxed);
        uint64_t dropped_bytes = atomic_load_explicit(&overflow_bytes[api], memory_order_relaxed);
        if (dropped_calls != 0) {
            dprintf(output_fd,
                    "COPY_CALLER api=%s class=table_overflow address=0x0 calls=%lu bytes=%lu\n",
                    name, (unsigned long)dropped_calls, (unsigned long)dropped_bytes);
        }
        binned_calls += dropped_calls;
        binned_bytes += dropped_bytes;
        dprintf(output_fd,
                "COPY_TOTAL api=%s calls=%lu bytes=%lu caller_calls=%lu caller_bytes=%lu\n",
                name,
                (unsigned long)atomic_load_explicit(&total_calls[api], memory_order_relaxed),
                (unsigned long)atomic_load_explicit(&total_bytes[api], memory_order_relaxed),
                (unsigned long)binned_calls, (unsigned long)binned_bytes);
        dprintf(output_fd,
                "COPY_TABLE api=%s collision_probes=%lu maximum_probe=%u overflow_calls=%lu "
                "overflow_bytes=%lu probe_internal_calls=%lu\n",
                name,
                (unsigned long)atomic_load_explicit(&collision_probes[api],
                                                    memory_order_relaxed),
                atomic_load_explicit(&maximum_probe[api], memory_order_relaxed),
                (unsigned long)dropped_calls, (unsigned long)dropped_bytes,
                (unsigned long)atomic_load_explicit(&suppressed_probe_calls[api],
                                                    memory_order_relaxed));
    }
    if (output_fd != STDERR_FILENO) {
        close(output_fd);
    }
}
