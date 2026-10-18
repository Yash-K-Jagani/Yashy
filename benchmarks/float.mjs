// Benchmark 3: floating-point dependency chain. See float.ot.
function work(n) {
  let x = 1.0;
  for (let i = 0; i < n; i++) {
    x = x * 1.0000001 + 0.0000001;
  }
  return x;
}

console.log(work(100000000));
