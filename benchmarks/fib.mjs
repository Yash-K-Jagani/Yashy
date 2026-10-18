// Benchmark 1: recursive Fibonacci. See fib.ot for what it measures.
function fib(n) {
  if (n < 2) return n;
  return fib(n - 1) + fib(n - 2);
}

console.log(fib(35));
