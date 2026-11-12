/* Platform functions behind the breadth modules (phase 12, Task 10):
 * process control, filesystem metadata and listing, the environment,
 * entropy, a monotonic clock, and tty detection.
 *
 * Every function answers with a value, never by aborting: `0` on success
 * and `-1` on failure, following the tag convention of `runtime.c`. String
 * results are freshly allocated `yashy_str` values handed to the caller.
 * Results the OS does not guarantee (a directory's mtime granularity, the
 * iterator order) are documented in the Yashy modules rather than papered
 * over here.
 */

#include "yashy_runtime.h"

#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <sys/stat.h>

#if defined(_WIN32)
#ifndef _WIN32_WINNT
#define _WIN32_WINNT 0x0600
#endif
#include <io.h>
#include <windows.h>
/* `bcrypt.h` needs the Windows types, so it comes after `windows.h`. */
#include <bcrypt.h>
#else
#include <dirent.h>
#include <fcntl.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
#endif

/* ---- Monotonic clock: nanoseconds that only move forwards. ---- */

long long yashy_monotonic_ns(void) {
#if defined(_WIN32)
    LARGE_INTEGER frequency;
    LARGE_INTEGER counter;
    if (QueryPerformanceFrequency(&frequency) == 0 || frequency.QuadPart <= 0) {
        return -1;
    }
    if (QueryPerformanceCounter(&counter) == 0) {
        return -1;
    }
    /* Split before scaling: `counter * 1e9` overflows a 64-bit integer within
       minutes on a machine that has been up for a while. */
    LONGLONG seconds = counter.QuadPart / frequency.QuadPart;
    LONGLONG remainder = counter.QuadPart % frequency.QuadPart;
    return seconds * 1000000000LL +
           (remainder * 1000000000LL) / frequency.QuadPart;
#else
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
        return -1;
    }
    return (long long)ts.tv_sec * 1000000000LL + (long long)ts.tv_nsec;
#endif
}

/* ---- Platform identity, process id, wall clock, entropy numbers ---- */

/** 0 Windows, 1 Linux, 2 macOS, 3 anything else. */
int yashy_platform_id(void) {
#if defined(_WIN32)
    return 0;
#elif defined(__APPLE__)
    return 2;
#elif defined(__linux__)
    return 1;
#else
    return 3;
#endif
}

/** 0 x86_64, 1 aarch64, 2 anything else. */
int yashy_arch_id(void) {
#if defined(_M_X64) || defined(__x86_64__)
    return 0;
#elif defined(_M_ARM64) || defined(__aarch64__)
    return 1;
#else
    return 2;
#endif
}

long long yashy_pid(void) {
#if defined(_WIN32)
    return (long long)GetCurrentProcessId();
#else
    return (long long)getpid();
#endif
}

/** Nanoseconds since the Unix epoch, from the wall clock. */
long long yashy_now_unix_ns(void) {
#if defined(_WIN32)
    FILETIME ft;
    GetSystemTimeAsFileTime(&ft);
    ULARGE_INTEGER value;
    value.LowPart = ft.dwLowDateTime;
    value.HighPart = ft.dwHighDateTime;
    /* 100 ns ticks between 1601-01-01 and 1970-01-01. */
    const unsigned long long epoch = 116444736000000000ULL;
    return (long long)((value.QuadPart - epoch) * 100ULL);
#else
    struct timespec ts;
    if (clock_gettime(CLOCK_REALTIME, &ts) != 0) {
        return -1;
    }
    return (long long)ts.tv_sec * 1000000000LL + (long long)ts.tv_nsec;
#endif
}

/* ---- TTY detection (term). ---- */

int yashy_is_tty(int fd) {
#if defined(_WIN32)
    return _isatty(fd) ? 1 : 0;
#else
    return isatty(fd) ? 1 : 0;
#endif
}

/* ---- Environment (env): iterate `K=V`, set, and unset. ---- */

/** Sort key: the name before `=`, as `qsort` wants.
 *
 * A record already starts with its name, so `strcmp` is nearly the name order;
 * it differs when one name is a prefix of the other and the next byte is below
 * `=`, where `A1=...` would sort before `A=...`. Comparing up to the separator
 * makes the order the name order the module documents. For a directory entry
 * there is no `=`, so this is plain `strcmp`. */
static int yashy_compare_names(const void *left, const void *right) {
    const char *a = *(const char *const *)left;
    const char *b = *(const char *const *)right;
    while (*a != '\0' && *b != '\0' && *a != '=' && *b != '=') {
        if (*a != *b) {
            return (unsigned char)*a < (unsigned char)*b ? -1 : 1;
        }
        a++;
        b++;
    }
    int a_done = (*a == '\0' || *a == '=');
    int b_done = (*b == '\0' || *b == '=');
    if (a_done && b_done) {
        return 0;
    }
    return a_done ? -1 : 1;
}

/** Append a record to a growing name array. */
static int yashy_push_name(char ***names, size_t *count, size_t *capacity, const char *name) {
    if (*count == *capacity) {
        size_t grown = *capacity == 0 ? 32 : *capacity * 2;
        char **next = (char **)realloc(*names, grown * sizeof(char *));
        if (next == NULL) {
            return -1;
        }
        *names = next;
        *capacity = grown;
    }
    (*names)[(*count)++] = (char *)name;
    return 0;
}

/** Every entry as newline-separated `K=V` records, sorted by name and
 * trailing-newline free. */
yashy_str *yashy_env_join(void) {
    char **names = NULL;
    size_t count = 0;
    size_t capacity = 0;
#if defined(_WIN32)
    char *block = GetEnvironmentStringsA();
    if (block == NULL) {
        return yashy_str_from_bytes((const uint8_t *)"", 0);
    }
    char *cursor = block;
    for (;;) {
        size_t entry_len = strlen(cursor);
        if (entry_len == 0) {
            break;
        }
        char *copy = (char *)malloc(entry_len + 1);
        if (copy == NULL || yashy_push_name(&names, &count, &capacity, copy) != 0) {
            free(copy);
            break;
        }
        memcpy(copy, cursor, entry_len + 1);
        cursor += entry_len + 1;
    }
    FreeEnvironmentStringsA(block);
#else
    extern char **environ;
    if (environ == NULL) {
        return yashy_str_from_bytes((const uint8_t *)"", 0);
    }
    for (char **entry = environ; *entry != NULL; entry++) {
        size_t entry_len = strlen(*entry);
        char *copy = (char *)malloc(entry_len + 1);
        if (copy == NULL || yashy_push_name(&names, &count, &capacity, copy) != 0) {
            free(copy);
            break;
        }
        memcpy(copy, *entry, entry_len + 1);
    }
#endif
    if (count > 1) {
        qsort(names, count, sizeof(char *), yashy_compare_names);
    }
    size_t length = 0;
    for (size_t i = 0; i < count; i++) {
        length += strlen(names[i]) + (i + 1 < count ? 1 : 0);
    }
    char *out = (char *)malloc(length + 1);
    if (out == NULL) {
        for (size_t i = 0; i < count; i++) {
            free(names[i]);
        }
        free(names);
        return yashy_str_from_bytes((const uint8_t *)"", 0);
    }
    size_t at = 0;
    for (size_t i = 0; i < count; i++) {
        size_t entry_len = strlen(names[i]);
        memcpy(out + at, names[i], entry_len);
        at += entry_len;
        if (i + 1 < count) {
            out[at++] = '\n';
        }
        free(names[i]);
    }
    free(names);
    yashy_str *result = yashy_str_from_bytes((const uint8_t *)out, (long long)length);
    free(out);
    return result;
}

int yashy_env_set(const char *name, const char *value) {
    if (name == NULL || value == NULL || strchr(name, '=') != NULL) {
        return -1;
    }
#if defined(_WIN32)
    /* `_putenv_s`, not `SetEnvironmentVariableA`: the MSVC CRT keeps its own
       copy of the environment, and `getenv` (which `yashy_getenv` reads)
       does not see changes made through the Win32 API -- verified: the block
       shows the variable while `getenv` answers NULL. `_putenv_s` updates
       both, and an empty value removes the variable. */
    return _putenv_s(name, value);
#else
    return setenv(name, value, 1) == 0 ? 0 : -1;
#endif
}

int yashy_env_unset(const char *name) {
    if (name == NULL) {
        return -1;
    }
#if defined(_WIN32)
    /* `_putenv_s` with an empty value removes the variable (and, unlike
       `SetEnvironmentVariableA`, also from the CRT copy `getenv` reads). */
    return _putenv_s(name, "");
#else
    return unsetenv(name) == 0 ? 0 : -1;
#endif
}

/* ---- Filesystem (fs): newline-separated names. ---- */

/** Every entry name of the directory at `path`, sorted by name and
 * newline-separated (`NULL` when the directory cannot be opened). */
yashy_str *yashy_fs_listdir_join(const char *path) {
    if (path == NULL) {
        return NULL;
    }
    char **names = NULL;
    size_t count = 0;
    size_t capacity = 0;
    int opened = 0;
#if defined(_WIN32)
    WIN32_FIND_DATAA find;
    char pattern[MAX_PATH];
    size_t path_len = strlen(path);
    if (path_len + 3 >= sizeof(pattern)) {
        return NULL;
    }
    memcpy(pattern, path, path_len);
    pattern[path_len] = '\\';
    pattern[path_len + 1] = '*';
    pattern[path_len + 2] = '\0';
    HANDLE handle = FindFirstFileA(pattern, &find);
    if (handle == INVALID_HANDLE_VALUE) {
        return NULL;
    }
    opened = 1;
    do {
        const char *name = find.cFileName;
        if (strcmp(name, ".") == 0 || strcmp(name, "..") == 0) {
            continue;
        }
        size_t name_len = strlen(name);
        char *copy = (char *)malloc(name_len + 1);
        if (copy == NULL || yashy_push_name(&names, &count, &capacity, copy) != 0) {
            free(copy);
            break;
        }
        memcpy(copy, name, name_len + 1);
    } while (FindNextFileA(handle, &find));
    FindClose(handle);
#else
    DIR *dir = opendir(path);
    if (dir == NULL) {
        return NULL;
    }
    opened = 1;
    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) {
            continue;
        }
        size_t name_len = strlen(entry->d_name);
        char *copy = (char *)malloc(name_len + 1);
        if (copy == NULL || yashy_push_name(&names, &count, &capacity, copy) != 0) {
            free(copy);
            break;
        }
        memcpy(copy, entry->d_name, name_len + 1);
    }
    closedir(dir);
#endif
    if (!opened) {
        return NULL;
    }
    if (count > 1) {
        qsort(names, count, sizeof(char *), yashy_compare_names);
    }
    size_t length = 0;
    for (size_t i = 0; i < count; i++) {
        length += strlen(names[i]) + (i + 1 < count ? 1 : 0);
    }
    char *out = (char *)malloc(length + 1);
    if (out == NULL) {
        for (size_t i = 0; i < count; i++) {
            free(names[i]);
        }
        free(names);
        return NULL;
    }
    size_t at = 0;
    for (size_t i = 0; i < count; i++) {
        size_t name_len = strlen(names[i]);
        memcpy(out + at, names[i], name_len);
        at += name_len;
        if (i + 1 < count) {
            out[at++] = '\n';
        }
        free(names[i]);
    }
    free(names);
    yashy_str *result = yashy_str_from_bytes((const uint8_t *)out, (long long)length);
    free(out);
    return result;
}

int yashy_fs_stat(const char *path, long long *size, int *is_dir, long long *modified_unix) {
    if (path == NULL || size == NULL || is_dir == NULL || modified_unix == NULL) {
        return -1;
    }
#if defined(_WIN32)
    struct _stat st;
    if (_stat(path, &st) != 0) {
        return -1;
    }
    *size = (long long)st.st_size;
    *is_dir = (st.st_mode & _S_IFDIR) != 0;
    *modified_unix = (long long)st.st_mtime;
#else
    struct stat st;
    if (stat(path, &st) != 0) {
        return -1;
    }
    *size = (long long)st.st_size;
    *is_dir = S_ISDIR(st.st_mode) ? 1 : 0;
    *modified_unix = (long long)st.st_mtime;
#endif
    return 0;
}

/** One file's metadata as a record: `"0\n<size>\n<is_dir>\n<modified>\n"` or
 * `"1\n"`. The record shape exists because the Yashy boundary cannot pass
 * three out-parameters and a null `yashy_str` would not be a Yashy string. */
yashy_str *yashy_fs_metadata_record(const char *path) {
    long long size = 0;
    int is_dir = 0;
    long long modified = 0;
    if (yashy_fs_stat(path, &size, &is_dir, &modified) != 0) {
        return yashy_str_from_bytes((const uint8_t *)"1\n", 2);
    }
    char header[128];
    int length = snprintf(header, sizeof(header), "0\n%lld\n%d\n%lld\n", size, is_dir,
                          modified);
    if (length <= 0) {
        return yashy_str_from_bytes((const uint8_t *)"1\n", 2);
    }
    return yashy_str_from_bytes((const uint8_t *)header, (long long)length);
}

/** The directory listing as a record: `"0\n<names>"` (empty when the
 * directory is empty) or `"1\n"` when it cannot be opened. */
yashy_str *yashy_fs_listdir_record(const char *path) {
    yashy_str *listing = yashy_fs_listdir_join(path);
    if (listing == NULL) {
        return yashy_str_from_bytes((const uint8_t *)"1\n", 2);
    }
    long long length = yashy_str_len(listing);
    char *buffer = (char *)malloc((size_t)length + 2);
    if (buffer == NULL) {
        yashy_str_release(listing);
        return yashy_str_from_bytes((const uint8_t *)"1\n", 2);
    }
    buffer[0] = '0';
    buffer[1] = '\n';
    memcpy(buffer + 2, yashy_str_ptr(listing), (size_t)length);
    yashy_str *record = yashy_str_from_bytes((const uint8_t *)buffer, length + 2);
    free(buffer);
    yashy_str_release(listing);
    return record;
}

/** The byte size of `path`, or `-1` when it cannot be stat'ed. */
long long yashy_fs_stat_size(const char *path) {
    long long size = 0;
    int is_dir = 0;
    long long modified = 0;
    if (yashy_fs_stat(path, &size, &is_dir, &modified) != 0) {
        return -1;
    }
    return size;
}

/** Whether `path` is a directory (`1`/`0`), or `-1` when it cannot be stat'ed. */
int yashy_fs_stat_is_dir(const char *path) {
    long long size = 0;
    int is_dir = 0;
    long long modified = 0;
    if (yashy_fs_stat(path, &size, &is_dir, &modified) != 0) {
        return -1;
    }
    return is_dir;
}

/** The modification time of `path` in Unix seconds, or `-1` when it cannot be
 * stat'ed. */
long long yashy_fs_stat_modified(const char *path) {
    long long size = 0;
    int is_dir = 0;
    long long modified = 0;
    if (yashy_fs_stat(path, &size, &is_dir, &modified) != 0) {
        return -1;
    }
    return modified;
}

int yashy_remove_file(const char *path) {
    if (path == NULL) {
        return -1;
    }
    return remove(path) == 0 ? 0 : -1;
}

int yashy_remove_dir(const char *path) {
    if (path == NULL) {
        return -1;
    }
#if defined(_WIN32)
    return RemoveDirectoryA(path) ? 0 : -1;
#else
    return rmdir(path) == 0 ? 0 : -1;
#endif
}

int yashy_rename(const char *old_path, const char *new_path) {
    if (old_path == NULL || new_path == NULL) {
        return -1;
    }
    return rename(old_path, new_path) == 0 ? 0 : -1;
}

int yashy_copy(const char *src, const char *dst) {
    if (src == NULL || dst == NULL) {
        return -1;
    }
    FILE *in = fopen(src, "rb");
    if (in == NULL) {
        return -1;
    }
    FILE *out = fopen(dst, "wb");
    if (out == NULL) {
        fclose(in);
        return -1;
    }
    uint8_t buffer[8192];
    size_t n;
    int ok = 1;
    while ((n = fread(buffer, 1, sizeof(buffer), in)) > 0) {
        if (fwrite(buffer, 1, n, out) != n) {
            ok = 0;
            break;
        }
    }
    if (ferror(in)) {
        ok = 0;
    }
    fclose(in);
    if (fclose(out) != 0) {
        ok = 0;
    }
    return ok ? 0 : -1;
}

/* ---- Entropy (rand): OS random bytes into `bytes` (cap `len`). ---- */

/* The entropy source, in one place.
 *
 * `BCryptGenRandom` with the system-preferred generator needs no handle, so a
 * draw is one API call with nothing to open and close; `getrandom` and
 * `/dev/urandom` are the POSIX answers. */
int yashy_entropy(uint8_t *bytes, long long len) {
    if (bytes == NULL || len < 0 || len > INT_MAX) {
        return -1;
    }
    if (len == 0) {
        return 0;
    }
#if defined(_WIN32)
    return BCryptGenRandom(NULL, bytes, (ULONG)len, BCRYPT_USE_SYSTEM_PREFERRED_RNG) == 0
               ? 0
               : -1;
#else
#if defined(SYS_getrandom)
    ssize_t done = 0;
    while (done < len) {
        ssize_t n = syscall(SYS_getrandom, bytes + done, (size_t)(len - done), 0);
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            break;
        }
        done += n;
    }
    if (done == len) {
        return 0;
    }
#endif
    FILE *urandom = fopen("/dev/urandom", "rb");
    if (urandom == NULL) {
        return -1;
    }
    size_t read = fread(bytes, 1, (size_t)len, urandom);
    fclose(urandom);
    return read == (size_t)len ? 0 : -1;
#endif
}

/* A pool of draws, so a program that asks for a million numbers does not make
 * a million system calls.
 *
 * The pool is per thread -- `__declspec(thread)` on Windows, `_Thread_local`
 * elsewhere -- so two threads never share a draw and neither of them waits for
 * the other to refill. */
#if defined(_MSC_VER)
#define YASHY_THREAD_LOCAL __declspec(thread)
#else
#define YASHY_THREAD_LOCAL _Thread_local
#endif

#define YASHY_RANDOM_POOL_SIZE 64
static YASHY_THREAD_LOCAL uint64_t yashy_random_pool[YASHY_RANDOM_POOL_SIZE];
static YASHY_THREAD_LOCAL size_t yashy_random_pool_used = YASHY_RANDOM_POOL_SIZE;

/** One 64-bit draw. OS entropy is the source; a degraded fallback keeps the
 * call total (the API has no failure value) if the OS refuses. */
long long yashy_random_i64(void) {
    if (yashy_random_pool_used >= YASHY_RANDOM_POOL_SIZE) {
        if (yashy_entropy((uint8_t *)yashy_random_pool,
                          (long long)sizeof yashy_random_pool) != 0) {
            unsigned long long value = (unsigned long long)yashy_monotonic_ns();
            value ^= ((unsigned long long)yashy_now_unix_ns()) << 21;
            return (long long)value;
        }
        yashy_random_pool_used = 0;
    }
    return (long long)yashy_random_pool[yashy_random_pool_used++];
}

/* ---- Process (process): run one child, capture stdout, run to exit.
 *
 * One blocking call: `argv` and `env` are newline-separated records, `cwd`
 * of "" means inherit. `*out_stdout` gets the child's whole stdout as a
 * string and `*exit_code` its exit status; `*pid_out` gets the child id.
 * The Yashy `Child` holds the captured stdout and the code, so `wait`
 * answers the recorded code and `kill` on an exited child is an error
 * *value*, never a second exit path. Argv only, never a shell: that is the
 * injection defence, and a test pins it.
 */

#if !defined(_WIN32)
/* The fork child's verdict on a failure that happens before `exec`: write
 * the errno through the close-on-exec pipe, then leave with the shell's
 * "could not execute" code. The write is retried on a signal interrupt —
 * a verdict lost to EINTR would read as success on the other end, which is
 * the exact bug this pipe exists to close: `execvp` cannot report failure
 * across a fork, so without the pipe a missing command exits 127 in the
 * child and the parent records it as a successful run. `write` is
 * async-signal-safe; nothing here allocates. */
static void yashy_child_exec_error(int errfd, int err) {
    size_t sent = 0;
    while (sent < sizeof err) {
        ssize_t wrote = write(errfd, (const char *)&err + sent, sizeof err - sent);
        if (wrote > 0) {
            sent += (size_t)wrote;
        } else if (wrote < 0 && errno == EINTR) {
            continue;
        } else {
            break;
        }
    }
    _exit(127);
}
#endif

#if defined(_WIN32)
/* One argv element, quoted the way the MSVC runtime splits them. */
static void yashy_append_quoted(char *line, size_t *at, const char *arg) {
    int quoted = arg[0] == '\0' || strpbrk(arg, " \t\n\v\"") != NULL;
    if (!quoted) {
        size_t length = strlen(arg);
        memcpy(line + *at, arg, length);
        *at += length;
        return;
    }
    line[(*at)++] = '"';
    for (const char *p = arg;; p++) {
        size_t backslashes = 0;
        while (*p == '\\') {
            backslashes++;
            p++;
        }
        if (*p == '\0') {
            for (size_t i = 0; i < backslashes * 2; i++) {
                line[(*at)++] = '\\';
            }
            break;
        }
        if (*p == '"') {
            for (size_t i = 0; i < backslashes * 2 + 1; i++) {
                line[(*at)++] = '\\';
            }
            line[(*at)++] = '"';
        } else {
            for (size_t i = 0; i < backslashes; i++) {
                line[(*at)++] = '\\';
            }
            line[(*at)++] = *p;
        }
    }
    line[(*at)++] = '"';
}
#endif

static int yashy_run_impl(char **argv, const char *cwd, char **env, yashy_str **out_stdout,
                          long long *exit_code, long long *pid_out) {
#if defined(_WIN32)
    /* Current environment with `env` overrides applied, as one NUL-terminated
     * block ending in a second NUL. NULL means "inherit unchanged". */
    char **records = NULL;
    size_t count = 0;
    size_t capacity = 0;
    int complete = 1;
    char *current = GetEnvironmentStringsA();
    if (current != NULL) {
        for (char *entry = current; *entry != '\0'; entry += strlen(entry) + 1) {
            if (entry[0] == '=') {
                continue;
            }
            const char *equals = strchr(entry, '=');
            size_t name_len = equals != NULL ? (size_t)(equals - entry) : strlen(entry);
            int overridden = 0;
            for (size_t i = 0; env[i] != NULL; i++) {
                const char *other = strchr(env[i], '=');
                size_t other_len = other != NULL ? (size_t)(other - env[i]) : strlen(env[i]);
                if (other_len == name_len && strncmp(entry, env[i], name_len) == 0) {
                    overridden = 1;
                    break;
                }
            }
            if (overridden) {
                continue;
            }
            if (count == capacity) {
                size_t grown = capacity == 0 ? 64 : capacity * 2;
                char **next = (char **)realloc(records, grown * sizeof(char *));
                if (next == NULL) {
                    complete = 0;
                    break;
                }
                records = next;
                capacity = grown;
            }
            size_t entry_len = strlen(entry);
            char *copy = (char *)malloc(entry_len + 1);
            if (copy == NULL) {
                complete = 0;
                break;
            }
            memcpy(copy, entry, entry_len + 1);
            records[count++] = copy;
        }
        FreeEnvironmentStringsA(current);
    }
    for (size_t i = 0; env[i] != NULL && complete; i++) {
        if (count == capacity) {
            size_t grown = capacity == 0 ? 64 : capacity * 2;
            char **next = (char **)realloc(records, grown * sizeof(char *));
            if (next == NULL) {
                complete = 0;
                break;
            }
            records = next;
            capacity = grown;
        }
        size_t entry_len = strlen(env[i]);
        char *copy = (char *)malloc(entry_len + 1);
        if (copy == NULL) {
            complete = 0;
            break;
        }
        memcpy(copy, env[i], entry_len + 1);
        records[count++] = copy;
    }
    char *block = NULL;
    if (complete) {
        size_t total = 2;
        for (size_t i = 0; i < count; i++) {
            total += strlen(records[i]) + 1;
        }
        block = (char *)malloc(total);
        if (block != NULL) {
            size_t at = 0;
            for (size_t i = 0; i < count; i++) {
                size_t entry_len = strlen(records[i]);
                memcpy(block + at, records[i], entry_len);
                at += entry_len;
                block[at++] = '\0';
            }
            block[at] = '\0';
        }
    }
    for (size_t i = 0; i < count; i++) {
        free(records[i]);
    }
    free(records);

    size_t line_capacity = 1;
    for (size_t i = 0; argv[i] != NULL; i++) {
        line_capacity += strlen(argv[i]) * 2 + 4;
    }
    char *line = (char *)malloc(line_capacity);
    if (line == NULL) {
        free(block);
        return -1;
    }
    size_t line_at = 0;
    for (size_t i = 0; argv[i] != NULL; i++) {
        if (i > 0) {
            line[line_at++] = ' ';
        }
        yashy_append_quoted(line, &line_at, argv[i]);
    }
    line[line_at] = '\0';

    SECURITY_ATTRIBUTES inherit = {sizeof(SECURITY_ATTRIBUTES), NULL, TRUE};
    HANDLE read_end = NULL;
    HANDLE write_end = NULL;
    if (CreatePipe(&read_end, &write_end, &inherit, 0) == 0) {
        free(block);
        free(line);
        return -1;
    }
    SetHandleInformation(read_end, HANDLE_FLAG_INHERIT, 0);
    STARTUPINFOA startup;
    ZeroMemory(&startup, sizeof(startup));
    startup.cb = sizeof(startup);
    startup.dwFlags = STARTF_USESTDHANDLES;
    startup.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
    startup.hStdOutput = write_end;
    startup.hStdError = GetStdHandle(STD_ERROR_HANDLE);
    PROCESS_INFORMATION child;
    ZeroMemory(&child, sizeof(child));
    BOOL started = CreateProcessA(NULL, line, NULL, NULL, TRUE, 0, block,
                                  (cwd != NULL && cwd[0] != '\0') ? cwd : NULL, &startup,
                                  &child);
    CloseHandle(write_end);
    free(block);
    free(line);
    if (started == 0) {
        CloseHandle(read_end);
        return -1;
    }
    CloseHandle(child.hThread);
    size_t capture_capacity = 8192;
    size_t capture_length = 0;
    char *captured = (char *)malloc(capture_capacity);
    if (captured != NULL) {
        for (;;) {
            if (capture_length == capture_capacity) {
                size_t grown = capture_capacity * 2;
                char *next = (char *)realloc(captured, grown);
                if (next == NULL) {
                    break;
                }
                captured = next;
                capture_capacity = grown;
            }
            DWORD read_now = 0;
            if (ReadFile(read_end, captured + capture_length,
                         (DWORD)(capture_capacity - capture_length), &read_now, NULL) == 0 ||
                read_now == 0) {
                break;
            }
            capture_length += read_now;
        }
    }
    CloseHandle(read_end);
    WaitForSingleObject(child.hProcess, INFINITE);
    DWORD status = 0;
    GetExitCodeProcess(child.hProcess, &status);
    *exit_code = (long long)status;
    *pid_out = (long long)child.dwProcessId;
    *out_stdout = yashy_str_from_bytes((const uint8_t *)captured, (long long)capture_length);
    free(captured);
    CloseHandle(child.hProcess);
    return 0;
#else
    int fds[2];
    if (pipe(fds) != 0) {
        return -1;
    }
    /* The verdict pipe: `fork` cannot carry an `exec` failure back, so the
     * child writes its errno here and `FD_CLOEXEC` closes the write end on
     * a successful exec. The parent reads it before the child's stdout:
     * bytes are the errno the child could not otherwise report (answer the
     * failure the Windows path answers), and a clean EOF means the program
     * is running. */
    int errfd[2];
    if (pipe(errfd) != 0) {
        close(fds[0]);
        close(fds[1]);
        return -1;
    }
    if (fcntl(errfd[1], F_SETFD, FD_CLOEXEC) != 0) {
        close(fds[0]);
        close(fds[1]);
        close(errfd[0]);
        close(errfd[1]);
        return -1;
    }
    pid_t child = fork();
    if (child < 0) {
        close(fds[0]);
        close(fds[1]);
        close(errfd[0]);
        close(errfd[1]);
        return -1;
    }
    if (child == 0) {
        close(errfd[0]);
        if (cwd != NULL && cwd[0] != '\0' && chdir(cwd) != 0) {
            yashy_child_exec_error(errfd[1], errno);
        }
        if (dup2(fds[1], STDOUT_FILENO) < 0) {
            yashy_child_exec_error(errfd[1], errno);
        }
        close(fds[0]);
        close(fds[1]);
        if (env != NULL) {
            for (char **entry = env; *entry != NULL; entry++) {
                char *equals = strchr(*entry, '=');
                if (equals != NULL) {
                    *equals = '\0';
                    setenv(*entry, equals + 1, 1);
                }
            }
        }
        execvp(argv[0], argv);
        yashy_child_exec_error(errfd[1], errno);
    }
    *pid_out = (long long)child;
    close(fds[1]);
    close(errfd[1]);
    int exec_errno = 0;
    size_t got = 0;
    int verdict_unreadable = 0;
    for (;;) {
        ssize_t n = read(errfd[0], (char *)&exec_errno + got, sizeof(int) - got);
        if (n > 0) {
            got += (size_t)n;
            if (got == sizeof(int)) {
                break;
            }
        } else if (n == 0) {
            break;
        } else if (errno != EINTR) {
            verdict_unreadable = 1;
            break;
        }
    }
    close(errfd[0]);
    if (got > 0 || verdict_unreadable) {
        /* The child could not prepare or exec: reap it and answer the
         * failure. `exec_errno` carries the cause for a debugger; the value
         * surface is the same `Err` the Windows path produces. */
        close(fds[0]);
        int status;
        while (waitpid(child, &status, 0) < 0 && errno == EINTR) {
        }
        return -1;
    }
    size_t capacity = 8192;
    size_t length = 0;
    uint8_t *data = (uint8_t *)malloc(capacity);
    if (data == NULL) {
        close(fds[0]);
        int status;
        waitpid(child, &status, 0);
        return -1;
    }
    for (;;) {
        if (length == capacity) {
            capacity *= 2;
            uint8_t *grown = (uint8_t *)realloc(data, capacity);
            if (grown == NULL) {
                free(data);
                close(fds[0]);
                int status;
                waitpid(child, &status, 0);
                return -1;
            }
            data = grown;
        }
        ssize_t n = read(fds[0], data + length, capacity - length);
        if (n < 0 && errno == EINTR) {
            continue;
        }
        if (n <= 0) {
            break;
        }
        length += (size_t)n;
    }
    close(fds[0]);
    int status = 0;
    waitpid(child, &status, 0);
    *exit_code = WIFEXITED(status) ? WEXITSTATUS(status) : -1;
    *out_stdout = yashy_str_from_bytes(data, (long long)length);
    free(data);
    return 0;
#endif
}

int yashy_spawn(const char *argv_text, const char *cwd, const char *env_text, yashy_str **out,
                long long *exit_code, long long *pid_out) {
    if (argv_text == NULL || cwd == NULL || env_text == NULL || out == NULL ||
        exit_code == NULL || pid_out == NULL || argv_text[0] == '\0') {
        return -1;
    }
    /* Duplicate both record blocks so the child can split them in place. */
    size_t argv_len = strlen(argv_text);
    char *argv_block = (char *)malloc(argv_len + 1);
    size_t env_len = strlen(env_text);
    char *env_block = (char *)malloc(env_len + 1);
    if (argv_block == NULL || env_block == NULL) {
        free(argv_block);
        free(env_block);
        return -1;
    }
    memcpy(argv_block, argv_text, argv_len + 1);
    memcpy(env_block, env_text, env_len + 1);
    /* Each block holds NUL-terminated records the writer joined with '\n'.
       Count newlines first, replace them with NULs, and point an array at
       each record. */
    size_t argc = 1;
    for (size_t i = 0; i < argv_len; i++) {
        if (argv_block[i] == '\n') {
            argv_block[i] = '\0';
            argc++;
        }
    }
    size_t envc = 0;
    if (env_text[0] != '\0') {
        envc = 1;
        for (size_t i = 0; i < env_len; i++) {
            if (env_block[i] == '\n') {
                env_block[i] = '\0';
                envc++;
            }
        }
    }
    char **argv = (char **)malloc(sizeof(char *) * (argc + 1));
    char **env = (char **)malloc(sizeof(char *) * (envc + 1));
    if (argv == NULL || env == NULL) {
        free(argv);
        free(env);
        free(argv_block);
        free(env_block);
        return -1;
    }
    char *cursor = argv_block;
    for (size_t i = 0; i < argc; i++) {
        argv[i] = cursor;
        cursor += strlen(cursor) + 1;
    }
    argv[argc] = NULL;
    cursor = env_block;
    for (size_t i = 0; i < envc; i++) {
        env[i] = cursor;
        cursor += strlen(cursor) + 1;
    }
    env[envc] = NULL;
    int rc = yashy_run_impl(argv, cwd, env, out, exit_code, pid_out);
    free(argv);
    free(env);
    free(argv_block);
    free(env_block);
    return rc;
}

/* The Yashy binding cannot pass three out-parameters, so the result is one
 * record: `"0\n<exit>\n<pid>\n<stdout>"` on success, `"1\n"` on failure. The
 * leading status is the reason the failure case is still a string, not NULL:
 * a Yashy `string` is never a null pointer. */
yashy_str *yashy_spawn_record(const char *argv_text, const char *cwd, const char *env_text) {
    static const char failed[] = "1\n";
    yashy_str *captured = NULL;
    long long code = -1;
    long long pid = -1;
    if (yashy_spawn(argv_text, cwd, env_text, &captured, &code, &pid) != 0) {
        return yashy_str_from_bytes((const uint8_t *)failed, 2);
    }
    char header[96];
    int header_len = snprintf(header, sizeof(header), "0\n%lld\n%lld\n", code, pid);
    if (header_len <= 0) {
        yashy_str_release(captured);
        return yashy_str_from_bytes((const uint8_t *)failed, 2);
    }
    long long text_len = yashy_str_len(captured);
    size_t total = (size_t)header_len + (size_t)text_len;
    char *buffer = (char *)malloc(total);
    if (buffer == NULL) {
        yashy_str_release(captured);
        return yashy_str_from_bytes((const uint8_t *)failed, 2);
    }
    memcpy(buffer, header, (size_t)header_len);
    memcpy(buffer + header_len, yashy_str_ptr(captured), (size_t)text_len);
    yashy_str *record = yashy_str_from_bytes((const uint8_t *)buffer, (long long)total);
    free(buffer);
    yashy_str_release(captured);
    return record;
}

/* Stdin to EOF as one record: `"0\n<bytes>"`, or `"1\n"` when the read
 * fails. Same leading-status shape as `yashy_spawn_record`. */
yashy_str *yashy_stdin_record(void) {
    size_t capacity = 8192;
    size_t length = 0;
    char *data = (char *)malloc(capacity);
    if (data == NULL) {
        return yashy_str_from_bytes((const uint8_t *)"1\n", 2);
    }
    for (;;) {
        if (length == capacity) {
            size_t grown = capacity * 2;
            char *next = (char *)realloc(data, grown);
            if (next == NULL) {
                free(data);
                return yashy_str_from_bytes((const uint8_t *)"1\n", 2);
            }
            data = next;
            capacity = grown;
        }
        size_t read_now = fread(data + length, 1, capacity - length, stdin);
        length += read_now;
        if (read_now == 0) {
            if (ferror(stdin) != 0) {
                free(data);
                return yashy_str_from_bytes((const uint8_t *)"1\n", 2);
            }
            break;
        }
    }
    char *out = (char *)malloc(length + 2);
    if (out == NULL) {
        free(data);
        return yashy_str_from_bytes((const uint8_t *)"1\n", 2);
    }
    out[0] = '0';
    out[1] = '\n';
    memcpy(out + 2, data, length);
    yashy_str *record = yashy_str_from_bytes((const uint8_t *)out, (long long)(length + 2));
    free(out);
    free(data);
    return record;
}
