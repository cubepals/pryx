// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * How an edge asks for its routes (`edge.ts`), in what the edge's own image carries, which
 * imports nothing.
 *
 * Every route list has a version, sent with it in `ROUTES_VERSION_HEADER`: milliseconds, larger for
 * a later list. An edge names itself in `EDGE_ID_HEADER` and says in `ROUTES_HELD_HEADER` the
 * version it holds every route of, applied. A sandbox reads as running only once every edge that
 * counts holds the version that first named it running, so an edge says a version only once it
 * holds all of that list, and asks again at once after applying a new one. The list's ETag, sent
 * back as `If-None-Match`, only spares sending a list the edge has already.
 */
export const EDGE_ID_HEADER = 'x-edge-id'
export const EDGE_FEATURES_HEADER = 'x-pryx-edge-features'
export const SSH_WEBSOCKET_FEATURE = 'ssh-websocket'
export const HTTP_ONLY_FEATURE = 'http-only'
export const AGENT_ROUTE_PORT = '_pryx'
export const ROUTES_VERSION_HEADER = 'x-routes-version'
export const ROUTES_HELD_HEADER = 'x-routes-held'
/** When the edge's agent started, in milliseconds: a restarted edge is given time to catch up. */
export const EDGE_STARTED_HEADER = 'x-edge-started'

// IANA service-name registry: default cleartext HTTP and TLS HTTP ports.
export const HTTP_FRONT_PORT = 80
export const TLS_FRONT_PORT = 443
