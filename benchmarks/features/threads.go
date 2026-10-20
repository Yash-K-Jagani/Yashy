// The threads.ot ping-pong with Go channels: 200,000 round trips over two
// capacity-one channels, which is the same strict alternation.
package main

import "fmt"

func echo(inbox chan int64, outbox chan int64, rounds int64) {
	for i := int64(0); i < rounds; i++ {
		n := <-inbox
		outbox <- n + 1
	}
}

func main() {
	const rounds = int64(200000)
	inbox := make(chan int64, 1)
	outbox := make(chan int64, 1)
	go echo(inbox, outbox, rounds)

	var sum int64
	for i := int64(0); i < rounds; i++ {
		inbox <- i
		sum += <-outbox
	}
	fmt.Println(sum)
}
