// SPDX-License-Identifier: FSL-1.1-ALv2
// Anonymous namespaces for the owned root CNI experiment. No global namespace mount or
// host policy changes are made; callers retain the returned kernel descriptor.
package rootnetwork

import (
	"errors"
	"github.com/vishvananda/netns"
)

// NewNamespace requires the caller to keep its current OS thread locked through this call.
func NewNamespace() (netns.NsHandle, error) {
	original, err := netns.Get()
	if err != nil {
		return netns.None(), err
	}
	defer original.Close()
	created, err := netns.New()
	if restored := netns.Set(original); err != nil || restored != nil {
		created.Close()
		return netns.None(), errors.Join(err, restored)
	}
	return created, nil
}
