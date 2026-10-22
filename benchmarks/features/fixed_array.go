package main

import "fmt"

// The Go counterpart of fixed_array.ot: a fixed-size array (not a slice),
// summed 50,000,000 times through one loop-carried accumulator.
func main() {
	const rounds = 50000000
	grid := [8]int64{1, 2, 3, 4, 5, 6, 7, 8}
	var total int64
	for i := 0; i < rounds; i++ {
		for _, value := range grid {
			total += value
		}
	}
	fmt.Println(total)
}
