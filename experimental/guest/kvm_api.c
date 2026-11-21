// SPDX-License-Identifier: FSL-1.1-ALv2
// Read ordinary-user KVM availability using the native Linux UAPI.
#include <asm/unistd.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/kvm.h>
#include <stdio.h>
#include <sys/ioctl.h>
#include <unistd.h>

int main(void) {
    int fd = open("/dev/kvm", O_RDWR | O_CLOEXEC);
    int open_error = fd < 0 ? errno : 0;
    int api = fd < 0 ? -1 : ioctl(fd, KVM_GET_API_VERSION, 0);
    int api_error = fd >= 0 && api < 0 ? errno : 0;
    if (fd >= 0) close(fd);
    printf("%d %d %d %d %ld\n", open_error, api, api_error, KVM_API_VERSION, (long)__NR_ppoll);
    return 0;
}
