"""Reference implementations for the prima-language cross-language benchmark suite (bench/RESULTS.md).

Each function mirrors the corresponding `benches/workloads/<name>.pra` kernel exactly, so that the
same semantic computation is measured across Prima, Python, and Rust. Every function is pure and
deterministic; the harness sizes (N) are fixed per workload.

Run: python3 bench_ref.py <name> <n>
Prints the kernel result to stdout (the harness ignores stdout and only times the subprocess);
run with `--time` to also emit the measured seconds for standalone checking.
"""

import time


def sumsq(n):
    """sum_{i=0}^{n-1} i^2"""
    s = 0
    i = 0
    while i < n:
        s += i * i
        i += 1
    return s


def pi(n):
    """Leibniz series (exact -> F64 promotion), times 4"""
    acc = 0.0
    sign = 1.0
    i = 0
    while i < n:
        t = 2 * i + 1
        acc += sign / t
        sign = -sign
        i += 1
    return 4.0 * acc


def fib(n):
    """iterative Fibonacci"""
    if n <= 1:
        return n
    a = 0
    b = 1
    i = 2
    while i <= n:
        s = a + b
        a = b
        b = s
        i += 1
    return b


def sieve(n):
    """Sieve of Eratosthenes: count primes <= n"""
    prime = [True] * (n + 1)
    prime[0] = False
    prime[1] = False
    i = 2
    while i * i <= n:
        if prime[i]:
            j = i * i
            while j <= n:
                prime[j] = False
                j += i
        i += 1
    c = 0
    for k in range(n + 1):
        if prime[k]:
            c += 1
    return c


def dot(n):
    """dot product of index-modulo arrays"""
    x = [float(i % 13) for i in range(n)]
    y = [float((i * 3 + 1) % 17) for i in range(n)]
    s = 0.0
    for i in range(n):
        s += x[i] * y[i]
    return s


def poly(n):
    """Horner evaluation of a degree-4 polynomial over n values"""
    acc = 0.0
    i = 0
    while i < n:
        t = i / 7.0
        r = (((1.0 * t + 2.0) * t + 3.0) * t + 4.0) * t + 5.0
        acc += r
        i += 1
    return acc


KERNELS = {"sumsq": sumsq, "pi": pi, "fib": fib, "sieve": sieve, "dot": dot, "poly": poly}

if __name__ == "__main__":
    import sys

    name, n_str = sys.argv[1], sys.argv[2]
    n = int(n_str)
    fn = KERNELS[name]
    if "--time" in sys.argv:
        t0 = time.perf_counter()
        r = fn(n)
        dt = time.perf_counter() - t0
        print(f"result={r} seconds={dt:.6f}")
    else:
        print(fn(n))
