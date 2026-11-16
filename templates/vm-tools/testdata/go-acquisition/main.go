// SPDX-License-Identifier: FSL-1.1-ALv2
// A loopback TLS module proxy for real Go acquisition checks. The standard HTTP/2
// server supplies stream resets; this fixture does not implement a production proxy.
package main

import (
	"encoding/json"
	"encoding/pem"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
)

func main() {
	root, mode, certificate := os.Args[1], os.Args[2], os.Args[3]
	var mutex sync.Mutex
	counts := make(map[string]int)
	files := http.FileServer(http.Dir(root))
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/counts" {
			mutex.Lock()
			defer mutex.Unlock()
			_ = json.NewEncoder(w).Encode(counts)
			return
		}
		mutex.Lock()
		counts[r.URL.Path]++
		attempt := counts[r.URL.Path]
		mutex.Unlock()
		if strings.HasSuffix(r.URL.Path, ".zip") {
			switch {
			case mode == "unavailable" && attempt <= 2 || mode == "persistent-unavailable":
				http.Error(w, "fixture response body must stay private", http.StatusServiceUnavailable)
				return
			case mode == "rate-limit" && attempt <= 2:
				http.Error(w, "fixture response body must stay private", http.StatusTooManyRequests)
				return
			case mode == "forbidden":
				http.Error(w, "fixture response body must stay private", http.StatusForbidden)
				return
			case mode == "missing":
				http.NotFound(w, r)
				return
			case mode == "stall":
				<-r.Context().Done()
				return
			case mode == "reset" && attempt == 1:
				data, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(r.URL.Path)))
				if err != nil {
					panic(err)
				}
				w.Header().Set("Content-Length", strconv.Itoa(len(data)))
				_, _ = w.Write(data[:len(data)/2])
				w.(http.Flusher).Flush()
				panic(http.ErrAbortHandler)
			}
		}
		files.ServeHTTP(w, r)
	})
	server := httptest.NewUnstartedServer(handler)
	server.EnableHTTP2 = true
	server.StartTLS()
	defer server.Close()
	if err := os.WriteFile(certificate, pem.EncodeToMemory(&pem.Block{
		Type: "CERTIFICATE", Bytes: server.Certificate().Raw,
	}), 0o600); err != nil {
		panic(err)
	}
	fmt.Println(server.URL)
	for {
		time.Sleep(time.Hour)
	}
}
