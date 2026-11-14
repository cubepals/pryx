// SPDX-License-Identifier: FSL-1.1-ALv2
// Serve retained fixture files with Go's standard HTTP server in either actual guest image.
// This is an application workload for installed-edge acceptance, never a product gateway.
package main

import (
	"flag"
	"log"
	"net/http"
)

func main() {
	address := flag.String("address", "127.0.0.1:3000", "Private application listener")
	flag.Parse()
	log.Fatal(http.ListenAndServe(*address, http.FileServer(http.Dir("."))))
}
