// The mutex.ot loop with Go's sync.Mutex: four goroutines, one empty critical
// section each time, one million contended lock/unlock pairs in total.
package main

import (
	"fmt"
	"sync"
)

func worker(gate *sync.Mutex, rounds int64, done *sync.WaitGroup) {
	defer done.Done()
	for i := int64(0); i < rounds; i++ {
		gate.Lock()
		gate.Unlock()
	}
}

func main() {
	const rounds = int64(250000)
	var gate sync.Mutex
	var wg sync.WaitGroup
	for i := 0; i < 4; i++ {
		wg.Add(1)
		go worker(&gate, rounds, &wg)
	}
	wg.Wait()
	fmt.Println(rounds * 4)
}
