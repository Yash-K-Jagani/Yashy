// Benchmark 2: integer loop with a data-dependent branch. See loop.ya.
//
// JavaScript has no 64-bit integers, so this runs on IEEE-754 doubles. That is
// not a footnote: the total exceeds 2^53, so the *printed value below differs
// from the C/Rust/Go/Yashy answer even though the loop does the same work*.
// 29999999922509360 is double rounding; 29999999900000000 is the exact sum.
// The timing is still comparable - same iteration count, same branch, same
// arithmetic shape - but the arithmetic is 53-bit, not 64-bit, so read this row
// as "a JIT'd double loop", not as "JavaScript beats C at 64-bit integers".
function work(n) {
  let acc = 0;
  for (let i = 0; i < n; i++) {
    if (i % 2 === 0) {
      acc = acc + i;
    } else {
      acc = acc + i * 2;
    }
  }
  return acc;
}

console.log(work(200000000));
