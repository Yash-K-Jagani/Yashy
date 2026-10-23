// The map_scan.ya loop with Rust's std HashMap: the same table, the same ten
// rounds of collecting its keys and its values into two vectors, and the same
// probe of a key that is there and one that is not.
use std::collections::HashMap;

fn main() {
    const N: i64 = 1_000_000;
    const ROUNDS: usize = 10;
    let mut m: HashMap<i64, i64> = HashMap::new();
    for i in 0..N {
        m.insert(i, i * 3);
    }
    let mut total: i64 = 0;
    for _ in 0..ROUNDS {
        let keys: Vec<i64> = m.keys().copied().collect();
        let values: Vec<i64> = m.values().copied().collect();
        for k in &keys {
            total += *k;
        }
        for v in &values {
            total += *v;
        }
    }
    let mut hits: i64 = 0;
    for j in 0..N {
        if m.contains_key(&j) {
            hits += 1;
        }
        if m.contains_key(&(j + N)) {
            hits += 1;
        }
    }
    println!("{}", total + hits);
}
