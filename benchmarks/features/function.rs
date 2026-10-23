// The function.ya loop with a Rust fn pointer.
//
// `black_box` keeps the optimiser from seeing which function the pointer
// names, so the call inside `apply` stays indirect: the point of the row is
// the cost of a call through a function value.
use std::hint::black_box;

fn scale(x: i64, k: i64) -> i64 {
    x * k + x % 3
}

#[inline(never)]
fn apply(f: fn(i64, i64) -> i64, x: i64, k: i64) -> i64 {
    f(x, k)
}

fn main() {
    const N: i64 = 100_000_000;
    const K: i64 = 1000;
    let mut sum: i64 = 0;
    let mut i: i64 = 0;
    while i < N {
        sum = sum.wrapping_add(apply(black_box(scale as fn(i64, i64) -> i64), i, K));
        i = i.wrapping_add(1);
    }
    println!("{}", sum);
}
