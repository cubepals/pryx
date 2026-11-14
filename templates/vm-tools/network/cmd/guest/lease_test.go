// SPDX-License-Identifier: FSL-1.1-ALv2
// Check resolver writes against the real filesystem before trusted networking is enabled.
package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestResolverCannotFollowImageLink(t *testing.T) {
	root := t.TempDir()
	target := filepath.Join(root, "outside")
	original := []byte("outside remains private\n")
	if err := os.WriteFile(target, original, 0600); err != nil {
		t.Fatal(err)
	}
	resolver := filepath.Join(root, "resolv.conf")
	if err := os.Symlink(target, resolver); err != nil {
		t.Fatal(err)
	}
	if err := writeResolver(resolver, "nameserver 10.73.0.1\n"); err == nil {
		t.Fatal("resolver followed an image link")
	}
	observed, err := os.ReadFile(target)
	if err != nil || string(observed) != string(original) {
		t.Fatalf("linked file changed: %q, %v", observed, err)
	}
	if err := os.Remove(resolver); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(resolver, original, 0600); err != nil {
		t.Fatal(err)
	}
	expected := "nameserver 10.73.0.1\n"
	if err := writeResolver(resolver, expected); err != nil {
		t.Fatal(err)
	}
	observed, err = os.ReadFile(resolver)
	if err != nil || string(observed) != expected {
		t.Fatalf("resolver differs: %q, %v", observed, err)
	}
}
