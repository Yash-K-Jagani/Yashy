// The map_array.ot loop in Rust: build a vector, map it into a second one,
// and sum the second one.
const N: i64 = 10_000_000;

fn main() {
    let xs: Vec<i64> = (0..N).collect();
    let doubled: Vec<i64> = xs.iter().map(|x| x * 2).collect();
    let sum: i64 = doubled.iter().sum();
    println!("{}", sum);
}
