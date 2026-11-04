// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Units of time, written once: in milliseconds for `Date` and timers, in seconds where an API
 * counts seconds (signed links, tokens, Postgres intervals). A length that is a setting, such as how
 * long a server idles before it sleeps, stays a named constant where it is used, written in these.
 *
 * It imports nothing, so domain/ and minecraft/, which import no packages, may use it, and so may
 * anything compiled without installing.
 */

export const SECOND_MS = 1000
export const MINUTE_MS = 60 * SECOND_MS
export const HOUR_MS = 60 * MINUTE_MS
export const DAY_MS = 24 * HOUR_MS

export const MINUTE_S = 60
export const HOUR_S = 60 * MINUTE_S
export const DAY_S = 24 * HOUR_S
