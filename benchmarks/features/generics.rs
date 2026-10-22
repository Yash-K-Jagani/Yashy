// The same loop with Rust's generic function.
use std::ops::{Add, Mul, Rem};

fn scale<T>(x: T, k: T) -> T
where
    T: Copy + From<u8> + Add<Output = T> + Mul<Output = T> + Rem<Output = T>,
{
    x * k + x % T::from(3u8)
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
