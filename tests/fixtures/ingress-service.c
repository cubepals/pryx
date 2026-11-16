/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/** A disposable real-network fixture: TCP replies only after EOF; UDP prefixes its peer
 * port. It runs as the ordinary workload user and never handles bootstrap credentials. */
#include <arpa/inet.h>
#include <errno.h>
#include <poll.h>
#include <signal.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

static int listener(int type, int port) {
    int fd = socket(AF_INET, type, 0);
    struct sockaddr_in address = {.sin_family = AF_INET, .sin_port = htons(port),
                                  .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
    int on = 1;
    if (fd < 0 || setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &on, sizeof(on)) < 0 ||
        bind(fd, (struct sockaddr *)&address, sizeof(address)) < 0)
        exit(1);
    if (type == SOCK_STREAM && listen(fd, 16) < 0) exit(1);
    return fd;
}

static void write_all(int fd, const void *bytes, size_t length) {
    const char *next = bytes;
    while (length) {
        ssize_t wrote = write(fd, next, length);
        if (wrote < 0 && errno == EINTR) continue;
        if (wrote <= 0) _exit(1);
        length -= wrote;
        next += wrote;
    }
}

static void reply(int fd) {
    char bytes[128 * 1024];
    size_t length = 0;
    for (;;) {
        ssize_t got = read(fd, bytes + length, sizeof(bytes) - length);
        if (got < 0 && errno == EINTR) continue;
        if (got < 0 || (got && length + got == sizeof(bytes))) _exit(1);
        if (!got) break;
        length += got;
    }
    usleep(100000);
    write_all(fd, "after-eof:", strlen("after-eof:"));
    write_all(fd, bytes, length);
    shutdown(fd, SHUT_WR);
    close(fd);
    _exit(0);
}

int main(void) {
    signal(SIGCHLD, SIG_IGN);
    signal(SIGPIPE, SIG_IGN);
    int tcp = listener(SOCK_STREAM, 8080);
    int udp = listener(SOCK_DGRAM, 8081);
    struct pollfd fds[] = {{.fd = tcp, .events = POLLIN}, {.fd = udp, .events = POLLIN}};
    for (;;) {
        if (poll(fds, 2, -1) < 0) {
            if (errno == EINTR) continue;
            return 1;
        }
        if (fds[0].revents & POLLIN) {
            int client = accept(tcp, NULL, NULL);
            if (client < 0) return 1;
            pid_t child = fork();
            if (child == 0) {
                close(tcp);
                close(udp);
                reply(client);
            }
            close(client);
            if (child < 0) return 1;
        }
        if (fds[1].revents & POLLIN) {
            unsigned char bytes[65507];
            struct sockaddr_in peer;
            socklen_t size = sizeof(peer);
            ssize_t length = recvfrom(udp, bytes + sizeof(peer.sin_port), sizeof(bytes) - sizeof(peer.sin_port), 0,
                                      (struct sockaddr *)&peer, &size);
            if (length < 0) return 1;
            memcpy(bytes, &peer.sin_port, sizeof(peer.sin_port));
            if (sendto(udp, bytes, length + sizeof(peer.sin_port), 0, (struct sockaddr *)&peer, size) < 0) return 1;
        }
    }
}
