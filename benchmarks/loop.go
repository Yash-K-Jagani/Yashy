// Benchmark 2: integer loop with a data-dependent branch. See loop.ot.
package main

import "fmt"

func work(n int64) int64 {
	var acc int64 = 0
	for i := int64(0); i < n; i++ {
		if i%2 == 0 {
			acc = acc + i
		} else {
			acc = acc + i*2
		}
	}
	return acc
}

func main() {
	fmt.Println(work(200000000))
}
