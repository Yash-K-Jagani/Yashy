// The function.ot loop calling the function directly: the same work, without
// the fn pointer. The difference between this row and the `function` row is
// the price of calling through a value.
fn scale(x: i64, k: i64) -> i64 {
    x * k + x % 3
}

fn main() {
    const N: i64 = 100_000_000;
    const K: i64 = 1000;
    let mut sum: i64 = 0;
    let mut i: i64 = 0;
    while i < N {
        sum = sum.wrapping_add(scale(i, K));
        i = i.wrapping_add(1);
    }
    println!("{}", sum);
}
