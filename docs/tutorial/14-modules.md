# 14. Modules

## What you will learn

- How to split a program across files with `import`, and where a dotted name
  is looked up.
- How `pub` decides which items another module may name.
- How an import alias (`import lib as l`) names a module without owning its
  names.
- Why an import cycle is an error rather than a loop, and how to break one.

One file is enough for a hello world. A real program is not one file: names
collide, responsibilities blur, and two files that need each other create a
circle the compiler has to detect before it can do anything else. Modules are
Yashy's answer. A module is a file; importing it brings its `pub` items into
reach, and nothing else.

## Import

An `import` names a module with dotted segments. Each search root tries two
shapes, in order: a file, then a directory holding `mod.ya`:

- `a/b/c.ya` — the module `a.b.c` is one file;
- `a/b/c/mod.ya` — the module `a.b.c` is a directory whose entry file is
  `mod.ya`.

```text
main.ya
  import lib.util;
  fn main() { print(seven()); }

lib/util.ya
  pub fn seven() -> int { return 7; }
```

The search roots are the entry file's directory — or, when `YASHY_PATH` is
set, the roots it lists (`;`-separated on Windows, `:` elsewhere), tried in
order — with the bundled standard library appended in both cases. That is
why `import io;` finds the standard library from any directory, and why a
missing module names every candidate it tried.

A module's `pub` items join the importing file's scope: an unqualified call
resolves through the import. Note that imports are per file, not transitive:
`b.ya` importing `c.ya` makes `c`'s items visible in `b.ya`, not in every
file that imports `b.ya`.

```yashy
import math;

fn main() {
    print(max(3, 7));
    // => 7
}
```

`max` is declared `pub` in the standard library's `math` module, so the
unqualified call resolves through the import and `print` shows its result.
The standard library is what `import` is mostly for; user modules use the
same form, with the dotted path of your own files.

## Aliases

Short aliases keep a call site readable when a module's name is long — and
some names cannot head an expression at all, since `io` is a keyword. An
import may rename the module: `import math as m;` then calls go through
`m`. The alias names the module, not the module's items:

```yashy
import math as m;

fn main() {
    print(m.max(3, 7));
    print(max(3, 7));
    // => 7
    // => 7
}
```

Both spellings work at once: `v.println_i64(9)` and the unqualified
`println_i64(9)` name the same item, and visibility rules apply to both (a
private item through an alias is the same `E0801` as through the import).

An alias is the way out of a collision. Yashy has one namespace: two modules
exporting the same name make an unqualified use ambiguous, and the compiler
says so, naming both origins:

```text
a.ya: pub fn thing() -> int { return 1; }
b.ya: pub fn thing() -> int { return 2; }
main.ya:
  import a;
  import b;
  fn main() { print(thing()); }

error: duplicate definition of `thing`, from modules `a` and `b`
```

Qualify the use through an alias for one of them; the unqualified name stays
ambiguous by design, because "which `thing`" is a decision the reader should
be able to see.

## Visibility

An item is private unless it says `pub`. That one rule covers functions,
types, `const fn`s, and struct fields: a `pub` struct with a private field
is half-visible — the type may be named from another module, the field may
not. Using a private item from another module is an error, not a warning,
and the message names both the item and the module that hides it:

```text
// lib.ya
fn helper() -> int { return 7; }

// main.ya
import lib;
fn main() { print(helper()); }

main.ya:2:24
error[E0801]: `helper` is private to module `lib`
fn main() { print(helper()); }
                       ^^^^^^
  docs: https://yashy.dev/errors/E0801
```

The fix is to mark the item `pub` where it is declared, or to move the use
into the module that owns it. For a field, the `pub` goes on the field; the
struct being `pub` is not enough.

## Cycles

Modules cannot import each other in a circle. The loader walks the import
graph depth-first and keeps track of which modules are still loading; the
import that would close the loop is reported once, at that import, with the
whole cycle written out:

```text
// a.ya
import b;
pub fn a() -> int { return b(); }

// b.ya
import a;
pub fn b() -> int { return 1; }

b.ya:1:1
error[E0802]: import cycle: a.ya -> b.ya -> a.ya
import a;
^^^^^^^^
  docs: https://yashy.dev/errors/E0802
```

A self-import is the same error with a one-element cycle. The fix is always
the same shape: move the declarations both modules need into a third module
that neither imports back, and import that from each. A diamond — two
modules importing a shared third — is not a cycle and loads the shared
module once.

## How it works

The loader is `SourceLoader` in `yashy-driver`: a depth-first walk with a
`visiting` stack and a `loaded` map keyed by canonical path. Discovery only
lexes; it finds `import` statements without parsing, and the driver parses
every discovered file in parallel afterwards. Each import edge is recorded
as it is walked, so the loader can project the graph the phase-4 query
engine consumes as `Db::module_graph(entry)`: every per-file query key
includes the hashes of its imports, so editing `b.ya` invalidates `a.ya`
when `a` imports `b` and invalidates nothing else.

Name resolution carries the same graph implicitly: every item records the
module that declares it and whether it is `pub`, and resolution tries the
alias head first, then the unqualified name — which is why one namespace
with enforced visibility, rather than a new naming syntax, is enough.

## The rules, as diagnostics

| Code | The rule |
|---|---|
| `E0801` | A path names an item that is not `pub` from outside its module. |
| `E0802` | An import closes a cycle in the module graph, including a self-import. |
| `E0001` | A name is defined in more than one module and used unqualified. |

## Common mistakes

- **Forgetting `pub`.** A helper in another file is private by default; the
  first use from `main.ya` is `E0801`. Mark the item `pub` where it is
  declared.
- **Expecting transitive imports.** Importing `b` does not import what `b`
  imports. Each file imports what it uses.
- **Breaking a cycle with a comment.** The cycle is in the edges, not the
  text: `a` importing `b` and `b` importing `a` is `E0802` no matter which
  import you read first. Move the shared declarations to a third module.
- **Naming a module `io` when you mean your file.** The standard library is
  appended to every search, so your `io.ya` in the entry directory shadows
  it for files that import `io` from that root. Name yours something else.
- **Qualifying without an alias.** `lib.thing()` needs `import lib as l;`
  first: the qualifier is the alias, and without it the dotted name reads as
  a module path, not an item.

## Try it

1. Split a program the way the examples do: an entry file importing
   `lib.util`, with `pub fn` in `lib/util.ya`. Build it, then remove `pub`
   and read the `E0801`. Hint: the caret is at the use, not the declaration.
2. Make two modules that import each other and read the `E0802`: check that
   the cycle path starts and ends at the same file and that it is reported
   once. Hint: break it by moving the shared item to a third module.

For traits, the object model, iterators and comprehensions, see
[15-traits](15-traits.md).

Continue to the [CLI reference](../cli.md).
