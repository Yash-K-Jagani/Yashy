# The standard library

Generated from the `///` and `/** */` doc blocks by `yashyc doc`; edit the
module, not this page. `cargo test -p yashy-driver --test docs` fails
when the page and the compiler's tables disagree.

## `JSON`

### `escape`

```yashy
fn escape(string text) -> string
```

`text` with `\`, `"`, newline, carriage return and tab written as their
JSON escapes. The result is a fresh owned string.

### `quote`

```yashy
fn quote(string text) -> string
```

`escape(text)` inside double quotes: a JSON string literal. The result is a
fresh owned string.

## `alloc`

### `alloc_one`

```yashy
fn alloc_one(int value) -> *mut int
```

A pointer to one `int` slot holding `value`. The caller owns it; release it
with `.free()`.

### `alloc_zeroed`

```yashy
fn alloc_zeroed(int count) -> *mut int
```

`alloc(count)` counts `int` slots, not bytes; Yashy has no byte type yet.
A pointer to `count` zeroed `int` slots. The caller owns it; release it with
`.free()`.

### `global_alloc`

```yashy
fn global_alloc(int count) -> *mut int
```

Allocate `count` zeroed `int` slots through the global allocator.
The caller owns the result; release it with `global_free` or `.free()`.

### `global_allocator`

```yashy
fn global_allocator() -> int
```

The identity of the process-wide allocator.

A collection that does not name an allocator uses this one. The value is
stable for the life of the program; it is an identity, not a handle that
needs releasing. A collection that takes an allocator records this value
and reaches the same runtime allocation path as a bare `alloc` call.

### `global_free`

```yashy
fn global_free(*mut int pointer) -> void
```

Release a block handed out by `global_alloc`. Tolerates a null pointer.

## `args`

### `all`

```yashy
fn all() -> []string
```

The process's user arguments as a fresh owned `[]string`.

### `flag`

```yashy
fn flag(string name) -> bool
```

Whether `--name` appears in the arguments.

### `nth`

```yashy
fn nth(int index) -> Option[string]
```

The argument at `index`, or `None` outside `0..len()`.

The name is `nth`, not `get`, because every module's items share one root
scope: `env.get` already owns `get`, and two modules imported together may
not declare the same name.

## `collections`

### `deque_back`

```yashy
fn deque_back[T]([]T items) -> QPop[T]
```

### `deque_front`

```yashy
fn deque_front[T]([]T items) -> QPop[T]
```

### `deque_pop_back`

```yashy
fn deque_pop_back[T]([]T items) -> QPop[T]
```

### `deque_pop_front`

```yashy
fn deque_pop_front[T]([]T items) -> QPop[T]
```

### `deque_push_back`

```yashy
fn deque_push_back[T]([]T items, T value) -> []T
```

### `deque_push_front`

```yashy
fn deque_push_front[T]([]T items, T value) -> []T
```

### `int_sort_insert`

```yashy
fn int_sort_insert([]int items, int value) -> []int
```

### `queue_peek`

```yashy
fn queue_peek[T]([]T items) -> QPop[T]
```

### `queue_pop`

```yashy
fn queue_pop[T]([]T items, fn(T, T) -> int cmp) -> QPop[T]
```

### `queue_push`

```yashy
fn queue_push[T]([]T items, T value, fn(T, T) -> int cmp) -> []T
```

### `set_contains`

```yashy
fn set_contains[T]([]T items, T value) -> QHas[T]
```

### `set_insert`

```yashy
fn set_insert[T]([]T items, T value) -> []T
```

### `set_remove`

```yashy
fn set_remove[T]([]T items, T value) -> []T
```

## `convert`

### `ConvertError`

```yashy
enum ConvertError
```

### `bool_to_string`

```yashy
fn bool_to_string(bool value) -> string
```

A boolean as `true` or `false`, the spellings `parse_bool` accepts.

### `ceil`

```yashy
fn ceil(float value) -> int
```

Round a float up. A positive non-integer moves one above the truncation.

### `float_to_string`

```yashy
fn float_to_string(float value) -> string
```

A float as decimal text.

### `floor`

```yashy
fn floor(float value) -> int
```

Round a float down. A negative non-integer moves one below the truncation.

### `int_to_string`

```yashy
fn int_to_string(int value) -> string
```

An integer as decimal text.

### `parse_bool`

```yashy
fn parse_bool(string text) -> Result[bool, ConvertError]
```

Parse `true`, `false`, `1` or `0`. Other spellings are `Invalid`.

### `parse_float`

```yashy
fn parse_float(string text) -> Result[float, ConvertError]
```

Parse a decimal float. This is the `Result` spelling of `parse_f64`.

### `parse_int`

```yashy
fn parse_int(string text) -> Result[int, ConvertError]
```

Parse a decimal integer. This is the `Result` spelling of `parse_i64`.

### `parse_radix`

```yashy
fn parse_radix(string text, int radix) -> Result[int, ConvertError]
```

Parse an unsigned magnitude in a radix from 2 through 36.

Digits are `0`..`9`, `a`..`z` and `A`..`Z`; their value must be below the
radix. The magnitude is bounded by `2147483647` so that a non-decimal parse
never panics on overflow: a larger value answers `Overflow` instead.

### `round`

```yashy
fn round(float value) -> int
```

Round a float to the nearest integer, halves away from zero.

### `trunc`

```yashy
fn trunc(float value) -> int
```

Truncate a float toward zero.

### `try_from_float`

```yashy
fn try_from_float(float value) -> Result[int, ConvertError]
```

A checked float-to-int conversion with a documented range.

Values outside `trunc(int)`'s decimal range answer `Overflow` rather than
panicking or wrapping.

## `csv`

### `CsvError`

```yashy
enum CsvError
```

### `fields`

```yashy
fn fields(string line) -> []string
```

### `parse_rows`

```yashy
fn parse_rows(string text) -> []string
```

### `record`

```yashy
fn record([]string fields) -> Result[string, CsvError]
```

### `write`

```yashy
fn write([]string rows) -> string
```

### `write_crlf`

```yashy
fn write_crlf([]string rows) -> string
```

## `datetime`

### `StringError`

```yashy
enum StringError
```

### `add_seconds`

```yashy
fn add_seconds(DateTime at, int seconds) -> DateTime
```

### `format`

```yashy
fn format(DateTime at, string format) -> string
```

### `from_unix`

```yashy
fn from_unix(int unix) -> DateTime
```

### `parse`

```yashy
fn parse(string text, string format) -> DateTime
```

### `to_unix`

```yashy
fn to_unix(DateTime at) -> int
```

### `DateTime`

```yashy
struct DateTime
```

### `Parts`

```yashy
struct Parts
```

## `env`

### `get`

```yashy
fn get(string name) -> Option[string]
```

### `name_of`

```yashy
fn name_of(string pair) -> string
```

### `set`

```yashy
fn set(string name, string value) -> Result[int, io.Error]
```

### `unset`

```yashy
fn unset(string name) -> Result[int, io.Error]
```

### `vars`

```yashy
fn vars() -> []string
```

## `fs`

### `copy`

```yashy
fn copy(string src, string dst) -> Result[int, io.Error]
```

### `create_dir`

```yashy
fn create_dir(string path) -> Result[int, io.Error]
```

### `create_dir_all`

```yashy
fn create_dir_all(string path) -> Result[int, io.Error]
```

### `exists`

```yashy
fn exists(string path) -> bool
```

### `metadata`

```yashy
fn metadata(string path) -> Result[Metadata, io.Error]
```

### `read_dir`

```yashy
fn read_dir(string path) -> Result[[]string, io.Error]
```

### `remove_dir`

```yashy
fn remove_dir(string path) -> Result[int, io.Error]
```

### `remove_file`

```yashy
fn remove_file(string path) -> Result[int, io.Error]
```

### `rename`

```yashy
fn rename(string old_path, string new_path) -> Result[int, io.Error]
```

### `Metadata`

```yashy
struct Metadata
```

## `glob`

### `matches`

```yashy
fn matches(string pattern, string path) -> bool
```

Whether `path` matches `pattern` under the glob rules in the module head.

## `hash`

### `fnv1a64`

```yashy
fn fnv1a64([]char data) -> int
```

FNV-1a 64-bit over the input bytes.

### `sha256`

```yashy
fn sha256([]char data) -> []char
```

SHA-256 over the input bytes, as 32 digest bytes.

### `xxhash64`

```yashy
fn xxhash64([]char data) -> int
```

xxHash64 over the input bytes.

## `io`

### `println_f64`

```yashy
fn println_f64(float value) -> void
```

Print `value` and a newline.

### `println_i64`

```yashy
fn println_i64(int value) -> void
```

Print `value` and a newline.

### `read_all`

```yashy
fn read_all() -> Result[string, io.Error]
```

Everything on standard input, read to end of input.

### `write_stderr`

```yashy
fn write_stderr(string text) -> void
```

Write `text` to standard error, with no added newline.

## `iter`

### `is_done`

```yashy
fn is_done[T](Option[T] next) -> bool
```

Whether `next`'s answer is the end of the sequence. An implementation reads
the protocol as "yield until `is_done`", so the test lives here rather than
being spelled out in every implementor.

## `json_scan`

### `Event`

```yashy
struct Event
```

One token and the byte range it covers.

### `JsonError`

```yashy
enum JsonError
```

Why the text is not well-formed at the byte level.

### `Limits`

```yashy
struct Limits
```

Explicit caps for a streaming read. A non-positive value is unbounded.

### `StreamError`

```yashy
enum StreamError
```

Why a streaming read could not finish.

### `Token`

```yashy
enum Token
```

A token kind in the streaming reader.

### `at`

```yashy
fn at([]Event events, int index) -> Option[Event]
```

The token at `index`, or `None` past the end. The event is copied out so
the caller does not have to retain the token slice.

### `minify`

```yashy
fn minify(string text) -> Result[string, JsonError]
```

Remove whitespace outside strings, keeping string bytes unchanged.

### `tokens`

```yashy
fn tokens(string text, Limits limits) -> Result[[]Event, StreamError]
```

Stream a document into tokens. The token list is flat: no object or array
value is built, and the caller sees each token with its byte span.

### `validate`

```yashy
fn validate(string text) -> Result[int, JsonError]
```

Check string closure and bracket balance; return the maximum depth.

## `log`

### `error`

```yashy
fn error(string message) -> void
```

Write one error-level JSON line to the log sink.

### `info`

```yashy
fn info(string message) -> void
```

Write one info-level JSON line to the log sink.

## `math`

### `abs`

```yashy
fn abs(int n) -> int
```

The magnitude of `n`. `abs(int::MIN)` has no representable answer and panics
rather than wrapping, like every other checked subtraction.

### `max`

```yashy
fn max(int left, int right) -> int
```

The larger of `left` and `right`.

### `min`

```yashy
fn min(int left, int right) -> int
```

The smaller of `left` and `right`.

### `pow`

```yashy
fn pow(int base, int exponent) -> int
```

`base` raised to a non-negative `exponent`, by repeated multiplication.
Overflow panics. A negative exponent has no integer answer in this signature
and is refused by the assertion.

### `sqrt`

```yashy
fn sqrt(float x) -> float
```

The non-negative square root of `x`, by Newton's method.

The iteration is `guess = (guess + x / guess) / 2`; starting from `x` it
converges to the nearest representable root in well under 32 steps for every
finite positive input, and squaring the answer lands within a bit of `x`.
A negative argument panics: there is no real answer, and returning NaN would
turn a programming mistake into a number that propagates.

## `metrics`

### `counter_add`

```yashy
fn counter_add(string name, int value) -> void
```

Add `value` to the counter named `name`.

### `gauge_set`

```yashy
fn gauge_set(string name, int value) -> void
```

Set the gauge named `name` to `value`.

## `mime`

### `Mime`

```yashy
struct Mime
```

A parsed media type, without parameters.

### `MimeError`

```yashy
enum MimeError
```

Why a text is not a media type.

### `essence`

```yashy
fn essence(Mime mime) -> string
```

The `type/subtype` spelling, without parameters.

### `is_json`

```yashy
fn is_json(Mime mime) -> bool
```

Whether the media type is `application/json` or `+json` suffixed.

### `is_text`

```yashy
fn is_text(Mime mime) -> bool
```

Whether the type is textual: `text/*`, or one of the text-like applications.

### `parse`

```yashy
fn parse(string text) -> Result[Mime, MimeError]
```

Parse `type/subtype`, ignoring anything after a semicolon.

Both halves must be non-empty. A string with no `/`, more than one `/`, or
an empty kind or subtype is `Err(Invalid)`.

## `observability`

### `Level`

```yashy
enum Level
```

The three event levels as stable spellings.

### `emit`

```yashy
fn emit(fn(string) -> void writer, string level, string message)
```

Send an event to the caller-supplied writer.

### `emit_stderr`

```yashy
fn emit_stderr(string level, string message)
```

Emit an event to the default stderr sink.

### `event_json`

```yashy
fn event_json(string level, string message) -> string
```

One event as a stable JSON line, with both fields quoted.

### `level_text`

```yashy
fn level_text(Level level) -> string
```

The spelling a level uses in a JSON event.

### `metric`

```yashy
fn metric(fn(string) -> void writer, string name, string kind, string value, string labels)
```

Send one metric sample to the caller-supplied writer.

### `metric_json`

```yashy
fn metric_json(string name, string kind, string value, string labels) -> string
```

A metric sample as a stable JSON line. `value` and `labels` are text so the
shape works for counters, gauges and histograms without a numeric surprise.

### `stderr_sink`

```yashy
fn stderr_sink(string line)
```

The default sink: one structured line to stderr.

It exists so a caller that has no writer can still use the same record
shapes. There is no global logger: the function is passed explicitly, and a
test can pass its own writer instead.

### `trace`

```yashy
fn trace(fn(string) -> void writer, string trace_id, string span_id, string parent_id, string operation, int duration_us)
```

Send one trace span to the caller-supplied writer.

### `trace_json`

```yashy
fn trace_json(string trace_id, string span_id, string parent_id, string operation, int duration_us) -> string
```

A trace span as a stable JSON line. `parent_id` is empty for a root span.
The duration is microseconds; the shape is fixed so a sink does not have to
guess which field is which.

## `os`

### `Arch`

```yashy
enum Arch
```

### `Platform`

```yashy
enum Platform
```

### `arch`

```yashy
fn arch() -> Arch
```

The CPU architecture this executable is running on.

### `pid`

```yashy
fn pid() -> int
```

This process's id.

### `platform`

```yashy
fn platform() -> Platform
```

The operating system this executable is running on.

## `path`

### `absolute`

```yashy
fn absolute(string path) -> Result[string, io.Error]
```

`path` made absolute against the current directory, lexically normalized.
A `getcwd()` failure is the caller's `Err`, never an invented path.

### `basename`

```yashy
fn basename(string path) -> string
```

The last component of `path`. `basename("/")` is `/` and `basename("")` is
the empty string.

### `dirname`

```yashy
fn dirname(string path) -> string
```

Everything before the last separator. A bare name has no directory, which is
spelled `.`; a path that is only separators has `/`.

### `extension`

```yashy
fn extension(string path) -> string
```

The text after the last `.` in the last component, without the dot. A name
with no dot, or a dot that is the first byte (`.gitignore`), has no
extension and answers the empty string.

### `file_stem`

```yashy
fn file_stem(string path) -> string
```

The last component of `path` without its extension: `notes.tar.gz` is
`notes.tar`, and a name with no extension is itself.

### `is_absolute`

```yashy
fn is_absolute(string path) -> bool
```

Whether `path` is absolute: it starts with `/`, or with a drive letter and
`:` (`C:/x`, `C:\x`).

### `join`

```yashy
fn join(string left, string right) -> string
```

One `/` between the components. A trailing separator on `left` and a leading
one on `right` are removed first, so `join("a/", "/b")` is `a/b` and not
`a//b`. An empty side contributes nothing.

### `normalize`

```yashy
fn normalize(string path) -> string
```

`path` with `.` removed and `..` folded into the component before it,
lexically. A relative path that starts with `..` keeps it.

### `parent`

```yashy
fn parent(string path) -> Option[string]
```

The directory above `path`, or `None` when there is no separator. The root
`/` and a drive root (`C:/`) answer themselves.

## `process`

### `StringError`

```yashy
enum StringError
```

### `run`

```yashy
fn run([]string argv) -> Result[Child, io.Error]
```

Run `argv` in this process's directory and environment.

### `run_in`

```yashy
fn run_in([]string argv, string cwd) -> Result[Child, io.Error]
```

Run `argv` in `cwd`.

### `run_with`

```yashy
fn run_with([]string argv, []string env, string cwd) -> Result[Child, io.Error]
```

Run `argv` in `cwd` with `env` (`NAME=VALUE` records).

### `Child`

```yashy
struct Child
```

## `rand`

### `fill`

```yashy
fn fill(int count) -> []char
```

`count` random bytes.

### `range`

```yashy
fn range(int lo, int hi) -> int
```

A uniform value in `lo..hi` (lower bound included, upper excluded).

### `shuffle`

```yashy
fn shuffle([]char data) -> void
```

Shuffle `data` in place with Fisher-Yates: every permutation is equally
likely.

### `u64`

```yashy
fn u64() -> int
```

One 64-bit draw from the OS entropy source.

## `regex`

### `RegexError`

```yashy
enum RegexError
```

### `compile`

```yashy
fn compile(string pattern) -> Result[Regex, RegexError]
```

### `find`

```yashy
fn find(Regex rx, string text) -> Option[Match]
```

### `find_all`

```yashy
fn find_all(Regex rx, string text) -> []string
```

### `is_match`

```yashy
fn is_match(Regex rx, string text) -> bool
```

### `replace_all`

```yashy
fn replace_all(Regex rx, string text, string replacement) -> string
```

### `Match`

```yashy
struct Match
```

### `Regex`

```yashy
struct Regex
```

## `semver`

### `Semver`

```yashy
struct Semver
```

A parsed version. `pre` and `build` are empty when absent.

### `SemverError`

```yashy
enum SemverError
```

Why a text is not a version.

### `compare`

```yashy
fn compare(Semver a, Semver b) -> int
```

Compare two versions: negative when `a` is older, positive when newer.

### `parse`

```yashy
fn parse(string text) -> Result[Semver, SemverError]
```

Parse `major.minor.patch[-pre][+build]`.

### `to_string`

```yashy
fn to_string(Semver version) -> string
```

The full spelling, with build metadata but no normalization.

## `string`

### `StringError`

```yashy
enum StringError
```

### `char_at`

```yashy
fn char_at(string text, int index) -> Option[string]
```

The grapheme at `index`, or `None` outside `0..graphemes()`.

### `from_bytes`

```yashy
fn from_bytes([]char data) -> Result[string, StringError]
```

The string for `data`, or `Err(InvalidUtf8)` when `data` is not valid
UTF-8: the scan is over the bytes of the input, so a dangling continuation
or an overlong form is the error, not a silent substitution.

### `join_list`

```yashy
fn join_list([]string parts, string sep) -> string
```

Join `parts` with `sep` between them.

### `pad_end`

```yashy
fn pad_end(string text, int width, string pad) -> string
```

Pad on the right to `width` with `pad` characters.

### `pad_start`

```yashy
fn pad_start(string text, int width, string pad) -> string
```

Pad on the left to `width` with `pad` characters.

### `to_bytes`

```yashy
fn to_bytes(string text) -> []char
```

The UTF-8 bytes of a string as `[]char`.

## `term`

### `bold`

```yashy
fn bold() -> string
```

Bold, `""` when disabled.

### `clear_screen`

```yashy
fn clear_screen() -> string
```

Clear the screen (home the cursor afterwards), `""` when disabled.

### `color`

```yashy
fn color(string name) -> string
```

Foreground SGR for `name` (`black red green yellow blue magenta cyan
white`), or `""` for an unknown color or when disabled.

### `is_tty`

```yashy
fn is_tty() -> bool
```

Whether standard output is a terminal.

### `move_to`

```yashy
fn move_to(int row, int column) -> string
```

Move to the 1-based row and column, `""` when disabled.

### `reset`

```yashy
fn reset() -> string
```

Reset, `""` when disabled.

## `text`

### `TextError`

```yashy
enum TextError
```

### `base64_decode`

```yashy
fn base64_decode(string text) -> Result[[]char, TextError]
```

Base64 text to bytes; a bad symbol is `Err(Invalid)`, not a dropped byte.

### `base64_encode`

```yashy
fn base64_encode([]char data) -> string
```

Bytes to base64 text.

### `base64url_decode`

```yashy
fn base64url_decode(string text) -> Result[[]char, TextError]
```

Decode unpadded or padded base64url text.

### `base64url_encode`

```yashy
fn base64url_encode([]char data) -> string
```

Bytes to unpadded base64url text (RFC 4648 section 5).

### `hex_decode`

```yashy
fn hex_decode(string text) -> Result[[]char, TextError]
```

Hex text to bytes; an odd length or a bad digit is `Err(Invalid)`.

### `hex_encode`

```yashy
fn hex_encode([]char data) -> string
```

Bytes to lowercase hex text.

## `time`

### `monotonic_ns`

```yashy
fn monotonic_ns() -> int
```

Nanoseconds from an unspecified origin, never decreasing.

### `now_unix_ns`

```yashy
fn now_unix_ns() -> int
```

Nanoseconds since the Unix epoch, from the wall clock.

## `url`

### `Url`

```yashy
struct Url
```

A parsed URL: scheme, host, port (0 when absent) and path.

### `UrlError`

```yashy
enum UrlError
```

Why a text is not an absolute hierarchical URL.

### `origin`

```yashy
fn origin(Url url) -> string
```

The `scheme://host` prefix, without port or path.

### `parse`

```yashy
fn parse(string text) -> Result[Url, UrlError]
```

Parse `scheme://host[:port]/path`.

A missing scheme separator or an empty scheme or host is `Err(Invalid)`.
A port that is not a decimal integer is also refused.

### `percent_decode`

```yashy
fn percent_decode(string text) -> Result[string, UrlError]
```

Decode `%XX` escapes in a URL component.

A `%` not followed by two hexadecimal digits is `Err(Invalid)`; a valid
escape produces exactly one byte. The `+` spelling is left alone: it is a
literal plus in a path and a space only in form bodies, which this module
does not parse.

## `uuid`

### `from_bytes`

```yashy
fn from_bytes([]char bytes) -> Result[string, text.TextError]
```

Format sixteen bytes as canonical lowercase UUID text.

### `is_valid`

```yashy
fn is_valid(string value) -> bool
```

Whether `value` is a canonical lowercase UUID text.

### `nil`

```yashy
fn nil() -> string
```

The nil UUID, all zeroes.

### `v4`

```yashy
fn v4() -> Result[string, text.TextError]
```

Generate a version-4 UUID from sixteen OS-random bytes.

The version nibble is set to `4` and the variant bits to `10`, so the text
this produces is a valid RFC 4122 UUID. The entropy comes from `rand.fill`,
which reads the OS source; a deterministic build fixes that source at the
process edge.

