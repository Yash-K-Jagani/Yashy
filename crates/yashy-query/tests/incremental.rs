//! The query database's contract is not "it is fast": it is that a result is
//! recomputed exactly when an input it read changed. A stale answer here is a
//! wrong binary, so these tests pin the invalidation, not the timing.

use yashy_query::{Db, FnId};

/// The four functions the tests edit. `first` and `second` are independent
/// siblings, so an edit to one must not touch the other.
const TWO_FUNCTIONS: &str = "\
fn first() -> int {
    return 1;
}

fn second() -> int {
    return 2;
}

fn main() {
    print(first() + second());
}
";

const TWO_FUNCTIONS_EDITED: &str = "\
fn first() -> int {
    return 40 + 2;
}

fn second() -> int {
    return 2;
}

fn main() {
    print(first() + second());
}
";

#[test]
fn a_second_check_on_unchanged_sources_is_all_hits() {
    let mut db = Db::new();
    let main = db.add_source("main.ya", TWO_FUNCTIONS);
    db.set_entry(main);

    db.check(main).expect("the program must check");
    db.reset_stats();

    db.check(main).expect("the program must still check");
    let stats = db.stats();
    assert_eq!(
        stats.misses, 0,
        "an unchanged program must not recompute anything, recomputed: {:?}",
        stats.recomputed
    );
    assert!(
        stats.hits > 0,
        "the second check must be answered from the memo"
    );
}

#[test]
fn editing_one_function_recomputes_that_function_only() {
    let mut db = Db::new();
    let main = db.add_source("main.ya", TWO_FUNCTIONS);
    db.set_entry(main);

    db.check(main).expect("the program must check");
    let first = FnId::new(main, "first");
    let second = FnId::new(main, "second");
    db.verify_fn(first.clone()).expect("first must verify");
    db.verify_fn(second.clone()).expect("second must verify");
    db.emit_fn(first.clone()).expect("first must emit");
    db.emit_fn(second.clone()).expect("second must emit");

    db.update_source(main, TWO_FUNCTIONS_EDITED);
    db.reset_stats();

    db.check(main).expect("the edited program must check");
    db.verify_fn(first.clone()).expect("first must verify");
    db.verify_fn(second.clone()).expect("second must verify");
    db.emit_fn(first.clone()).expect("first must emit");
    db.emit_fn(second.clone()).expect("second must emit");

    let stats = db.stats();
    assert!(
        stats.recomputed.iter().any(|name| name == "check:main.ya"),
        "the edited file must be checked again: {:?}",
        stats.recomputed
    );
    assert!(
        stats
            .recomputed
            .iter()
            .any(|name| name == "verify:main.ya:first"),
        "the edited function's facts must be recomputed: {:?}",
        stats.recomputed
    );
    assert!(
        stats
            .recomputed
            .iter()
            .any(|name| name == "emit:main.ya:first"),
        "the edited function's IR must be recomputed: {:?}",
        stats.recomputed
    );
    assert!(
        !stats.recomputed.iter().any(|name| name.contains("second")),
        "an untouched sibling must not recompute: {:?}",
        stats.recomputed
    );
}

#[test]
fn editing_a_struct_field_recomputes_every_function_that_mentions_the_type() {
    const BEFORE: &str = "\
struct Point {
    x: int,
    y: int,
}

fn uses(Point p) -> int {
    return p.x;
}

fn untouched() -> int {
    return 7;
}

fn main() {
    let p = Point { x: 1, y: 2 };
    print(uses(p) + untouched());
}
";
    const AFTER: &str = "\
struct Point {
    x: int,
    y: float,
}

fn uses(Point p) -> int {
    return p.x;
}

fn untouched() -> int {
    return 7;
}

fn main() {
    let p = Point { x: 1, y: 2.0 };
    print(uses(p) + untouched());
}
";

    let mut db = Db::new();
    let main = db.add_source("main.ya", BEFORE);
    db.set_entry(main);

    db.check(main).expect("the program must check");
    let uses = FnId::new(main, "uses");
    let untouched = FnId::new(main, "untouched");
    db.verify_fn(uses.clone()).expect("uses must verify");
    db.verify_fn(untouched.clone())
        .expect("untouched must verify");
    db.emit_fn(uses.clone()).expect("uses must emit");
    db.emit_fn(untouched.clone()).expect("untouched must emit");

    db.update_source(main, AFTER);
    db.reset_stats();

    db.check(main).expect("the edited program must check");
    db.verify_fn(uses.clone()).expect("uses must verify");
    db.verify_fn(untouched.clone())
        .expect("untouched must verify");
    db.emit_fn(uses.clone()).expect("uses must emit");
    db.emit_fn(untouched.clone()).expect("untouched must emit");

    let stats = db.stats();
    assert!(
        stats
            .recomputed
            .iter()
            .any(|name| name == "verify:main.ya:uses"),
        "a function that mentions the type must be re-verified: {:?}",
        stats.recomputed
    );
    assert!(
        stats
            .recomputed
            .iter()
            .any(|name| name == "emit:main.ya:uses"),
        "a function that mentions the type must be re-emitted: {:?}",
        stats.recomputed
    );
    assert!(
        !stats
            .recomputed
            .iter()
            .any(|name| name.contains("untouched")),
        "a function that never names the type must not recompute: {:?}",
        stats.recomputed
    );
}

#[test]
fn recomputed_is_sorted_and_deterministic() {
    fn run() -> Vec<String> {
        let mut db = Db::new();
        let main = db.add_source("main.ya", TWO_FUNCTIONS);
        db.set_entry(main);
        db.check(main).expect("the program must check");
        db.verify_fn(FnId::new(main, "main"))
            .expect("main must verify");
        db.emit_fn(FnId::new(main, "main")).expect("main must emit");
        db.stats().recomputed
    }

    let first = run();
    let second = run();
    assert_eq!(
        first, second,
        "two identical runs must name the same queries"
    );
    let mut sorted = first.clone();
    sorted.sort();
    assert_eq!(first, sorted, "`recomputed` must be sorted");
    assert!(
        !first.is_empty(),
        "a fresh database must report what it computed"
    );
}
