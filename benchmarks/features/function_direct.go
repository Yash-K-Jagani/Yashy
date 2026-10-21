// The function.ot loop calling the function directly: the same work, without
// the func value. The difference between this row and the `function` row is
// the price of calling through a value.
package main

import "fmt"

func scale(x, k int64) int64 {
	return x*k + x%3
}

func main() {
	const n = 100000000
	const k = 1000
	var sum int64
	for i := int64(0); i < n; i++ {
		sum += scale(i, k)
	}
	fmt.Println(sum)
}
