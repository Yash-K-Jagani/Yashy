package main

import "fmt"

func minMax(a, b int64) (int64, int64) {
	if a < b {
		return a, b
	}
	return b, a
}

func main() {
	const rounds int64 = 50000000
	var total int64
	for i := int64(0); i < rounds; i++ {
		lo, hi := minMax(i, rounds-i)
		total += lo*3 + hi
	}
	fmt.Println(total)
}
