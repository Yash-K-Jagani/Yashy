// The map.ot loop with Go's built-in map: one million inserts, one million
// lookups of the same keys.
package main

import "fmt"

func main() {
	const n = int64(1000000)
	m := make(map[int64]int64)
	for i := int64(0); i < n; i++ {
		m[i] = i * 3
	}
	var sum int64
	for j := int64(0); j < n; j++ {
		sum += m[j]
	}
	fmt.Println(sum)
}
