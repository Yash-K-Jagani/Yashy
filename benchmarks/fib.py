# Benchmark 1: recursive Fibonacci. See fib.ot for what it measures.
def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


print(fib(35))
