// The array.ot loop with a Go slice: append, then walk it.
package main

import "fmt"

func main() {
	const n = 10000000
	xs := []int64{}
	for i := int64(0); i < n; i++ {
		xs = append(xs, i)
	}
	var sum int64
	for _, x := range xs {
		sum += x
	}
	fmt.Println(sum)
}
