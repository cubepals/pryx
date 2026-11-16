// SPDX-License-Identifier: FSL-1.1-ALv2
// Owned constructor marker and confined workload controls; never reads a private value.
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ptrace.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <sys/uio.h>
#include <unistd.h>

#ifdef PRYX_LOADER_PROGRAM
extern char **environ;
static int failed(const char *phase, int detail) {
    int reason = errno;
    fprintf(stderr, "OWNED_ENV_CHECK phase=%s errno=%d detail=%d\n", phase, reason, detail);
    return 1;
}
static int private_state(void) {
    pid_t parent = getppid();
    char path[128];
    for (size_t i = 0; i < 2; ++i) {
        snprintf(path, sizeof(path), "/proc/%ld/%s", (long)parent, i == 0 ? "environ" : "mem");
        int input = open(path, O_RDONLY | O_CLOEXEC);
        if (input >= 0) {
            close(input);
            return failed("parent-proc-readable", (int)i);
        }
        if (errno != EACCES && errno != EPERM)
            return failed("parent-proc-denial", (int)i);
    }
    errno = 0;
    if (ptrace(PTRACE_PEEKDATA, parent, NULL, NULL) != -1 || errno != EPERM)
        return failed("ptrace-denial", 0);
    char byte;
    struct iovec local = { &byte, sizeof(byte) };
    struct iovec remote = { NULL, sizeof(byte) };
    errno = 0;
    if (process_vm_readv(parent, &local, 1, &remote, 1, 0) != -1 || errno != EPERM)
        return failed("process-memory-denial", 0);
    errno = 0;
    if (syscall(SYS_pidfd_getfd, -1, -1, 0) != -1 || errno != EPERM)
        return failed("pidfd-transfer-denial", 0);
    struct rlimit limit;
    if (getrlimit(RLIMIT_NOFILE, &limit) != 0 || limit.rlim_cur == RLIM_INFINITY)
        return failed("descriptor-limit", 0);
    for (rlim_t number = STDERR_FILENO + 1; number < limit.rlim_cur; ++number) {
        errno = 0;
        if (fcntl((int)number, F_GETFD) != -1 || errno != EBADF)
            return failed("descriptor-inheritance", (int)number);
    }
    return 0;
}
int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "bootstrap-probe") == 0)
        puts("{}");
    if (argc > 2 && strcmp(argv[1], "--verify-workload") == 0) {
        const char *value = getenv("APP_VALUE");
        if (value == NULL || strcmp(value, "owned-value") != 0 ||
            getenv("PRYX_PROCESS_POLICY") != NULL || getenv("PRYX_PROCESS_LISTENERS") != NULL ||
            getenv("PRYX_PROCESS_NETWORK_OFF") != NULL)
            return failed("workload-values", 0);
        size_t prefix = strlen(argv[2]);
        for (char **item = environ; *item != NULL; ++item) {
            size_t key = strcspn(*item, "=");
            if (key >= prefix && strncmp(*item, argv[2], prefix) == 0)
                return failed("private-prefix-presence", 0);
        }
        return private_state();
    }
    return 0;
}
#else
__attribute__((constructor)) static void loaded(void) {
    const char *port = getenv("PRYX_TEST_LOADER_PORT");
    if (port != NULL) {
        int connection = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
        if (connection >= 0) {
            struct sockaddr_in address = { 0 };
            address.sin_family = AF_INET;
            address.sin_port = htons((unsigned short)strtoul(port, NULL, 10));
            address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
            if (connect(connection, (struct sockaddr *)&address, sizeof(address)) == 0) {
                const char value[] = "owned loader connection\n";
                (void)send(connection, value, sizeof(value) - 1, MSG_NOSIGNAL);
            }
            close(connection);
        }
    }
    const char *path = getenv("PRYX_TEST_LOADER_MARKER");
    if (path == NULL)
        return;
    int output = open(path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0600);
    if (output >= 0) {
        const char value[] = "owned loader executed\n";
        (void)write(output, value, sizeof(value) - 1);
        close(output);
    }
}
#endif
