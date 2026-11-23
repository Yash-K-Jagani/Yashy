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

/* The last socket failure, per thread. Codes are the Yashy-facing values:
 * 1 refused, 2 reset, 3 timeout, 4 cancelled, 5 DNS, 6 address in use,
 * 7 other. A successful call clears it. */
#if defined(_WIN32)
#define YASHY_NET_THREAD_LOCAL __declspec(thread)
#else
#define YASHY_NET_THREAD_LOCAL __thread
#endif
static YASHY_NET_THREAD_LOCAL int yashy_net_error = 0;
static void yashy_net_note_fd(int64_t fd, unsigned char cause);

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
#include <arpa/inet.h>
#include <netdb.h>
#include <pthread.h>
#include <sys/socket.h>
#include <sys/time.h>
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

int yashy_net_last_error(void) {
    int value = yashy_net_error;
    yashy_net_error = 0;
    return value;
}

static int yashy_net_os_error(void) {
#if defined(_WIN32)
    return (int)WSAGetLastError();
#else
    return errno;
#endif
}

static int yashy_net_error_code(int os_error) {
#if defined(_WIN32)
    switch (os_error) {
        case WSAECONNREFUSED: return 1;
        case WSAECONNRESET: return 2;
        case WSAETIMEDOUT: return 3;
        case WSAEINTR: return 4;
        case WSAHOST_NOT_FOUND:
        case WSANO_DATA: return 5;
        case WSAEADDRINUSE: return 6;
        default: return 7;
    }
#else
    switch (os_error) {
        case ECONNREFUSED: return 1;
        case ECONNRESET: return 2;
        case ETIMEDOUT:
        case EAGAIN: return 3;
        case EINTR: return 4;
        case EAI_NONAME:
        case EAI_AGAIN:
        case EAI_FAIL: return 5;
        case EADDRINUSE: return 6;
        default: return 7;
    }
#endif
}

static void yashy_net_note_last_error(void) {
    yashy_net_error = yashy_net_error_code(yashy_net_os_error());
}

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
    int status = getaddrinfo(host, service, &hints, out);
    if (status != 0) {
        yashy_net_error = yashy_net_error_code(status);
        return -1;
    }
    yashy_net_error = 0;
    return 0;
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
            yashy_net_note_last_error();
            yashy_close_socket(handle);
            continue;
        }
        freeaddrinfo(addresses);
        yashy_net_error = 0;
        int64_t listen_fd = yashy_socket_to_i64(handle);
        yashy_net_note_fd(listen_fd, 0);
        return listen_fd;
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
            yashy_net_error = 0;
            int64_t accepted_fd = yashy_socket_to_i64(accepted);
            yashy_net_note_fd(accepted_fd, 0);
            return accepted_fd;
        }
        int os_error = yashy_net_os_error();
#if defined(_WIN32)
        if (os_error == WSAEINTR) {
            continue;
        }
#else
        if (os_error == EINTR) {
            continue;
        }
#endif
        yashy_net_error = yashy_net_error_code(os_error);
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
            yashy_net_error = 0;
            int64_t connected_fd = yashy_socket_to_i64(handle);
            yashy_net_note_fd(connected_fd, 0);
            return connected_fd;
        }
        yashy_net_note_last_error();
        yashy_close_socket(handle);
    }
    freeaddrinfo(addresses);
    return -1;
}

/* Absolute deadlines, distinct from the per-call SO_*TIMEO settings. A
 * deadline is an instant in Unix milliseconds; a blocking read/write that
 * starts after it is refused with the Timeout error. */
#define YASHY_NET_DEADLINES 256
static struct {
    int64_t fd;
    int64_t at_ms;
} yashy_net_deadlines[YASHY_NET_DEADLINES];

/* Per-descriptor lifetime state. A cancelled or closed handle keeps its
 * cause so a later read reports Cancelled or Closed instead of a generic
 * socket failure. Descriptors are small on every supported platform; the
 * array is a fixed size so no allocation happens on the socket path. */
#define YASHY_NET_FD_STATE 4096
static unsigned char yashy_net_fd_state[YASHY_NET_FD_STATE]; /* 4 cancelled, 8 closed */

static void yashy_net_note_fd(int64_t fd, unsigned char cause) {
    if (fd >= 0 && fd < YASHY_NET_FD_STATE) yashy_net_fd_state[fd] = cause;
}

static int yashy_net_fd_cause(int64_t fd) {
    return fd >= 0 && fd < YASHY_NET_FD_STATE ? yashy_net_fd_state[fd] : 0;
}

static int yashy_net_expired(int64_t fd) {
    for (int i = 0; i < YASHY_NET_DEADLINES; i++) {
        if (yashy_net_deadlines[i].fd == fd && yashy_net_deadlines[i].at_ms > 0) {
            if (yashy_now_ms() >= yashy_net_deadlines[i].at_ms) {
                yashy_net_error = 3; /* Timeout */
                return 1;
            }
            return 0;
        }
    }
    return 0;
}

int64_t yashy_net_set_deadline(int64_t fd, int64_t at_ms) {
    if (fd < 0 || at_ms < 0) return -1;
    for (int i = 0; i < YASHY_NET_DEADLINES; i++) {
        if (yashy_net_deadlines[i].fd == fd || yashy_net_deadlines[i].fd == 0) {
            yashy_net_deadlines[i].fd = fd;
            yashy_net_deadlines[i].at_ms = at_ms;
            return 0;
        }
    }
    return -1;
}

int64_t yashy_tcp_read(int64_t fd, uint8_t *buf, int64_t len) {
    yashy_net_socket socket = yashy_socket_from_i64(fd);
    int cause = yashy_net_fd_cause(fd);
    if (cause != 0) {
        yashy_net_error = cause;
        return -1;
    }
    if (socket == yashy_invalid_socket() || len < 0 || len > INT_MAX ||
        (len > 0 && buf == NULL)) {
        yashy_net_error = 7;
        return -1;
    }
    if (len == 0) {
        return 0;
    }
    if (yashy_net_expired(fd)) {
        return -1;
    }
    for (;;) {
        int received = recv(socket, (char *)buf, (size_t)len, 0);
        if (received >= 0) {
            yashy_net_error = 0;
            return (int64_t)received;
        }
        int os_error = yashy_net_os_error();
#if defined(_WIN32)
        if (os_error == WSAEINTR) {
            continue;
        }
#else
        if (os_error == EINTR) {
            continue;
        }
#endif
        yashy_net_error = yashy_net_error_code(os_error);
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
    int cause = yashy_net_fd_cause(fd);
    if (cause != 0) {
        yashy_net_error = cause;
        return -1;
    }
    if (socket == yashy_invalid_socket() || len < 0 || len > INT_MAX ||
        (len > 0 && buf == NULL)) {
        yashy_net_error = 7;
        return -1;
    }
    if (yashy_net_expired(fd)) {
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
        int os_error = yashy_net_os_error();
#if defined(_WIN32)
        if (os_error == WSAEINTR) {
            continue;
        }
#else
        if (os_error == EINTR) {
            continue;
        }
#endif
        yashy_net_error = yashy_net_error_code(os_error);
        return -1;
    }
    return offset;
}

void yashy_tcp_close(int64_t fd) {
    yashy_net_socket socket = yashy_socket_from_i64(fd);
    yashy_net_note_fd(fd, 8); /* Closed */
    yashy_close_socket(socket);
}

void yashy_tcp_cancel(int64_t fd) {
    yashy_net_socket socket = yashy_socket_from_i64(fd);
    yashy_net_note_fd(fd, 4); /* Cancelled */
    yashy_close_socket(socket);
}

/* ---- UDP ---------------------------------------------------------------- */

int64_t yashy_udp_open(const char *host, int64_t port, int passive) {
    struct addrinfo *addresses = NULL;
    struct addrinfo *address;
    if (yashy_socket_ready() != 0 || host == NULL || port < 0 || port > 65535) {
        yashy_net_error = 7;
        return -1;
    }
    char service[16];
    if (snprintf(service, sizeof(service), "%lld", (long long)port) >= (int)sizeof(service)) {
        yashy_net_error = 7;
        return -1;
    }
    struct addrinfo hints;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_DGRAM;
    hints.ai_protocol = IPPROTO_UDP;
    hints.ai_flags = passive ? AI_PASSIVE : 0;
    int status = getaddrinfo(host, service, &hints, &addresses);
    if (status != 0) {
        yashy_net_error = yashy_net_error_code(status);
        return -1;
    }
    for (address = addresses; address != NULL; address = address->ai_next) {
        yashy_net_socket handle = socket(address->ai_family, SOCK_DGRAM, IPPROTO_UDP);
        if (handle == yashy_invalid_socket()) {
            yashy_net_note_last_error();
            continue;
        }
        if (passive) {
            if (bind(handle, address->ai_addr, address->ai_addrlen) != 0) {
                yashy_net_note_last_error();
                yashy_close_socket(handle);
                continue;
            }
        } else if (connect(handle, address->ai_addr, address->ai_addrlen) != 0) {
            yashy_net_note_last_error();
            yashy_close_socket(handle);
            continue;
        }
        freeaddrinfo(addresses);
        yashy_net_error = 0;
        int64_t udp_fd = yashy_socket_to_i64(handle);
        yashy_net_note_fd(udp_fd, 0);
        return udp_fd;
    }
    freeaddrinfo(addresses);
    return -1;
}

int64_t yashy_udp_send(int64_t fd, const uint8_t *buf, int64_t len) {
    yashy_net_socket socket = yashy_socket_from_i64(fd);
    int cause = yashy_net_fd_cause(fd);
    if (cause != 0) {
        yashy_net_error = cause;
        return -1;
    }
    if (socket == yashy_invalid_socket() || len < 0 || len > INT_MAX ||
        (len > 0 && buf == NULL)) {
        yashy_net_error = 7;
        return -1;
    }
    if (yashy_net_expired(fd)) return -1;
    int sent = sendto(socket, (const char *)buf, (size_t)len, 0, NULL, 0);
    if (sent < 0) {
        yashy_net_note_last_error();
        return -1;
    }
    yashy_net_error = 0;
    return (int64_t)sent;
}

yashy_str *yashy_udp_recv_str(int64_t fd, int64_t max) {
    yashy_net_socket socket = yashy_socket_from_i64(fd);
    uint8_t error = 2;
    int cause = yashy_net_fd_cause(fd);
    if (cause != 0) {
        yashy_net_error = cause;
        return yashy_str_from_bytes(&error, 1);
    }
    if (socket == yashy_invalid_socket() || max < 0 || max > INT_MAX) {
        yashy_net_error = 7;
        return yashy_str_from_bytes(&error, 1);
    }
    if (yashy_net_expired(fd)) return yashy_str_from_bytes(&error, 1);
    uint8_t *buffer = (uint8_t *)malloc(max > 0 ? (size_t)max : 1);
    if (buffer == NULL) return yashy_str_from_bytes(&error, 1);
    int received = recvfrom(socket, (char *)buffer, (size_t)max, 0, NULL, NULL);
    if (received < 0) {
        yashy_net_note_last_error();
        free(buffer);
        return yashy_str_from_bytes(&error, 1);
    }
    yashy_net_error = 0;
    uint8_t *encoded = (uint8_t *)malloc((size_t)received + 1);
    if (encoded == NULL) {
        free(buffer);
        return yashy_str_from_bytes(&error, 1);
    }
    encoded[0] = 1;
    memcpy(encoded + 1, buffer, (size_t)received);
    yashy_str *result = yashy_str_from_bytes(encoded, received + 1);
    free(encoded);
    free(buffer);
    return result;
}

/* ---- DNS ---------------------------------------------------------------- */

yashy_str *yashy_dns_resolve(const char *host, int64_t port) {
    struct addrinfo *addresses = NULL;
    struct addrinfo *address;
    if (host == NULL || port < 0 || port > 65535 || yashy_socket_ready() != 0) {
        yashy_net_error = 5;
        return yashy_str_from_bytes((const uint8_t *)"", 0);
    }
    char service[16];
    if (snprintf(service, sizeof(service), "%lld", (long long)port) >= (int)sizeof(service)) {
        yashy_net_error = 5;
        return yashy_str_from_bytes((const uint8_t *)"", 0);
    }
    struct addrinfo hints;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;
    int status = getaddrinfo(host, service, &hints, &addresses);
    if (status != 0) {
        yashy_net_error = yashy_net_error_code(status);
        return yashy_str_from_bytes((const uint8_t *)"", 0);
    }
    size_t capacity = 128;
    size_t used = 0;
    char *out = (char *)malloc(capacity);
    if (out == NULL) {
        freeaddrinfo(addresses);
        return yashy_str_from_bytes((const uint8_t *)"", 0);
    }
    out[0] = '\0';
    for (address = addresses; address != NULL; address = address->ai_next) {
        char text[INET6_ADDRSTRLEN];
        const void *source = NULL;
        if (address->ai_family == AF_INET) {
            source = &((struct sockaddr_in *)address->ai_addr)->sin_addr;
        } else if (address->ai_family == AF_INET6) {
            source = &((struct sockaddr_in6 *)address->ai_addr)->sin6_addr;
        } else {
            continue;
        }
        if (inet_ntop(address->ai_family, source, text, sizeof(text)) == NULL) continue;
        size_t length = strlen(text);
        if (used + length + 2 > capacity) {
            capacity = (used + length + 2) * 2;
            char *grown = (char *)realloc(out, capacity);
            if (grown == NULL) break;
            out = grown;
        }
        if (used > 0) out[used++] = '\n';
        memcpy(out + used, text, length);
        used += length;
        out[used] = '\0';
    }
    freeaddrinfo(addresses);
    yashy_net_error = 0;
    yashy_str *result = yashy_str_from_bytes((const uint8_t *)out, (long long)used);
    free(out);
    return result;
}

int64_t yashy_tcp_set_timeout(int64_t fd, int64_t millis) {
    yashy_net_socket socket = yashy_socket_from_i64(fd);
    if (socket == yashy_invalid_socket() || millis < 0 || millis > INT_MAX) {
        return -1;
    }
#if defined(_WIN32)
    DWORD timeout = (DWORD)millis;
    if (setsockopt(socket, SOL_SOCKET, SO_RCVTIMEO, (const char *)&timeout,
                   (int)sizeof(timeout)) != 0 ||
        setsockopt(socket, SOL_SOCKET, SO_SNDTIMEO, (const char *)&timeout,
                   (int)sizeof(timeout)) != 0) {
        return -1;
    }
#else
    struct timeval timeout;
    timeout.tv_sec = (time_t)(millis / 1000);
    timeout.tv_usec = (suseconds_t)((millis % 1000) * 1000);
    if (setsockopt(socket, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout)) != 0 ||
        setsockopt(socket, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout)) != 0) {
        return -1;
    }
#endif
    return 0;
}
