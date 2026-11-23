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

### `global_alloc_value`

```yashy
fn global_alloc_value() -> GlobalAllocator
```

A fresh process-wide allocator value. A collection that takes an allocator
can be given this one to use the global path. A program that wants to record
calls defines its own type and `impl Allocator for` it, then calls `alloc`
/ `free` on a local binding of that type — dispatch is static, so each call
lowers to the concrete type's `Allocator` symbol with no vtable.

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

### `GlobalAllocator`

```yashy
struct GlobalAllocator
```

The process-wide allocator as a value. Its methods reach the same runtime
path as `global_alloc` / `global_free`, so a collection given this value
allocates exactly where a bare `alloc` call does.

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

## `casing`

### `eq_ignore_ascii_case`

```yashy
fn eq_ignore_ascii_case(string a, string b) -> bool
```

Compare two strings by bytes, folding ASCII letters only.

Lengths must be equal and each byte either identical or an ASCII letter
pair differing only in case. Non-ASCII bytes are compared as-is, so this is
explicitly not Unicode case folding.

### `is_ascii_alpha`

```yashy
fn is_ascii_alpha(char value) -> bool
```

Whether `value` is an ASCII letter of either case.

### `is_ascii_lower`

```yashy
fn is_ascii_lower(char value) -> bool
```

Whether `value` is an ASCII lowercase letter `a`..`z`.

### `is_ascii_upper`

```yashy
fn is_ascii_upper(char value) -> bool
```

Whether `value` is an ASCII uppercase letter `A`..`Z`.

### `to_ascii_lower`

```yashy
fn to_ascii_lower(string text) -> string
```

Map `A`..`Z` to lowercase, leaving every other byte unchanged.

### `to_ascii_upper`

```yashy
fn to_ascii_upper(string text) -> string
```

Map `a`..`z` to uppercase, leaving every other byte unchanged.

## `collections`

### `deque_back`

```yashy
fn deque_back[T]([]T items) -> QPop[T]
```

### `deque_front`

```yashy
fn deque_front[T]([]T items) -> QPop[T]
```

### `deque_new`

```yashy
fn deque_new[T]() -> Deque[T]
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

### `map_new`

```yashy
fn map_new[K, V]() -> Map[K, V]
```

### `pq_new`

```yashy
fn pq_new[T]() -> PriorityQueue[T]
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

### `set_new`

```yashy
fn set_new[T]() -> Set[T]
```

### `set_remove`

```yashy
fn set_remove[T]([]T items, T value) -> []T
```

### `vec_new`

```yashy
fn vec_new[T]() -> Vec[T]
```

### `Chunks`

```yashy
struct Chunks
```

### `Windows`

```yashy
struct Windows
```

## `convert`

### `ConvertError`

```yashy
enum ConvertError
```

Why a conversion could not produce a value.

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

### `try_from_bool`

```yashy
fn try_from_bool(string text) -> Result[bool, ConvertError]
```

Parse a boolean with the `try_from` spelling.

### `try_from_float`

```yashy
fn try_from_float(float value) -> Result[int, ConvertError]
```

A checked float-to-int conversion with a documented range.

Values outside `trunc(int)`'s decimal range answer `Overflow` rather than
panicking or wrapping.

### `try_from_int`

```yashy
fn try_from_int(string text) -> Result[int, ConvertError]
```

Parse a decimal integer with the `try_from` spelling.

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

### `DateTimeError`

```yashy
enum DateTimeError
```

### `StringError`

```yashy
enum StringError
```

### `add_days`

```yashy
fn add_days(DateTime at, int days) -> DateTime
```

Add whole civil days. Crossing a leap day is ordinary calendar arithmetic,
not an extra rule: the day count is converted to seconds at fixed length.

### `add_seconds`

```yashy
fn add_seconds(DateTime at, int seconds) -> DateTime
```

### `format`

```yashy
fn format(DateTime at, string format) -> string
```

### `from_rfc3339`

```yashy
fn from_rfc3339(string text) -> Result[DateTime, DateTimeError]
```

Parse RFC 3339 seconds precision. Both `Z` and `±HH:MM` offsets are
accepted; malformed text answers `Invalid` rather than guessing a date.

### `from_unix`

```yashy
fn from_unix(int unix) -> DateTime
```

### `parse`

```yashy
fn parse(string text, string format) -> DateTime
```

### `to_rfc3339`

```yashy
fn to_rfc3339(DateTime at) -> string
```

A timestamp in RFC 3339 seconds precision: `Z` for UTC, otherwise a
`±HH:MM` offset. There is no fractional part because the storage is whole
seconds.

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

## `format`

### `FormatError`

```yashy
enum FormatError
```

Why a dynamic format specifier could not be rendered.

### `describe_format`

```yashy
fn describe_format(FormatError error) -> string
```

A human-readable rendering of a format error.

### `fixed`

```yashy
fn fixed(float value, int precision) -> string
```

Render `value` with exactly `precision` digits after the decimal point.

### `format_dynamic`

```yashy
fn format_dynamic(string spec, []string args) -> Result[string, FormatError]
```

Render a dynamic specifier against already-stringified `args`.

A malformed spec answers `Err(FormatError.Invalid(offset))`; a wrong
argument count answers `Err(MissingArg)`/`Err(ExtraArg)`. Otherwise the
fields are substituted with alignment, width and precision, `{{`/`}}`
become literal braces, and the pieces are joined.

### `pad_center`

```yashy
fn pad_center(string text, int width, string fill) -> string
```

Centre `text` in `width` display cells with `fill`.

### `pad_left`

```yashy
fn pad_left(string text, int width, string fill) -> string
```

Pad `text` on the left to `width` display cells with `fill`.

### `pad_right`

```yashy
fn pad_right(string text, int width, string fill) -> string
```

Pad `text` on the right to `width` display cells with `fill`.

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

### `glob_matches`

```yashy
fn glob_matches(string pattern, string path) -> bool
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

## `iter_adapters`

### `Chunks.Iterator.next`

```yashy
fn next(*mut Chunks self) -> Option[(int, int)]
```

### `Windows.Iterator.next`

```yashy
fn next(*mut Windows self) -> Option[(int, int)]
```

### `chain`

```yashy
fn chain[A]([]A left, []A right, fn(A) -> void visit) -> PairSeq[A, A]
```

Call `visit(value)` for every element of `left` then every element of
`right`, preserving each sequence's order. Returns both slices.

### `chain_of`

```yashy
fn chain_of[T]([]T left, []T right) -> Chain[T]
```

### `chunks`

```yashy
fn chunks[T]([]T items, int size, fn(int, int) -> void visit) -> []T
```

Call `visit(start, end)` for each non-overlapping window of `size`
consecutive elements. `start` is included and `end` excluded; a trailing
partial window is still visited, so every element is covered exactly once.
A non-positive `size` is an assertion failure rather than an infinite loop.

### `chunks_of`

```yashy
fn chunks_of(int len, int size) -> Chunks
```

### `enumerate`

```yashy
fn enumerate[T]([]T items, fn(int, T) -> void visit) -> []T
```

Call `visit(index, value)` for every element of `items`, starting at 0.
The index is never negative. Returns `items` so the caller keeps ownership.

### `enumerate_of`

```yashy
fn enumerate_of[T]([]T values) -> Enumerate[T]
```

### `rev`

```yashy
fn rev[T]([]T items, fn(T) -> void visit) -> []T
```

Call `visit(value)` for every element from last to first.

### `rev_of`

```yashy
fn rev_of[T]([]T values) -> Rev[T]
```

### `skip`

```yashy
fn skip[T]([]T items, int count, fn(T) -> void visit) -> []T
```

Call `visit(value)` for every element after the first `count`. A non-positive
`count` visits all elements; a `count` past the end visits nothing.

### `skip_of`

```yashy
fn skip_of[T]([]T values, int count) -> Skip[T]
```

### `take`

```yashy
fn take[T]([]T items, int count, fn(T) -> void visit) -> []T
```

Call `visit(value)` for the first `count` elements, or all of them when
`count` is larger. A non-positive `count` visits nothing.

### `take_of`

```yashy
fn take_of[T]([]T values, int count) -> Take[T]
```

### `windows`

```yashy
fn windows[T]([]T items, int size, fn(int, int) -> void visit) -> []T
```

Call `visit(start, end)` for each sliding window of `size` consecutive
elements, left to right. `start` is included and `end` excluded. A `size`
larger than the sequence visits nothing; a non-positive `size` is an
assertion failure.

### `windows_of`

```yashy
fn windows_of(int len, int size) -> Windows
```

### `zip`

```yashy
fn zip[A, B]([]A left, []B right, fn(A, B) -> void visit) -> PairSeq[A, B]
```

Call `visit(left, right)` for the paired elements of `left` and `right`,
stopping at the shorter input and never reading past either. Returns both.

### `zip_of`

```yashy
fn zip_of[A, B]([]A left, []B right) -> Zip[A, B]
```

### `Chunks`

```yashy
struct Chunks
```

### `Windows`

```yashy
struct Windows
```

## `json_scan`

### `JsonError`

```yashy
enum JsonError
```

Why the text is not well-formed at the byte level.

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

### `TokenList.free`

```yashy
fn free(TokenList self) -> void
```

### `TokenList.len`

```yashy
fn len(TokenList self) -> int
```

### `at`

```yashy
fn at(TokenList events, int index) -> Option[Event]
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
fn tokens(string text, Limits limits) -> Result[TokenList, StreamError]
```

Stream a document into tokens. The token list is flat: no object or array
value is built, and the caller sees each token with its byte span.

### `validate`

```yashy
fn validate(string text) -> Result[int, JsonError]
```

Check string closure and bracket balance; return the maximum depth.

### `Event`

```yashy
struct Event
```

One token and the byte range it covers.

### `Limits`

```yashy
struct Limits
```

Explicit caps for a streaming read. A non-positive value is unbounded.

### `TokenList`

```yashy
struct TokenList
```

A flat token list: one row per token, in source order.

## `json_value`

### `DocError`

```yashy
enum DocError
```

Why a document could not be parsed.

### `bool_value`

```yashy
fn bool_value(JsonDoc doc, int node) -> Option[bool]
```

The bool at `node`, or `None` when the node is not a bool.

### `count`

```yashy
fn count(JsonDoc doc) -> int
```

The number of nodes in the document.

### `describe`

```yashy
fn describe(DocError error) -> string
```

A human-readable rendering of a parse error with its position.

### `emit_doc`

```yashy
fn emit_doc(JsonDoc doc) -> string
```

Emit a compact JSON document: insertion-ordered keys, no trailing space.

### `first_child`

```yashy
fn first_child(JsonDoc doc, int node) -> int
```

The index of the first child of `node`, or `-1` when it has none.

### `float_value`

```yashy
fn float_value(JsonDoc doc, int node) -> Option[float]
```

The float at `node`, or `None` when the node is not a float.

### `free_doc`

```yashy
fn free_doc(JsonDoc doc) -> void
```

Release every column. The caller must not use the document afterwards.

### `get_doc`

```yashy
fn get_doc(JsonDoc doc, int node, string wanted) -> Option[int]
```

Find `wanted` in the object `node`, returning the child's index. The first
match wins when a document repeats a key; both rows stay in the tree and
both are emitted, in source order.

### `int_value`

```yashy
fn int_value(JsonDoc doc, int node) -> Option[int]
```

The int at `node`, or `None` when the node is not an int.

### `is_scalar`

```yashy
fn is_scalar(JsonDoc doc, int node) -> bool
```

Whether `node` has a value a caller can read directly (not a container).

### `key`

```yashy
fn key(JsonDoc doc, int index) -> string
```

The key a node carries in its parent object, or `""` for an array element
or the root.

### `kind`

```yashy
fn kind(JsonDoc doc, int index) -> int
```

The kind code of a node: 0 null, 1 bool, 2 int, 3 float, 4 string,
5 array, 6 object. `-1` is past the end.

### `next_child`

```yashy
fn next_child(JsonDoc doc, int node, int after) -> int
```

The index of the next child of `node` after `after`, or `-1` at the end.
Children are rows in document order, so walking with this function yields
insertion order. Pass `-1` for `after` to start.

### `parent_doc`

```yashy
fn parent_doc(JsonDoc doc, int index) -> int
```

The parent index of a node, or `-1` for the root or a past-end index.

### `parse_doc`

```yashy
fn parse_doc(string text) -> Result[JsonDoc, DocError]
```

Parse a document. A malformed input answers `Err` with a byte offset.

### `parse_with_doc`

```yashy
fn parse_with_doc(string text, DocLimits limits) -> Result[JsonDoc, DocError]
```

Parse with explicit depth and byte caps. Validation runs first, so the
building pass below has no failure paths and no linear value is ever freed
conditionally.

### `value`

```yashy
fn value(JsonDoc doc, int index) -> Option[string]
```

The scalar text of a node, or `None` for a container or a past-end index.

### `write_stream`

```yashy
fn write_stream(JsonDoc doc, fn(string) -> void sink) -> void
```

Write a document to `sink` one piece at a time, without building the whole
text first. The callback receives structural characters, keys and scalar
text in document order; concatenating the pieces gives exactly `emit_doc`'s
answer. This is the streaming counterpart to the token reader in
`json_scan`: a caller that cannot hold the whole output can consume it as
it is produced.

### `DocLimits`

```yashy
struct DocLimits
```

Explicit caps for a parse. A non-positive value is unbounded.

### `JsonDoc`

```yashy
struct JsonDoc
```

A parsed document: one row per node across four columns.

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

### `MimeError`

```yashy
enum MimeError
```

Why a text is not a media type. The `int` is the byte offset of the first
problem in the input.

### `describe_mime`

```yashy
fn describe_mime(MimeError error) -> string
```

A human-readable rendering of a media-type error with its byte offset.

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

### `matches_media`

```yashy
fn matches_media(Mime mime, string text, string pattern) -> bool
```

Whether a parsed media type matches `pattern`. The pattern is
`type/subtype` and either half may be `*`. A parameter `key=value` in the
pattern must be present with that exact value in `text`; a bare `key` in
the pattern requires the parameter to be present.

### `param`

```yashy
fn param(string text, string key) -> Option[string]
```

The value of parameter `key` in a media type text, or `None` when absent.
Parameter values are trimmed; quotes around a value are kept as written.

### `parse_media_type`

```yashy
fn parse_media_type(string text) -> Result[Mime, MimeError]
```

Parse `type/subtype`, ignoring anything after a semicolon.

Both halves must be non-empty. A string with no `/`, more than one `/`, or
an empty kind or subtype is `Err(Invalid)`.

### `Mime`

```yashy
struct Mime
```

A parsed media type, without parameters.

## `normalize`

### `eq_ignore_case`

```yashy
fn eq_ignore_case(string a, string b) -> bool
```

Whether two strings are equal after full case folding.

### `fold`

```yashy
fn fold(string text) -> string
```

Full Unicode case folding, independent of locale.

### `nfc`

```yashy
fn nfc(string text) -> string
```

NFC: canonical decomposition followed by canonical composition.

### `nfd`

```yashy
fn nfd(string text) -> string
```

NFD: the full canonical decomposition.

### `nfkc`

```yashy
fn nfkc(string text) -> string
```

NFKC: compatibility decomposition followed by canonical composition.

### `nfkd`

```yashy
fn nfkd(string text) -> string
```

NFKD: the full compatibility decomposition.

## `observability`

### `Level`

```yashy
enum Level
```

The three event levels as stable spellings.

### `emit`

```yashy
fn emit(fn(string) -> void writer, string level, string message) -> void
```

Send an event to the caller-supplied writer.

### `emit_stderr`

```yashy
fn emit_stderr(string level, string message) -> void
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
fn metric(fn(string) -> void writer, string name, string kind, string value, string labels) -> void
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
fn stderr_sink(string line) -> void
```

The default sink: one structured line to stderr.

It exists so a caller that has no writer can still use the same record
shapes. There is no global logger: the function is passed explicitly, and a
test can pass its own writer instead.

### `trace`

```yashy
fn trace(fn(string) -> void writer, string trace_id, string span_id, string parent_id, string operation, int duration_us) -> void
```

Send one trace span to the caller-supplied writer.

### `trace_json`

```yashy
fn trace_json(string trace_id, string span_id, string parent_id, string operation, int duration_us) -> string
```

A trace span as a stable JSON line. `parent_id` is empty for a root span.
The duration is microseconds; the shape is fixed so a sink does not have to
guess which field is which.

## `ordered_map`

### `ordered_free`

```yashy
fn ordered_free(OMap map) -> void
```

Release every column. The map must not be used afterwards.

### `ordered_get`

```yashy
fn ordered_get(OMap map, string key) -> OLookup
```

Look up `key`. The map is returned in `entries` so the caller keeps it.

### `ordered_key`

```yashy
fn ordered_key(OMap map, int index) -> Option[string]
```

The live key at row `index` (skipping removed rows), or `None` past the end.

### `ordered_len`

```yashy
fn ordered_len(OMap map) -> int
```

The number of live entries.

### `ordered_new`

```yashy
fn ordered_new() -> OMap
```

A fresh, empty map.

### `ordered_remove`

```yashy
fn ordered_remove(OMap map, string key) -> OMap
```

Remove `key`, returning the map. Removal keeps the order of the rest.

### `ordered_set`

```yashy
fn ordered_set(OMap map, string key, string new_value) -> OMap
```

Insert `key` with `value`, or overwrite an existing key in place (keeping
its position). A new key is appended at the end.

### `ordered_touch`

```yashy
fn ordered_touch(OMap map, string key, string value) -> OMap
```

Remove `key` then re-insert it, moving it to the end.

### `ordered_value`

```yashy
fn ordered_value(OMap map, int index) -> Option[string]
```

The live value at row `index`, or `None` past the end.

### `OLookup`

```yashy
struct OLookup
```

A lookup answer: the map plus the value found, if any.

### `OMap`

```yashy
struct OMap
```

An insertion-ordered map: append-only columns and a live flag per key row.

## `os`

### `Arch`

```yashy
enum Arch
```

The host CPU architecture.

### `Platform`

```yashy
enum Platform
```

The host operating system.

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

A child that has already exited.

## `rand`

### `CsprngError`

```yashy
enum CsprngError
```

Why a CSPRNG request could not be satisfied.

### `csprng_bytes`

```yashy
fn csprng_bytes(int count) -> Result[[]char, CsprngError]
```

`count` fresh bytes from the one entropy source, or a failure value.

This is the `Result` spelling of `fill`: `0` bytes is a valid empty answer,
so it cannot also mean "failed". Under `--deterministic` the bytes come from
the same fixed seed as every other draw, and `deterministic()` says so.
The caller owns the returned array and releases it with `.free()`.

### `deterministic`

```yashy
fn deterministic() -> bool
```

Whether the program runs under `--deterministic`, so its draws come from a
fixed seed instead of the OS entropy source.

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

### `SemverError`

```yashy
enum SemverError
```

Why a text is not a version. The `int` is the byte offset of the first
problem in the input.

### `compare`

```yashy
fn compare(Semver a, Semver b) -> int
```

Compare two versions: negative when `a` is older, positive when newer.

### `describe_semver`

```yashy
fn describe_semver(SemverError error) -> string
```

A human-readable rendering of a version error with its byte offset.

### `parse_version`

```yashy
fn parse_version(string text) -> Result[Semver, SemverError]
```

Parse `major.minor.patch[-pre][+build]`.

### `to_string`

```yashy
fn to_string(Semver version) -> string
```

The full spelling, with build metadata but no normalization.

### `Semver`

```yashy
struct Semver
```

A parsed version. `pre` and `build` are empty when absent.

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

### `StringError`

```yashy
enum StringError
```

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

### `describe_text`

```yashy
fn describe_text(TextError error) -> string
```

A human-readable rendering of a codec error with its byte offset.

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

## `toml`

### `TomlError`

```yashy
enum TomlError
```

Why a document is not valid TOML. The `int` is the byte offset of the
first problem.

### `count_toml`

```yashy
fn count_toml(TomlDoc doc) -> int
```

The number of keys in the document.

### `describe_toml`

```yashy
fn describe_toml(TomlError error) -> string
```

A human-readable rendering of a TOML error with its byte offset.

### `emit_toml`

```yashy
fn emit_toml(TomlDoc doc) -> string
```

Emit a canonical document: sections in first-seen order, keys in insertion
order, one `key = value` per line, no trailing space.

### `free_toml`

```yashy
fn free_toml(TomlDoc doc) -> void
```

Release every column. The document must not be used afterwards.

### `get_toml`

```yashy
fn get_toml(TomlDoc doc, string section, string key) -> Option[int]
```

Look up `key` in `section`, returning the row index. The first match wins.

### `key_of_toml`

```yashy
fn key_of_toml(TomlDoc doc, int index) -> string
```

The name of a key.

### `kind_of_toml`

```yashy
fn kind_of_toml(TomlDoc doc, int index) -> int
```

A value kind: 1 string, 2 int, 3 float, 4 bool, 5 array.

### `parse_toml`

```yashy
fn parse_toml(string text) -> Result[TomlDoc, TomlError]
```

Parse a TOML document. A malformed input answers `Err` with a byte offset.
Validation runs first, so the building pass below has no failure paths and
no owned column is freed on a path that later uses it.

### `section_of_toml`

```yashy
fn section_of_toml(TomlDoc doc, int index) -> string
```

The section a key belongs to (empty for the root section).

### `value_of_toml`

```yashy
fn value_of_toml(TomlDoc doc, int index) -> Option[string]
```

The value text of a key, or `None` past the end.

### `TomlDoc`

```yashy
struct TomlDoc
```

A parsed document: one row per key across four owned columns.

## `url`

### `UrlError`

```yashy
enum UrlError
```

Why a text is not a well-formed URL. The `int` is the byte offset in the
input where the problem begins.

### `describe_url`

```yashy
fn describe_url(UrlError error) -> string
```

A human-readable rendering of a URL error with its byte offset.

### `origin`

```yashy
fn origin(Url url) -> string
```

The `scheme://host` prefix, without port or path.

### `parse_url`

```yashy
fn parse_url(string text) -> Result[Url, UrlError]
```

Parse `scheme://[userinfo@]host[:port]/path[?query][#fragment]`.

A missing scheme separator or an empty scheme or host is `Err(Invalid)`.
A port that is not a decimal integer in `0..=65535` is also refused. A
bracketed host may contain colons (IPv6); the port must follow `]`.

### `percent_decode`

```yashy
fn percent_decode(string text) -> Result[string, UrlError]
```

Decode `%XX` escapes in a URL component.

A `%` not followed by two hexadecimal digits is `Err(Invalid)`; a valid
escape produces exactly one byte. The `+` spelling is left alone: it is a
literal plus in a path and a space only in form bodies, which this module
does not parse.

### `resolve`

```yashy
fn resolve(string base, string reference) -> Result[string, UrlError]
```

Resolve a URI reference against a base URL (RFC 3986 §5.2).

Handles all five reference forms: an absolute URI, a network-path
reference (`//host/path`), an absolute-path reference (`/path`), a
relative-path reference (`a/b`), and a query- or fragment-only reference.
Dot segments (`.` and `..`) in the merged path are removed. The answer is a
URL string, always absolute; a malformed base is `Err(Invalid)`.

### `Url`

```yashy
struct Url
```

A parsed URL: its scheme, authority parts, path, query and fragment.
`userinfo`, `query` and `fragment` are empty when absent; `port` is 0 when
absent. Bracketed IPv6 hosts keep their brackets in `host`.

## `uuid`

### `CsprngError`

```yashy
enum CsprngError
```

### `UuidError`

```yashy
enum UuidError
```

Why a UUID could not be built from bytes.

### `format_bytes`

```yashy
fn format_bytes([]char bytes) -> Result[string, UuidError]
```

Format sixteen bytes as canonical lowercase UUID text.

The byte array is consumed: like `csv.record`, this function hands it to a
helper that releases it on both the success and the error path.

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
fn v4() -> Result[string, UuidError]
```

Generate a version-4 UUID from sixteen OS-random bytes.

The version nibble is set to `4` and the variant bits to `10`, so the text
this produces is a valid RFC 4122 UUID. The entropy comes from `rand.fill`,
which reads the OS source; a deterministic build fixes that source at the
process edge.

### `v7`

```yashy
fn v7() -> Result[string, UuidError]
```

Generate a version-7 UUID (RFC 9562) from the current wall clock.

The first 48 bits are the Unix timestamp in milliseconds, so v7 values sort
by creation time. Byte 6 carries version `7`, byte 8 carries variant `10`,
and the remaining 74 bits are OS-random. The clock is `time.now_unix_ns()`,
which a deterministic build fixes at the process edge.

## `width`

### `is_narrow`

```yashy
fn is_narrow(string text) -> bool
```

Whether `text` occupies as many cells as it has grapheme clusters.

A combining mark makes this false: it is a grapheme but no cell.

### `width`

```yashy
fn width(string text) -> int
```

The width of `text` in terminal cells.

## `xml`

### `XmlError`

```yashy
enum XmlError
```

Why a document is not well-formed XML. The `int` is a byte offset.

### `count_xml`

```yashy
fn count_xml(XmlDoc doc) -> int
```

The number of elements in the document.

### `decode_entities`

```yashy
fn decode_entities(string text) -> string
```

Decode the five predefined entities and numeric character references in
`text`. Unknown entities are left as written so a caller can handle them.

### `depth_of_xml`

```yashy
fn depth_of_xml(XmlDoc doc, int index) -> int
```

The depth of an element.

### `describe_xml`

```yashy
fn describe_xml(XmlError error) -> string
```

A human-readable rendering of an XML error with its byte offset.

### `emit_xml`

```yashy
fn emit_xml(XmlDoc doc) -> string
```

Emit a canonical document: one line per element, two spaces per depth,
attributes in source order, text between the tags. An element with children
(the next row is deeper) gets an open tag whose close is emitted when the
depth falls back; a leaf with no text is written `<name/>`.

### `find_xml`

```yashy
fn find_xml(XmlDoc doc, string wanted) -> Option[int]
```

Find the first element with the given name, returning its row index.

### `free_xml`

```yashy
fn free_xml(XmlDoc doc) -> void
```

Release every column. The document must not be used afterwards.

### `name_of_xml`

```yashy
fn name_of_xml(XmlDoc doc, int index) -> string
```

The name of an element.

### `parse_xml`

```yashy
fn parse_xml(string text) -> Result[XmlDoc, XmlError]
```

Parse an XML document. A malformed input answers `Err` with a byte offset.
Validation runs first, so the building pass below has no failure paths and
no owned column is freed on a path that later uses it.

### `root_name`

```yashy
fn root_name(XmlDoc doc) -> string
```

The root element's name, or `""` for an empty document. An empty document
is not a failure state: it is a valid document with no elements.

### `text_of_xml`

```yashy
fn text_of_xml(XmlDoc doc, int index) -> Option[string]
```

The decoded text content of an element, or `None` past the end.

### `XmlDoc`

```yashy
struct XmlDoc
```

A parsed document: one row per element across five owned columns.

## `yaml`

### `YamlError`

```yashy
enum YamlError
```

Why a document is not valid YAML. The `int` is the byte offset of the
first problem.

### `count_yaml`

```yashy
fn count_yaml(YamlDoc doc) -> int
```

The number of rows in the document.

### `describe_yaml`

```yashy
fn describe_yaml(YamlError error) -> string
```

A human-readable rendering of a YAML error with its byte offset.

### `emit_yaml`

```yashy
fn emit_yaml(YamlDoc doc) -> string
```

Emit a canonical document: two spaces per indent level, one row per line.

### `free_yaml`

```yashy
fn free_yaml(YamlDoc doc) -> void
```

Release every column. The document must not be used afterwards.

### `get_yaml`

```yashy
fn get_yaml(YamlDoc doc, string wanted) -> Option[int]
```

Look up a top-level key, returning the row index of its value.

### `indent_of_yaml`

```yashy
fn indent_of_yaml(YamlDoc doc, int index) -> int
```

The indentation level of a row.

### `key_of_yaml`

```yashy
fn key_of_yaml(YamlDoc doc, int index) -> string
```

The key of a row (empty for a sequence item).

### `kind_of_yaml`

```yashy
fn kind_of_yaml(YamlDoc doc, int index) -> int
```

A node kind: 1 mapping key with a value, 2 mapping key opening a block,
3 sequence item. `-1` is past the end.

### `parse_yaml`

```yashy
fn parse_yaml(string text) -> Result[YamlDoc, YamlError]
```

Parse a YAML document. A malformed input answers `Err` with a byte offset.
Validation runs first so the build pass below has no failure paths.

### `value_of_yaml`

```yashy
fn value_of_yaml(YamlDoc doc, int index) -> Option[string]
```

The value text of a row, or `None` past the end.

### `YamlDoc`

```yashy
struct YamlDoc
```

A parsed document: one row per line across four owned columns.

## `zone`

### `DateTimeError`

```yashy
enum DateTimeError
```

### `StringError`

```yashy
enum StringError
```

### `in_zone`

```yashy
fn in_zone(DateTime utc, string name) -> Option[DateTime]
```

### `zone_offset`

```yashy
fn zone_offset(string name, int unix) -> Option[int]
```

### `DateTime`

```yashy
struct DateTime
```

### `Parts`

```yashy
struct Parts
```
