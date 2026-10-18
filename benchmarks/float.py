# Benchmark 3: floating-point dependency chain. See float.ot.
def work(n):
    x = 1.0
    for _ in range(n):
        x = x * 1.0000001 + 0.0000001
    return x


print(work(100000000))
