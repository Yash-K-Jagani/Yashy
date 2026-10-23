// Benchmark 3: floating-point dependency chain. See float.ya.
fn work(n: i64) -> f64 {
    let mut x: f64 = 1.0;
    for _ in 0..n {
        x = x * 1.0000001 + 0.0000001;
    }
    x
}

fn main() {
    println!("{}", work(100_000_000));
}
