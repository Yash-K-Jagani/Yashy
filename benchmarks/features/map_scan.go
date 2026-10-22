// The map_scan.ot loop with Go's built-in map: the same table, the same ten
// rounds of copying its keys and its values into two slices, and the same
// probe of a key that is there and one that is not.
package main

import "fmt"

func main() {
	const n = int64(1000000)
	const rounds = 10
	m := make(map[int64]int64)
	for i := int64(0); i < n; i++ {
		m[i] = i * 3
	}
	var total int64
	for r := 0; r < rounds; r++ {
		keys := make([]int64, 0, n)
		values := make([]int64, 0, n)
		for k, v := range m {
			keys = append(keys, k)
			values = append(values, v)
		}
		for _, k := range keys {
			total += k
		}
		for _, v := range values {
			total += v
		}
	}
	var hits int64
	for j := int64(0); j < n; j++ {
		if _, ok := m[j]; ok {
			hits++
		}
		if _, ok := m[j+n]; ok {
			hits++
		}
	}
	fmt.Println(total + hits)
}
