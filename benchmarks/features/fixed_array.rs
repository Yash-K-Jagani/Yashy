// The Rust counterpart of fixed_array.ot: a stack array, summed 50,000,000
// times through one loop-carried accumulator.
fn main() {
    const ROUNDS: i64 = 50_000_000;
    let grid: [i64; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
    let mut total: i64 = 0;
    for _ in 0..ROUNDS {
        for value in grid {
            total += value;
        }
    }
    println!("{}", total);
}
