/* Yashy-facing shim over the pinned SQLite amalgamation.
 *
 * SQLite is a single vetted C source file, so it fits the `link` convention
 * the compiler already has for vendored C. This shim owns the only raw
 * pointers: a Yashy program sees integer handles and strings. A failed call
 * reports a status; a message or an empty result set is a value, never the
 * failure signal.
 */
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "sqlite3.h"

/* The driver compiles link inputs without the runtime include path. These are
 * the same ABI facts the generated IR uses for every Yashy string. */
typedef struct {
    long long refcount;
    long long len;
    long long cap;
    char *data;
} yashy_str;

extern yashy_str *yashy_str_from_bytes(const uint8_t *bytes, long long len);
extern long long yashy_str_len(const yashy_str *s);
extern const char *yashy_str_ptr(const yashy_str *s);

#define YASHY_SQLITE_MAX_DB 64
#define YASHY_SQLITE_MAX_STMT 256

static sqlite3 *yashy_dbs[YASHY_SQLITE_MAX_DB];
static sqlite3_stmt *yashy_stmts[YASHY_SQLITE_MAX_STMT];
static char yashy_sqlite_message[1024];

static yashy_str *yashy_sqlite_string(const char *text, long long len) {
    if (text == NULL) return yashy_str_from_bytes((const uint8_t *)"", 0);
    if (len < 0) len = (long long)strlen(text);
    return yashy_str_from_bytes((const uint8_t *)text, len);
}

static void yashy_sqlite_note(sqlite3 *db, int rc) {
    const char *message = db != NULL ? sqlite3_errmsg(db) : "invalid database handle";
    size_t n = strlen(message);
    if (n >= sizeof(yashy_sqlite_message)) n = sizeof(yashy_sqlite_message) - 1;
    memcpy(yashy_sqlite_message, message, n);
    yashy_sqlite_message[n] = '\0';
    (void)rc;
}

/* Open a database. `":memory:"` is the in-memory database. Returns 0 on
 * failure; a positive value is a database handle. */
long long yashy_sqlite_open(const yashy_str *path) {
    const char *name = path == NULL ? "" : yashy_str_ptr(path);
    sqlite3 *db = NULL;
    int rc = sqlite3_open(name, &db);
    if (rc != SQLITE_OK) {
        yashy_sqlite_note(db, rc);
        if (db != NULL) sqlite3_close(db);
        return 0;
    }
    for (int i = 1; i < YASHY_SQLITE_MAX_DB; i++) {
        if (yashy_dbs[i] == NULL) {
            yashy_dbs[i] = db;
            return i;
        }
    }
    sqlite3_close(db);
    yashy_sqlite_message[0] = '\0';
    strcpy(yashy_sqlite_message, "database table full");
    return 0;
}

void yashy_sqlite_close(long long handle) {
    if (handle <= 0 || handle >= YASHY_SQLITE_MAX_DB) return;
    sqlite3 *db = yashy_dbs[handle];
    if (db == NULL) return;
    for (int i = 1; i < YASHY_SQLITE_MAX_STMT; i++) {
        /* Statements are finalized by the caller; close still runs after them. */
        (void)i;
    }
    sqlite3_close(db);
    yashy_dbs[handle] = NULL;
}

/* Execute a statement that returns no rows. 0 means success. */
int yashy_sqlite_exec(long long handle, const yashy_str *sql) {
    if (handle <= 0 || handle >= YASHY_SQLITE_MAX_DB || yashy_dbs[handle] == NULL) {
        strcpy(yashy_sqlite_message, "invalid database handle");
        return 1;
    }
    const char *text = sql == NULL ? "" : yashy_str_ptr(sql);
    char *error = NULL;
    int rc = sqlite3_exec(yashy_dbs[handle], text, NULL, NULL, &error);
    if (rc != SQLITE_OK) {
        yashy_sqlite_note(yashy_dbs[handle], rc);
        if (error != NULL) sqlite3_free(error);
        return 1;
    }
    yashy_sqlite_message[0] = '\0';
    return 0;
}

/* Prepare a parameterised statement. Returns 0 on failure. */
long long yashy_sqlite_prepare(long long handle, const yashy_str *sql) {
    if (handle <= 0 || handle >= YASHY_SQLITE_MAX_DB || yashy_dbs[handle] == NULL) {
        strcpy(yashy_sqlite_message, "invalid database handle");
        return 0;
    }
    sqlite3_stmt *stmt = NULL;
    const char *text = sql == NULL ? "" : yashy_str_ptr(sql);
    int rc = sqlite3_prepare_v2(yashy_dbs[handle], text, -1, &stmt, NULL);
    if (rc != SQLITE_OK) {
        yashy_sqlite_note(yashy_dbs[handle], rc);
        return 0;
    }
    for (int i = 1; i < YASHY_SQLITE_MAX_STMT; i++) {
        if (yashy_stmts[i] == NULL) {
            yashy_stmts[i] = stmt;
            yashy_sqlite_message[0] = '\0';
            return i;
        }
    }
    sqlite3_finalize(stmt);
    strcpy(yashy_sqlite_message, "statement table full");
    return 0;
}

int yashy_sqlite_bind_text(long long stmt_id, int index, const yashy_str *value) {
    if (stmt_id <= 0 || stmt_id >= YASHY_SQLITE_MAX_STMT || yashy_stmts[stmt_id] == NULL) {
        strcpy(yashy_sqlite_message, "invalid statement handle");
        return 1;
    }
    const char *text = value == NULL ? "" : yashy_str_ptr(value);
    long long len = value == NULL ? 0 : yashy_str_len(value);
    int rc = sqlite3_bind_text(yashy_stmts[stmt_id], index, text, (int)len, SQLITE_TRANSIENT);
    if (rc != SQLITE_OK) {
        yashy_sqlite_note(sqlite3_db_handle(yashy_stmts[stmt_id]), rc);
        return 1;
    }
    return 0;
}

int yashy_sqlite_bind_int(long long stmt_id, int index, long long value) {
    if (stmt_id <= 0 || stmt_id >= YASHY_SQLITE_MAX_STMT || yashy_stmts[stmt_id] == NULL) {
        strcpy(yashy_sqlite_message, "invalid statement handle");
        return 1;
    }
    int rc = sqlite3_bind_int64(yashy_stmts[stmt_id], index, value);
    if (rc != SQLITE_OK) {
        yashy_sqlite_note(sqlite3_db_handle(yashy_stmts[stmt_id]), rc);
        return 1;
    }
    return 0;
}

int yashy_sqlite_bind_null(long long stmt_id, int index) {
    if (stmt_id <= 0 || stmt_id >= YASHY_SQLITE_MAX_STMT || yashy_stmts[stmt_id] == NULL) {
        strcpy(yashy_sqlite_message, "invalid statement handle");
        return 1;
    }
    int rc = sqlite3_bind_null(yashy_stmts[stmt_id], index);
    if (rc != SQLITE_OK) {
        yashy_sqlite_note(sqlite3_db_handle(yashy_stmts[stmt_id]), rc);
        return 1;
    }
    return 0;
}

/* 1: a row is available. 0: done. -1: error. */
int yashy_sqlite_step(long long stmt_id) {
    if (stmt_id <= 0 || stmt_id >= YASHY_SQLITE_MAX_STMT || yashy_stmts[stmt_id] == NULL) {
        strcpy(yashy_sqlite_message, "invalid statement handle");
        return -1;
    }
    int rc = sqlite3_step(yashy_stmts[stmt_id]);
    if (rc == SQLITE_ROW) return 1;
    if (rc == SQLITE_DONE) return 0;
    yashy_sqlite_note(sqlite3_db_handle(yashy_stmts[stmt_id]), rc);
    return -1;
}

int yashy_sqlite_column_count(long long stmt_id) {
    if (stmt_id <= 0 || stmt_id >= YASHY_SQLITE_MAX_STMT || yashy_stmts[stmt_id] == NULL) return 0;
    return sqlite3_column_count(yashy_stmts[stmt_id]);
}

/* SQLite type code: 1 int, 2 float, 3 text, 4 blob, 5 null. */
int yashy_sqlite_column_type(long long stmt_id, int column) {
    if (stmt_id <= 0 || stmt_id >= YASHY_SQLITE_MAX_STMT || yashy_stmts[stmt_id] == NULL) return 5;
    switch (sqlite3_column_type(yashy_stmts[stmt_id], column)) {
        case SQLITE_INTEGER: return 1;
        case SQLITE_FLOAT: return 2;
        case SQLITE_TEXT: return 3;
        case SQLITE_BLOB: return 4;
        default: return 5;
    }
}

yashy_str *yashy_sqlite_column_text(long long stmt_id, int column) {
    if (stmt_id <= 0 || stmt_id >= YASHY_SQLITE_MAX_STMT || yashy_stmts[stmt_id] == NULL) {
        return yashy_sqlite_string("", 0);
    }
    const unsigned char *text = sqlite3_column_text(yashy_stmts[stmt_id], column);
    int bytes = sqlite3_column_bytes(yashy_stmts[stmt_id], column);
    if (text == NULL) return yashy_sqlite_string("", 0);
    return yashy_sqlite_string((const char *)text, bytes);
}

long long yashy_sqlite_column_int(long long stmt_id, int column) {
    if (stmt_id <= 0 || stmt_id >= YASHY_SQLITE_MAX_STMT || yashy_stmts[stmt_id] == NULL) return 0;
    return sqlite3_column_int64(yashy_stmts[stmt_id], column);
}

void yashy_sqlite_finalize(long long stmt_id) {
    if (stmt_id <= 0 || stmt_id >= YASHY_SQLITE_MAX_STMT) return;
    if (yashy_stmts[stmt_id] != NULL) sqlite3_finalize(yashy_stmts[stmt_id]);
    yashy_stmts[stmt_id] = NULL;
}

yashy_str *yashy_sqlite_error_message(void) {
    return yashy_sqlite_string(yashy_sqlite_message, -1);
}

void yashy_sqlite_busy_timeout(long long handle, int millis) {
    if (handle <= 0 || handle >= YASHY_SQLITE_MAX_DB || yashy_dbs[handle] == NULL) return;
    sqlite3_busy_timeout(yashy_dbs[handle], millis);
}

long long yashy_sqlite_changes(long long handle) {
    if (handle <= 0 || handle >= YASHY_SQLITE_MAX_DB || yashy_dbs[handle] == NULL) return 0;
    return sqlite3_changes(yashy_dbs[handle]);
}

long long yashy_sqlite_last_insert_rowid(long long handle) {
    if (handle <= 0 || handle >= YASHY_SQLITE_MAX_DB || yashy_dbs[handle] == NULL) return 0;
    return sqlite3_last_insert_rowid(yashy_dbs[handle]);
}

/* sqlite3_libversion returns a C string, not a Yashy string. */
yashy_str *yashy_sqlite_libversion(void) {
    return yashy_sqlite_string(sqlite3_libversion(), -1);
}
