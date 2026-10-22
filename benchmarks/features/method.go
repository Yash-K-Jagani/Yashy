// 10,000,000 calls to a method that mutates its receiver through a pointer.
package main

import "fmt"

type Counter struct {
	value int64
}

func (c *Counter) bump() {
	c.value = c.value + 1
}

func (c Counter) get() int64 {
	return c.value
}

func main() {
	c := Counter{}
	for i := 0; i < 10000000; i++ {
		c.bump()
	}
	fmt.Println(c.get())
}
