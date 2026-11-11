# The language reference

Generated from the standard library by `scripts/book-reference.sh`.
Every signature here is the one the compiler checks a program against;
the CI job regenerates the page and fails when it differs.

The standard library is documented in [the reference page](stdlib.md).

## The standard library

### `JSON`

- `fn escape(string text) -> string`
- `fn quote(string text) -> string`
### `alloc`

- `fn alloc_one(int value) -> *mut int`
- `fn alloc_zeroed(int count) -> *mut int`
### `args`

- `fn all() -> []string`
- `fn flag(string name) -> bool`
- `fn nth(int index) -> Option[string]`
### `collections`

- `fn deque_back[T]([]T items) -> QPop[T]`
- `fn deque_front[T]([]T items) -> QPop[T]`
- `fn deque_pop_back[T]([]T items) -> QPop[T]`
- `fn deque_pop_front[T]([]T items) -> QPop[T]`
- `fn deque_push_back[T]([]T items, T value) -> []T`
- `fn deque_push_front[T]([]T items, T value) -> []T`
- `fn int_sort_insert([]int items, int value) -> []int`
- `fn queue_peek[T]([]T items) -> QPop[T]`
- `fn queue_pop[T]([]T items, fn(T, T) -> int cmp) -> QPop[T]`
- `fn queue_push[T]([]T items, T value, fn(T, T) -> int cmp) -> []T`
- `fn set_contains[T]([]T items, T value) -> QHas[T]`
- `fn set_insert[T]([]T items, T value) -> []T`
- `fn set_remove[T]([]T items, T value) -> []T`
### `csv`

- `fn fields(string line) -> []string`
- `fn parse_rows(string text) -> []string`
- `fn record([]string fields) -> Result[string, CsvError]`
- `fn write([]string rows) -> string`
- `fn write_crlf([]string rows) -> string`
### `datetime`

- `fn add_seconds(DateTime at, int seconds) -> DateTime`
- `fn char_at(string text, int index) -> Option[string]`
- `fn civil_from_days(int z) -> Parts`
- `fn days_from_civil(int year, int month, int day) -> int`
- `fn format(DateTime at, string format) -> string`
- `fn from_bytes([]char data) -> Result[string, StringError]`
- `fn from_unix(int unix) -> DateTime`
- `fn join_list([]string parts, string sep) -> string`
- `fn pad_end(string text, int width, string pad) -> string`
- `fn pad_start(string text, int width, string pad) -> string`
- `fn parse(string text, string format) -> DateTime`
- `fn to_bytes(string text) -> []char`
- `fn to_unix(DateTime at) -> int`
### `env`

- `fn get(string name) -> Option[string]`
- `fn name_of(string pair) -> string`
- `fn println_f64(float value) -> void`
- `fn println_i64(int value) -> void`
- `fn read_all() -> Result[string, io.Error]`
- `fn set(string name, string value) -> Result[int, io.Error]`
- `fn unset(string name) -> Result[int, io.Error]`
- `fn vars() -> []string`
- `fn write_stderr(string text) -> void`
### `fs`

- `fn as_result(int status) -> Result[int, io.Error]`
- `fn copy(string src, string dst) -> Result[int, io.Error]`
- `fn create_dir(string path) -> Result[int, io.Error]`
- `fn create_dir_all(string path) -> Result[int, io.Error]`
- `fn exists(string path) -> bool`
- `fn field_int(string text) -> int`
- `fn metadata(string path) -> Result[Metadata, io.Error]`
- `fn println_f64(float value) -> void`
- `fn println_i64(int value) -> void`
- `fn read_all() -> Result[string, io.Error]`
- `fn read_dir(string path) -> Result[[]string, io.Error]`
- `fn record_ok(string record) -> bool`
- `fn remove_dir(string path) -> Result[int, io.Error]`
- `fn remove_file(string path) -> Result[int, io.Error]`
- `fn rename(string old_path, string new_path) -> Result[int, io.Error]`
- `fn write_stderr(string text) -> void`
### `glob`

- `fn in_class(string class, char next) -> bool`
- `fn matches(string pattern, string path) -> bool`
- `fn matches_here(string pattern, string path, int at, int start) -> bool`
- `fn same_char(char a, char b) -> bool`
### `hash`

- `fn fnv1a64([]char data) -> int`
- `fn fnv_offset() -> int`
- `fn fnv_prime() -> int`
- `fn k_at(int index) -> int`
- `fn mask32() -> int`
- `fn rotr32(int x, int n) -> int`
- `fn sha256([]char data) -> []char`
- `fn xx_lshr(int value, int n) -> int`
- `fn xx_merge_round(int seed, int lane) -> int`
- `fn xx_prime64_1() -> int`
- `fn xx_prime64_2() -> int`
- `fn xx_prime64_3() -> int`
- `fn xx_prime64_4() -> int`
- `fn xx_prime64_5() -> int`
- `fn xx_rotl(int value, int n) -> int`
- `fn xx_round(int seed, int input) -> int`
- `fn xxhash64([]char data) -> int`
### `io`

- `fn println_f64(float value) -> void`
- `fn println_i64(int value) -> void`
- `fn read_all() -> Result[string, io.Error]`
- `fn write_stderr(string text) -> void`
### `iter`

- `fn is_done[T](Option[T] next) -> bool`
### `log`

- `fn error(string message) -> void`
- `fn info(string message) -> void`
### `math`

- `fn abs(int n) -> int`
- `fn max(int left, int right) -> int`
- `fn min(int left, int right) -> int`
- `fn pow(int base, int exponent) -> int`
- `fn sqrt(float x) -> float`
### `metrics`

- `fn counter_add(string name, int value) -> void`
- `fn gauge_set(string name, int value) -> void`
### `os`

- `fn arch() -> Arch`
- `fn pid() -> int`
- `fn platform() -> Platform`
### `path`

- `fn absolute(string path) -> Result[string, io.Error]`
- `fn basename(string path) -> string`
- `fn dirname(string path) -> string`
- `fn extension(string path) -> string`
- `fn file_stem(string path) -> string`
- `fn is_absolute(string path) -> bool`
- `fn join(string left, string right) -> string`
- `fn last_dot(string text) -> int`
- `fn last_separator(string text) -> int`
- `fn normalize(string path) -> string`
- `fn parent(string path) -> Option[string]`
- `fn println_f64(float value) -> void`
- `fn println_i64(int value) -> void`
- `fn read_all() -> Result[string, io.Error]`
- `fn strip_leading(string text) -> string`
- `fn strip_trailing(string text) -> string`
- `fn write_stderr(string text) -> void`
### `process`

- `fn char_at(string text, int index) -> Option[string]`
- `fn child_from(string body) -> Child`
- `fn from_bytes([]char data) -> Result[string, StringError]`
- `fn header_int(string text) -> int`
- `fn join_list([]string parts, string sep) -> string`
- `fn pad_end(string text, int width, string pad) -> string`
- `fn pad_start(string text, int width, string pad) -> string`
- `fn println_f64(float value) -> void`
- `fn println_i64(int value) -> void`
- `fn read_all() -> Result[string, io.Error]`
- `fn run([]string argv) -> Result[Child, io.Error]`
- `fn run_in([]string argv, string cwd) -> Result[Child, io.Error]`
- `fn run_with([]string argv, []string env, string cwd) -> Result[Child, io.Error]`
- `fn to_bytes(string text) -> []char`
- `fn write_stderr(string text) -> void`
### `rand`

- `fn fill(int count) -> []char`
- `fn range(int lo, int hi) -> int`
- `fn shuffle([]char data) -> void`
- `fn u64() -> int`
### `regex`

- `fn atom_after(string pattern, int pos) -> int`
- `fn atom_matches(string pattern, int pos, string text, int t) -> bool`
- `fn class_holds(string body, int byte) -> bool`
- `fn close_paren(string pattern, int open) -> int`
- `fn compile(string pattern) -> Result[Regex, RegexError]`
- `fn escape_holds(int code, int byte) -> bool`
- `fn escape_is_class(int code) -> bool`
- `fn find(Regex rx, string text) -> Option[Match]`
- `fn find_all(Regex rx, string text) -> []string`
- `fn is_match(Regex rx, string text) -> bool`
- `fn is_meta(int code) -> bool`
- `fn match_arm_end(string pattern, string text, int t, int depth) -> int`
- `fn match_rest(string pattern, string text, int pos, int t, int depth) -> int`
- `fn replace_all(Regex rx, string text, string replacement) -> string`
- `fn split_alternatives(string pattern) -> []string`
### `string`

- `fn char_at(string text, int index) -> Option[string]`
- `fn from_bytes([]char data) -> Result[string, StringError]`
- `fn join_list([]string parts, string sep) -> string`
- `fn pad_end(string text, int width, string pad) -> string`
- `fn pad_start(string text, int width, string pad) -> string`
- `fn to_bytes(string text) -> []char`
### `term`

- `fn bold() -> string`
- `fn clear_screen() -> string`
- `fn color(string name) -> string`
- `fn disabled() -> bool`
- `fn esc() -> string`
- `fn is_tty() -> bool`
- `fn move_to(int row, int column) -> string`
- `fn no_color_set() -> bool`
- `fn reset() -> string`
- `fn sgr_code(string name) -> int`
- `fn term_is_dumb() -> bool`
### `text`

- `fn base64_decode(string text) -> Result[[]char, TextError]`
- `fn base64_digit(int value) -> char`
- `fn base64_encode([]char data) -> string`
- `fn base64_valid(string text) -> bool`
- `fn base64_value(char next) -> int`
- `fn char_at(string text, int index) -> Option[string]`
- `fn from_bytes([]char data) -> Result[string, StringError]`
- `fn hex_decode(string text) -> Result[[]char, TextError]`
- `fn hex_digit(int value) -> char`
- `fn hex_encode([]char data) -> string`
- `fn hex_valid(string text) -> bool`
- `fn hex_value(char next) -> int`
- `fn join_list([]string parts, string sep) -> string`
- `fn pad_end(string text, int width, string pad) -> string`
- `fn pad_start(string text, int width, string pad) -> string`
- `fn to_bytes(string text) -> []char`
### `time`

- `fn monotonic_ns() -> int`
- `fn now_unix_ns() -> int`
