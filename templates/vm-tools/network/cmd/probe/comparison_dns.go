// SPDX-License-Identifier: FSL-1.1-ALv2
// Bound the first positive DNS read so one timeout leaves time for a fresh lookup.
package main

import (
	"context"
	"errors"
	"fmt"
	"net"
	"time"
)

func lookupComparisonHost(ctx context.Context, firstTimeout time.Duration, lookup func(context.Context, string) ([]string, error)) ([]string, int, error) {
	const name = "example.com"
	first, cancel := context.WithTimeout(ctx, firstTimeout)
	addresses, err := lookup(first, name)
	cancel()

	attempts := 1
	var networkError net.Error
	if ctx.Err() == nil && errors.As(err, &networkError) && networkError.Timeout() {
		attempts++
		addresses, err = lookup(ctx, name)
	}
	if err == nil && len(addresses) == 0 {
		err = fmt.Errorf("no addresses for %s", name)
	}
	return addresses, attempts, err
}
