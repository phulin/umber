#include <stddef.h>
#include <pthread.h>
#include <stdint.h>
#include <string.h>

__attribute__((noinline)) void *external_memmove(
    void *destination, const void *source, size_t size) {
    void *result = memmove(destination, source, size);
    __asm__ volatile("" : "+r"(result));
    return result;
}

static void *external_only_worker(void *argument) {
    unsigned char *bytes = argument;
    memmove(bytes + 1, bytes, 63);
    return NULL;
}

__attribute__((noinline)) unsigned char external_only_thread_gate(void) {
    unsigned char bytes[64] = {19};
    pthread_t thread;
    if (pthread_create(&thread, NULL, external_only_worker, bytes) != 0) {
        return 0;
    }
    if (pthread_join(thread, NULL) != 0) {
        return 0;
    }
    return bytes[1];
}
