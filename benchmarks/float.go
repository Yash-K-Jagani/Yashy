// Benchmark 3: floating-point dependency chain. See float.ot.
package main

import "fmt"

func work(n int64) float64 {
	var x float64 = 1.0
	for i := int64(0); i < n; i++ {
		x = x*1.0000001 + 0.0000001
	}
	return x
}

func main() {
	fmt.Println(work(100000000))
}
