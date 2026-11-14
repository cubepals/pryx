// SPDX-License-Identifier: FSL-1.1-ALv2
// Anonymous namespaces for the owned root CNI experiment. No global namespace mount or
// host policy changes are made; callers retain the returned kernel descriptor.
package rootnetwork

import (
	"errors"
	"github.com/vishvananda/netns"
	"runtime"
)

// NewNamespace never exposes a namespace change to the caller's reusable OS threads.
func NewNamespace() (netns.NsHandle, error) {
	type result struct {
		handle  netns.NsHandle
		problem error
	}
	completed := make(chan result, 1)
	go func() {
		// Keep this dedicated thread locked: Go discards it after the goroutine returns,
		// including when restoration fails. No pooled caller thread changes namespace.
		runtime.LockOSThread()
		original, err := netns.Get()
		if err != nil {
			completed <- result{netns.None(), err}
			return
		}
		defer original.Close()
		created, err := netns.New()
		if restored := netns.Set(original); err != nil || restored != nil {
			created.Close()
			completed <- result{netns.None(), errors.Join(err, restored)}
			return
		}
		completed <- result{created, nil}
	}()
	value := <-completed
	return value.handle, value.problem
}
