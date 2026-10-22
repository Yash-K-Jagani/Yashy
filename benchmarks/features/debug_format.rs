// The debug_format.ot loop with Rust's derived Debug: the same point, the
// same million format calls, and the same total length.
#[derive(Debug)]
struct Point {
    x: i64,
    y: i64,
}

fn main() {
    const ROUNDS: i64 = 1_000_000;
    let mut total: i64 = 0;
    for i in 0..ROUNDS {
        let p = Point { x: i, y: i * 2 };
        let text = format!("{:?}", p);
        total += text.len() as i64;
    }
    println!("{}", total);
}
