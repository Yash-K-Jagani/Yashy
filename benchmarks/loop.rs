// Benchmark 2: integer loop with a data-dependent branch. See loop.ya.
fn work(n: i64) -> i64 {
    let mut acc: i64 = 0;
    for i in 0..n {
        if i % 2 == 0 {
            acc = acc + i;
        } else {
            acc = acc + i * 2;
        }
    }
    acc
}

fn main() {
    println!("{}", work(200_000_000));
}
