// The map_array.ot loop in Go: build a slice, map it into a second slice, and
// sum the second one.
package main

import "fmt"

const n = 10000000

func main() {
	xs := make([]int64, n)
	for i := int64(0); i < n; i++ {
		xs[i] = i
	}
	doubled := make([]int64, n)
	for i, x := range xs {
		doubled[i] = x * 2
	}
	var sum int64
	for _, x := range doubled {
		sum += x
	}
	fmt.Println(sum)
}
