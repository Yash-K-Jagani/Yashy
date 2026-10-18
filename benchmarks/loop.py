# Benchmark 2: integer loop with a data-dependent branch. See loop.ot.
def work(n):
    acc = 0
    for i in range(n):
        if i % 2 == 0:
            acc = acc + i
        else:
            acc = acc + i * 2
    return acc


print(work(200000000))
