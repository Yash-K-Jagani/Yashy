// The function.ot loop with a Go func value.
//
// `apply` is marked noinline so the call inside it stays a call through the
// func value rather than being inlined into the caller and turned back into a
// direct call.
package main

import "fmt"

func scale(x, k int64) int64 {
	return x*k + x%3
}

//go:noinline
func apply(f func(int64, int64) int64, x, k int64) int64 {
	return f(x, k)
}

func main() {
	const n = 100000000
	const k = 1000
	f := scale
	var sum int64
	for i := int64(0); i < n; i++ {
		sum += apply(f, i, k)
	}
	fmt.Println(sum)
}
