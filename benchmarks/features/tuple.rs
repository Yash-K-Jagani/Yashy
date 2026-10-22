fn min_max(a: i64, b: i64) -> (i64, i64) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

fn main() {
    let rounds: i64 = 50_000_000;
    let mut total: i64 = 0;
    for i in 0..rounds {
        let (lo, hi) = min_max(i, rounds - i);
        total += lo * 3 + hi;
    }
    println!("{}", total);
}
