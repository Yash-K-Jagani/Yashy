// The array.ya loop with a Rust Vec: push, then walk it.
fn main() {
    const N: i64 = 10_000_000;
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..N {
        xs.push(i);
    }
    let sum: i64 = xs.iter().sum();
    println!("{}", sum);
}
