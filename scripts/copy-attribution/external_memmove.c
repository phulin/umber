#include <stddef.h>
#include <string.h>

__attribute__((noinline)) void *external_memmove(
    void *destination, const void *source, size_t size) {
    void *result = memmove(destination, source, size);
    __asm__ volatile("" : "+r"(result));
    return result;
}
