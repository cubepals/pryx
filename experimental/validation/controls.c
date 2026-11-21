/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* Deliberately incorrect inputs to the symbolic checker. Neither is executed natively. */
#include <stddef.h>

static unsigned int reference(const unsigned char *data, size_t length) {
    for (size_t index = 0; index < length; ++index) {
        if (data[index] != 0) return 0;
    }
    return 1;
}

unsigned int control_wrong_single_byte(const unsigned char *data, size_t length) {
    return length == 1 || reference(data, length);
}

unsigned int control_extra_load(const unsigned char *data, size_t length) {
    if (length != 0) {
        /* Retain a machine-code read just beyond the declared slice; ignore its value. */
        volatile unsigned char sink = data[length];
        (void)sink;
    }
    return reference(data, length);
}

unsigned int control_discarded_load(const unsigned char *data, size_t length) {
    if (length != 0) {
#if defined(__aarch64__)
        /* Exercise the architectural read that VEX omits when its result targets WZR. */
        __asm__ volatile("ldrb wzr, [%0, %1]" : : "r"(data), "r"(length) : "memory");
#else
        (void)((const volatile unsigned char *)data)[length];
#endif
    }
    return reference(data, length);
}
