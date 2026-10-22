// The same loop with Go's generic function.
package main

import "fmt"

func scale[T int64 | int32](x, k T) T {
	return x*k + x%3
}

func main() {
	const n = 100000000
	const k int64 = 1000
	var sum int64
	for i := int64(0); i < n; i++ {
		sum += scale(i, k)
	}
	fmt.Println(sum)
}
