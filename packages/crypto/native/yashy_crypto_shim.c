/* Yashy-facing shim over the vendored Monocypher sources.
 *
 * The generated program passes and returns Yashy strings, whose C shape is
 * `yashy_str`. Keeping that boundary here means the Yashy wrapper never has
 * to name a raw byte pointer or a C allocation. Every returned string is a
 * fresh Yashy string the caller owns; a failure is reported by a separate
 * status function, never by an empty or zero value.
 *
 * Monocypher supplies the S-boxes, field arithmetic, constant-time tag checks
 * and key derivation. This file only moves bytes between Yashy strings and
 * those functions, and spells the hex form the Yashy API uses.
 */
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "monocypher.h"
#include "optional/monocypher-ed25519.h"

/* The driver compiles link inputs without the runtime include path, so the
 * three string ABI facts the shim needs are stated here. They are the same
 * declarations the generated IR uses for every string: a Yashy `string` is a
 * pointer to this four-word record. */
typedef struct {
    long long refcount;
    long long len;
    long long cap;
    char *data;
} yashy_str;

extern yashy_str *yashy_str_from_bytes(const uint8_t *bytes, long long len);
extern long long yashy_str_len(const yashy_str *s);
extern const char *yashy_str_ptr(const yashy_str *s);

static int yashy_hex_value(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}

static const char yashy_hex_digits[] = "0123456789abcdef";

static yashy_str *yashy_hex_string(const uint8_t *bytes, size_t len) {
    char *out = (char *)malloc(len * 2 + 1);
    if (out == NULL) return NULL;
    for (size_t i = 0; i < len; i++) {
        out[i * 2] = yashy_hex_digits[bytes[i] >> 4];
        out[i * 2 + 1] = yashy_hex_digits[bytes[i] & 15];
    }
    out[len * 2] = '\0';
    yashy_str *result = yashy_str_from_bytes((const uint8_t *)out, (long long)(len * 2));
    free(out);
    return result;
}

/* Decode a fixed-width hex string into `out`, returning 1 only on a
 * well-formed input of exactly `len` bytes. */
static int yashy_hex_bytes(const yashy_str *text, uint8_t *out, size_t len) {
    if (text == NULL) return 0;
    long long size = yashy_str_len(text);
    if (size != (long long)(len * 2)) return 0;
    const char *data = yashy_str_ptr(text);
    for (size_t i = 0; i < len; i++) {
        int high = yashy_hex_value(data[i * 2]);
        int low = yashy_hex_value(data[i * 2 + 1]);
        if (high < 0 || low < 0) return 0;
        out[i] = (uint8_t)((high << 4) | low);
    }
    return 1;
}

/* Decode a variable-length hex string into a fresh byte buffer. */
static uint8_t *yashy_hex_alloc(const yashy_str *text, size_t *out_len) {
    if (text == NULL) return NULL;
    long long size = yashy_str_len(text);
    if (size <= 0 || size % 2 != 0) return NULL;
    uint8_t *bytes = (uint8_t *)malloc((size_t)(size / 2));
    if (bytes == NULL) return NULL;
    if (!yashy_hex_bytes(text, bytes, (size_t)(size / 2))) {
        free(bytes);
        return NULL;
    }
    *out_len = (size_t)(size / 2);
    return bytes;
}

static size_t yashy_text_size(const yashy_str *text) {
    return text == NULL ? 0 : (size_t)yashy_str_len(text);
}

static const uint8_t *yashy_text_data(const yashy_str *text) {
    return (const uint8_t *)(text == NULL ? NULL : yashy_str_ptr(text));
}

/* A C failure is a null string. The wrapper asks this before it turns the
 * answer into `Ok`, so a sentinel never crosses into Yashy. */
int yashy_crypto_ok(const yashy_str *value) {
    return value != NULL;
}

int yashy_crypto_valid_any_hex(const yashy_str *text) {
    if (text == NULL || yashy_str_len(text) <= 0 || yashy_str_len(text) % 2 != 0) return 0;
    const char *data = yashy_str_ptr(text);
    for (long long i = 0; i < yashy_str_len(text); i++) {
        if (yashy_hex_value(data[i]) < 0) return 0;
    }
    return 1;
}

int yashy_crypto_valid_hex(const yashy_str *text, long long bytes) {
    if (bytes <= 0 || text == NULL || yashy_str_len(text) != bytes * 2) return 0;
    const char *data = yashy_str_ptr(text);
    for (long long i = 0; i < bytes * 2; i++) {
        if (yashy_hex_value(data[i]) < 0) return 0;
    }
    return 1;
}

/* ---- Hashes and key derivation ------------------------------------------- */

yashy_str *yashy_crypto_sha512_hex(const yashy_str *message) {
    uint8_t digest[64];
    crypto_sha512(digest, yashy_text_data(message), yashy_text_size(message));
    return yashy_hex_string(digest, sizeof digest);
}

yashy_str *yashy_crypto_hmac_sha512_hex(const yashy_str *key, const yashy_str *message) {
    size_t key_size = 0;
    uint8_t *key_bytes = yashy_hex_alloc(key, &key_size);
    if (key_bytes == NULL) return NULL;
    uint8_t mac[64];
    crypto_sha512_hmac(mac, key_bytes, key_size,
                       yashy_text_data(message), yashy_text_size(message));
    yashy_str *result = yashy_hex_string(mac, sizeof mac);
    crypto_wipe(key_bytes, key_size);
    free(key_bytes);
    return result;
}

yashy_str *yashy_crypto_hkdf_sha512_hex(const yashy_str *ikm, const yashy_str *salt,
                                        const yashy_str *info, long long length) {
    if (length <= 0 || length > 256) return NULL;
    uint8_t *okm = (uint8_t *)malloc((size_t)length);
    if (okm == NULL) return NULL;
    size_t ikm_size = 0, salt_size = 0, info_size = 0;
    uint8_t *ikm_bytes = yashy_hex_alloc(ikm, &ikm_size);
    uint8_t *salt_bytes = yashy_hex_alloc(salt, &salt_size);
    uint8_t *info_bytes = yashy_hex_alloc(info, &info_size);
    if (ikm_bytes == NULL || salt_bytes == NULL || info_bytes == NULL) {
        free(ikm_bytes);
        free(salt_bytes);
        free(info_bytes);
        free(okm);
        return NULL;
    }
    crypto_sha512_hkdf(okm, (size_t)length, ikm_bytes, ikm_size,
                       salt_bytes, salt_size, info_bytes, info_size);
    yashy_str *result = yashy_hex_string(okm, (size_t)length);
    crypto_wipe(ikm_bytes, ikm_size);
    free(ikm_bytes);
    free(salt_bytes);
    free(info_bytes);
    free(okm);
    return result;
}

/* ---- Ed25519 ------------------------------------------------------------- */

yashy_str *yashy_crypto_ed25519_key_pair_hex(const yashy_str *seed_hex) {
    uint8_t seed[32], secret[64], public[32];
    if (!yashy_hex_bytes(seed_hex, seed, sizeof seed)) return NULL;
    crypto_ed25519_key_pair(secret, public, seed);
    uint8_t joined[96];
    memcpy(joined, secret, sizeof secret);
    memcpy(joined + sizeof secret, public, sizeof public);
    yashy_str *result = yashy_hex_string(joined, sizeof joined);
    crypto_wipe(secret, sizeof secret);
    return result;
}

yashy_str *yashy_crypto_ed25519_sign_hex(const yashy_str *secret_hex, const yashy_str *message) {
    uint8_t secret[64], signature[64];
    if (!yashy_hex_bytes(secret_hex, secret, sizeof secret)) return NULL;
    crypto_ed25519_sign(signature, secret, yashy_text_data(message), yashy_text_size(message));
    yashy_str *result = yashy_hex_string(signature, sizeof signature);
    crypto_wipe(secret, sizeof secret);
    return result;
}

int yashy_crypto_ed25519_check_hex(const yashy_str *signature_hex,
                                   const yashy_str *public_hex,
                                   const yashy_str *message) {
    uint8_t signature[64], public[32];
    if (!yashy_hex_bytes(signature_hex, signature, sizeof signature)) return 0;
    if (!yashy_hex_bytes(public_hex, public, sizeof public)) return 0;
    return crypto_ed25519_check(signature, public, yashy_text_data(message),
                                yashy_text_size(message)) == 0;
}

/* ---- X25519 -------------------------------------------------------------- */

yashy_str *yashy_crypto_x25519_public_hex(const yashy_str *secret_hex) {
    uint8_t secret[32], public[32];
    if (!yashy_hex_bytes(secret_hex, secret, sizeof secret)) return NULL;
    crypto_x25519_public_key(public, secret);
    yashy_str *result = yashy_hex_string(public, sizeof public);
    crypto_wipe(secret, sizeof secret);
    return result;
}

yashy_str *yashy_crypto_x25519_shared_hex(const yashy_str *secret_hex,
                                          const yashy_str *public_hex) {
    uint8_t secret[32], public[32], shared[32];
    if (!yashy_hex_bytes(secret_hex, secret, sizeof secret)) return NULL;
    if (!yashy_hex_bytes(public_hex, public, sizeof public)) return NULL;
    crypto_x25519(shared, secret, public);
    yashy_str *result = yashy_hex_string(shared, sizeof shared);
    crypto_wipe(secret, sizeof secret);
    crypto_wipe(shared, sizeof shared);
    return result;
}

/* ---- ChaCha20-Poly1305 --------------------------------------------------- */

/* The hex form is a 16-byte tag followed by the ciphertext. */
static int yashy_aead_parts(const yashy_str *combined, const yashy_str *key_hex,
                            const yashy_str *nonce_hex, uint8_t *key, uint8_t *nonce,
                            uint8_t **mac, uint8_t **cipher, size_t *text_size) {
    if (!yashy_hex_bytes(key_hex, key, 32)) return 0;
    if (!yashy_hex_bytes(nonce_hex, nonce, 24)) return 0;
    long long size = yashy_text_size(combined);
    if (size < 32 || size % 2 != 0) return 0;
    *text_size = (size_t)(size / 2) - 16;
    uint8_t *bytes = (uint8_t *)malloc((size_t)(size / 2));
    if (bytes == NULL) return 0;
    if (!yashy_hex_bytes(combined, bytes, (size_t)(size / 2))) {
        free(bytes);
        return 0;
    }
    *mac = bytes;
    *cipher = bytes + 16;
    return 1;
}

yashy_str *yashy_crypto_aead_lock_hex(const yashy_str *key_hex, const yashy_str *nonce_hex,
                                      const yashy_str *ad, const yashy_str *message) {
    uint8_t key[32], nonce[24], mac[16];
    if (!yashy_hex_bytes(key_hex, key, sizeof key)) return NULL;
    if (!yashy_hex_bytes(nonce_hex, nonce, sizeof nonce)) return NULL;
    size_t text_size = yashy_text_size(message);
    uint8_t *cipher = (uint8_t *)malloc(text_size + 1);
    if (cipher == NULL) return NULL;
    crypto_aead_lock(cipher, mac, key, nonce, yashy_text_data(ad), yashy_text_size(ad),
                     yashy_text_data(message), text_size);
    uint8_t *joined = (uint8_t *)malloc(text_size + 16);
    if (joined == NULL) {
        free(cipher);
        return NULL;
    }
    memcpy(joined, mac, 16);
    memcpy(joined + 16, cipher, text_size);
    yashy_str *result = yashy_hex_string(joined, text_size + 16);
    free(joined);
    free(cipher);
    crypto_wipe(key, sizeof key);
    return result;
}

int yashy_crypto_aead_unlock_ok(const yashy_str *key_hex, const yashy_str *nonce_hex,
                                const yashy_str *ad, const yashy_str *combined) {
    uint8_t key[32], nonce[24], *mac = NULL, *cipher = NULL;
    size_t text_size = 0;
    if (!yashy_aead_parts(combined, key_hex, nonce_hex, key, nonce,
                          &mac, &cipher, &text_size)) {
        return 0;
    }
    uint8_t *plain = (uint8_t *)malloc(text_size + 1);
    if (plain == NULL) {
        free(mac);
        return 0;
    }
    int ok = crypto_aead_unlock(plain, mac, key, nonce, yashy_text_data(ad),
                                yashy_text_size(ad), cipher, text_size) == 0;
    free(plain);
    free(mac);
    crypto_wipe(key, sizeof key);
    return ok;
}

yashy_str *yashy_crypto_aead_unlock_hex(const yashy_str *key_hex, const yashy_str *nonce_hex,
                                        const yashy_str *ad, const yashy_str *combined) {
    uint8_t key[32], nonce[24], *mac = NULL, *cipher = NULL;
    size_t text_size = 0;
    if (!yashy_aead_parts(combined, key_hex, nonce_hex, key, nonce,
                          &mac, &cipher, &text_size)) {
        return NULL;
    }
    uint8_t *plain = (uint8_t *)malloc(text_size + 1);
    if (plain == NULL) {
        free(mac);
        return NULL;
    }
    if (crypto_aead_unlock(plain, mac, key, nonce, yashy_text_data(ad),
                           yashy_text_size(ad), cipher, text_size) != 0) {
        free(plain);
        free(mac);
        return NULL;
    }
    yashy_str *result = yashy_hex_string(plain, text_size);
    free(plain);
    free(mac);
    crypto_wipe(key, sizeof key);
    return result;
}

int yashy_crypto_verify16_hex(const yashy_str *left_hex, const yashy_str *right_hex) {
    uint8_t left[16], right[16];
    if (!yashy_hex_bytes(left_hex, left, sizeof left)) return 0;
    if (!yashy_hex_bytes(right_hex, right, sizeof right)) return 0;
    return crypto_verify16(left, right) == 0;
}
