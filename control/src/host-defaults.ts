// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Bridges to pryxd's documented host reservation defaults, not product tuning. Override only with matching host policy. */
// src/config.rs CapacityConfig::default; src/infer.rs reserved_memory_mb. Rust does not export these to TypeScript.
export const HOST_MIN_MEMORY_MB = 2048
export const HOST_MEMORY_DIVISOR = 16
export const HOST_CPU_MILLIS = 1000
export const HOST_DISK_FLOOR_GB = 5
