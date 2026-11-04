-- SPDX-License-Identifier: FSL-1.1-ALv2
CREATE TABLE edge_addresses(sandbox text NOT NULL,name text NOT NULL,hostname text NOT NULL,front text NOT NULL,public_port integer NOT NULL,retired_at timestamptz, PRIMARY KEY(sandbox,name), UNIQUE(hostname,front,public_port));
CREATE TABLE edge_route_lists(version bigserial PRIMARY KEY,digest text NOT NULL,routes jsonb NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE edges(id text PRIMARY KEY,held bigint,synced_at timestamptz,first_seen_at timestamptz NOT NULL DEFAULT clock_timestamp(),seen_at timestamptz NOT NULL DEFAULT clock_timestamp());
ALTER TABLE sandboxes ADD COLUMN route_listed_version bigint;
