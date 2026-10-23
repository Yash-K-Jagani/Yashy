// The map.ya loop with Rust's std HashMap: one million inserts, one million
// lookups of the same keys.
use std::collections::HashMap;

fn main() {
    const N: i64 = 1_000_000;
    let mut m: HashMap<i64, i64> = HashMap::new();
    for i in 0..N {
        m.insert(i, i * 3);
    }
    let mut sum: i64 = 0;
    for j in 0..N {
        sum += m[&j];
    }
    println!("{}", sum);
}
