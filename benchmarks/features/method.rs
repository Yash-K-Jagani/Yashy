// 10,000,000 calls to a method that mutates its receiver through `&mut self`.
struct Counter {
    value: i64,
}

impl Counter {
    fn bump(&mut self) {
        self.value = self.value + 1;
    }

    fn get(&self) -> i64 {
        self.value
    }
}

fn main() {
    let mut c = Counter { value: 0 };
    for _ in 0..10_000_000 {
        c.bump();
    }
    println!("{}", c.get());
}
