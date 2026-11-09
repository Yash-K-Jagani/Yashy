/* TCP sockets for the `net` package.
 *
 * This translation unit is intentionally small: it adapts Winsock on Windows
 * and POSIX sockets elsewhere to one value-oriented contract. Every failure
 * is returned as a value (`-1`) rather than terminating the process; `read`
 * answers `0` only for a clean EOF after the peer closed its side.
 *
 * The platform is selected once at file scope. Winsock starts once through
 * `INIT_ONCE`; POSIX has no separate startup. Both are guarded by
 * `pthread_once`/`INIT_ONCE` semantics so the first socket operation, from
 * any thread, performs the one-time work exactly once.
 */
#include "yashy_runtime.h"

#include <errno.h>
#include <limits.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(_WIN32)
#ifndef _WIN32_WINNT
#define _WIN32_WINNT 0x0600
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#else
#include <netdb.h>
#include <pthread.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <unistd.h>
#endif

#if defined(_WIN32)
typedef SOCKET yashy_net_socket;
#else
typedef int yashy_net_socket;
#endif

#if defined(_WIN32)
static BOOL CALLBACK yashy_wsa_startup(PINIT_ONCE once, PVOID parameter, PVOID *context) {
    WSADATA data;
    (void)once;
    (void)parameter;
    (void)context;
    return WSAStartup(MAKEWORD(2, 2), &data) == 0;
}
static INIT_ONCE yashy_startup_once = INIT_ONCE_STATIC_INIT;
static int yashy_socket_ready(void) {
    return InitOnceExecuteOnce(&yashy_startup_once, yashy_wsa_startup, NULL, NULL) ? 0 : -1;
}
#else
static pthread_once_t yashy_startup_once = PTHREAD_ONCE_INIT;
static void yashy_posix_startup(void) {}
static int yashy_socket_ready(void) {
    return pthread_once(&yashy_startup_once, yashy_posix_startup) == 0 ? 0 : -1;
}
#endif

static yashy_net_socket yashy_invalid_socket(void) {
#if defined(_WIN32)
    return INVALID_SOCKET;
#else
    return (yashy_net_socket)-1;
#endif
}

static yashy_net_socket yashy_socket_from_i64(int64_t fd) {
    if (fd < 0) {
        return yashy_invalid_socket();
    }
    return (yashy_net_socket)fd;
}

static int64_t yashy_socket_to_i64(yashy_net_socket socket) {
    if (socket == yashy_invalid_socket()) {
        return -1;
    }
    return (int64_t)socket;
}

static void yashy_close_socket(yashy_net_socket socket) {
    if (socket == yashy_invalid_socket()) {
        return;
    }
#if defined(_WIN32)
    (void)closesocket(socket);
#else
    (void)close((int)socket);
#endif
}

static int yashy_resolve(const char *host, int64_t port, int passive, struct addrinfo **out) {
    struct addrinfo hints;
    char service[16];

    if (host == NULL || port < 0 || port > 65535) {
        return -1;
    }
    if (snprintf(service, sizeof(service), "%lld", (long long)port) >= (int)sizeof(service)) {
        return -1;
    }
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;
    hints.ai_flags = passive ? AI_PASSIVE : 0;
    return getaddrinfo(host, service, &hints, out) == 0 ? 0 : -1;
}

int64_t yashy_tcp_listen(const char *host, int64_t port) {
    struct addrinfo *addresses = NULL;
    struct addrinfo *address;

    if (yashy_socket_ready() != 0 || yashy_resolve(host, port, 1, &addresses) != 0) {
        return -1;
    }
    for (address = addresses; address != NULL; address = address->ai_next) {
        yashy_net_socket handle =
            socket(address->ai_family, address->ai_socktype, address->ai_protocol);
        int enabled = 1;
        if (handle == yashy_invalid_socket()) {
            continue;
        }
        (void)setsockopt(handle, SOL_SOCKET, SO_REUSEADDR, (const char *)&enabled,
                         (int)sizeof(enabled));
        if (bind(handle, address->ai_addr, address->ai_addrlen) != 0 ||
            listen(handle, 128) != 0) {
            yashy_close_socket(handle);
            continue;
        }
        freeaddrinfo(addresses);
        return yashy_socket_to_i64(handle);
    }
    freeaddrinfo(addresses);
    return -1;
}

int64_t yashy_tcp_accept(int64_t listener) {
    yashy_net_socket listener_socket = yashy_socket_from_i64(listener);
    if (listener_socket == yashy_invalid_socket()) {
        return -1;
    }
    for (;;) {
        yashy_net_socket accepted = accept(listener_socket, NULL, NULL);
        if (accepted != yashy_invalid_socket()) {
            return yashy_socket_to_i64(accepted);
        }
#if defined(_WIN32)
        if (WSAGetLastError() == WSAEINTR) {
            continue;
        }
#else
        if (errno == EINTR) {
            continue;
        }
#endif
        return -1;
    }
}

int64_t yashy_tcp_connect(const char *host, int64_t port) {
    struct addrinfo *addresses = NULL;
    struct addrinfo *address;

    if (yashy_socket_ready() != 0 || yashy_resolve(host, port, 0, &addresses) != 0) {
        return -1;
    }
    for (address = addresses; address != NULL; address = address->ai_next) {
        yashy_net_socket handle =
            socket(address->ai_family, address->ai_socktype, address->ai_protocol);
        if (handle == yashy_invalid_socket()) {
            continue;
        }
        if (connect(handle, address->ai_addr, address->ai_addrlen) == 0) {
            freeaddrinfo(addresses);
            return yashy_socket_to_i64(handle);
        }
        yashy_close_socket(handle);
    }
    freeaddrinfo(addresses);
    return -1;
}

int64_t yashy_tcp_read(int64_t fd, uint8_t *buf, int64_t len) {
    yashy_net_socket socket = yashy_socket_from_i64(fd);
    if (socket == yashy_invalid_socket() || len < 0 || len > INT_MAX ||
        (len > 0 && buf == NULL)) {
        return -1;
    }
    if (len == 0) {
        return 0;
    }
    for (;;) {
        int received = recv(socket, (char *)buf, (size_t)len, 0);
        if (received >= 0) {
            return (int64_t)received;
        }
#if defined(_WIN32)
        if (WSAGetLastError() == WSAEINTR) {
            continue;
        }
#else
        if (errno == EINTR) {
            continue;
        }
#endif
        return -1;
    }
}

yashy_str *yashy_tcp_read_str(int64_t fd, int64_t max) {
    uint8_t *buffer;
    uint8_t *encoded;
    int64_t received;
    yashy_str *result;

    if (max < 0 || max > INT_MAX) {
        uint8_t error = 2;
        return yashy_str_from_bytes(&error, 1);
    }
    buffer = (uint8_t *)malloc(max > 0 ? (size_t)max : 1);
    if (buffer == NULL) {
        uint8_t error = 2;
        return yashy_str_from_bytes(&error, 1);
    }
    received = yashy_tcp_read(fd, buffer, max);
    if (received < 0) {
        uint8_t error = 2;
        free(buffer);
        return yashy_str_from_bytes(&error, 1);
    }
    if (received == 0) {
        uint8_t eof = 0;
        free(buffer);
        return yashy_str_from_bytes(&eof, 1);
    }
    encoded = (uint8_t *)malloc((size_t)received + 1);
    if (encoded == NULL) {
        uint8_t error = 2;
        free(buffer);
        return yashy_str_from_bytes(&error, 1);
    }
    encoded[0] = 1;
    memcpy(encoded + 1, buffer, (size_t)received);
    result = yashy_str_from_bytes(encoded, received + 1);
    free(encoded);
    free(buffer);
    return result;
}

int64_t yashy_tcp_write(int64_t fd, const uint8_t *buf, int64_t len) {
    yashy_net_socket socket = yashy_socket_from_i64(fd);
    int64_t offset = 0;
    if (socket == yashy_invalid_socket() || len < 0 || len > INT_MAX ||
        (len > 0 && buf == NULL)) {
        return -1;
    }
    while (offset < len) {
        size_t remaining = (size_t)(len - offset);
        int sent = send(socket, (const char *)(buf + offset),
#if defined(_WIN32)
                        (int)remaining,
#else
                        remaining,
#endif
                        0);
        if (sent > 0) {
            offset += (int64_t)sent;
            continue;
        }
#if defined(_WIN32)
        if (WSAGetLastError() == WSAEINTR) {
            continue;
        }
#else
        if (errno == EINTR) {
            continue;
        }
#endif
        return -1;
    }
    return offset;
}

void yashy_tcp_close(int64_t fd) {
    yashy_net_socket socket = yashy_socket_from_i64(fd);
    yashy_close_socket(socket);
}
