# Compile-time evaluation

A `const fn` is a function the compiler runs itself. It has no run-time
existence: a call to one is replaced by the value the compiler computed, so the
body is never compiled and never called. A `const` item and an array length may
be the result of such a call.

```yashy
const fn sum_to(int n) -> int {
    let int total = 0;
    let i = 1;
    while i <= n {
        total += i;
        i += 1;
    }
    return total;
}

fn main() {
    const total = sum_to(10);
    print(total); // 55, computed once, by the compiler
}
```

That is why the body is a *restricted* language: an expression the evaluator
cannot compute is one whose value is not known until the program runs.

## What a `const fn` body may contain

* literals, and the parameters the call supplied;
* arithmetic, comparison, bitwise, `and`/`or`/`not` on those;
* fixed-array literals and checked indexing into them;
* calls to other `const fn`s (a private one only from its own module);
* `if`, with a `return` in each arm;
* `let`, written with a scalar type (`int`, `float`, `bool`, `string`, `char`)
  or inferred from its initializer, plus struct values, `.field` reads and enum constructors;
* assignment and compound assignment to a `let` binding;
* `while c { .. }` and `for ; c; u { .. }`, with unlabelled `break` and
  `continue`;
* `return`, on every path of a function that returns a value.

## What it refuses

Everything else is refused by name at the expression that wrote it, whether or
not anything calls the function:

| Refused | Why |
|---|---|
| `read_line()` and the other I/O builtins | the value exists only once the program runs |
| an `extern` call or a qualified call `A::b()` | the caller is not a `const fn` |
| method calls, lambdas | no compile-time value is built for them yet |
| `defer`, `arena`, `scope`, `unsafe { }`, `.await` | control flow the evaluator has no value for |
| a `switch` arm with a payload, tuple or or-pattern | only literal, range, binding and unit-variant patterns have compile-time values; guards are checked too |
| a condition binding (`if d := f(); ..`) | the binding's value is a run-time one |
| assigning a parameter | a parameter is immutable, as in every function |
| a labelled `break`/`continue` | there is one loop level to leave |

A `const fn` is also not callable at run time: `let x = sum_to(3);` outside a
`const` is a diagnostic, because the function has no body in the image.

## The budget

The evaluator runs on the compiler's own stack, so an evaluation that never
ends would take the compiler down with it. Three counters are the bound:

| Limit | Value | What it bounds |
|---|---|---|
| depth | 160 nested calls | the compiler's stack |
| calls | 1,000,000 per evaluation | a wide recursion |
| steps | 1,000,000 per evaluation | one expression tree, and one loop iteration |

A loop spends one step per iteration, so `while true { }` is a diagnostic and
never a hang.

## Codes

A failure is reported at the call that wanted the value, because that is where
the value was used:

## Compile-time assertions

`const_assert(condition)` is checked by the same interpreter. A true condition
lowers to nothing; a false one is `E1525` at the call, and a condition that
names a run-time value is `E1524`. Use the ordinary `assert(condition)` when the
value is only known when the program runs.

| Code | Rule | Cause |
|---|---|---|
| `E1520` | CONST-1 | integer overflow in a compile-time operation |
| `E1521` | CONST-2 | division or remainder by zero |
| `E1522` | CONST-3 | the step budget ran out |
| `E1523` | CONST-4 | the call depth or call budget ran out |
| `E1524` | CONST-5 | a name that is only known at run time |
| `E1525` | CONST-6 | a `const_assert` condition that evaluated to false |
| `E0400` | MUT-1 | assigning a parameter (the same code a run-time body gets) |

## Seeing what the compiler computed

`yashyc build --emit-ir` writes the `.ll` file beside the binary. A folded call
appears there as the literal it produced, never as a call; a `const fn` that no
run-time code reaches has no body in the module at all.
