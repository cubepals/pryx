-- SPDX-License-Identifier: FSL-1.1-ALv2
CREATE TABLE store_lease_placements(workload text NOT NULL,node_id uuid NOT NULL REFERENCES fleet_nodes(id),epoch bigint NOT NULL,generation text NOT NULL,session text NOT NULL,PRIMARY KEY(workload,epoch));
CREATE TABLE store_lease_fences(workload text NOT NULL,node_id uuid NOT NULL REFERENCES fleet_nodes(id),epoch bigint NOT NULL,generation text NOT NULL,fenced_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(workload,epoch));
