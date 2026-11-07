-- SPDX-License-Identifier: FSL-1.1-ALv2
CREATE TABLE projects (
  id text PRIMARY KEY, name text NOT NULL, created_at timestamptz NOT NULL DEFAULT now(),
  state text NOT NULL DEFAULT 'active' CHECK (state IN ('active','frozen','deleted')),
  max_units bigint CHECK(max_units >= 0), max_sandboxes bigint CHECK(max_sandboxes >= 0),
  max_running bigint CHECK(max_running >= 0), max_units_per_sandbox bigint CHECK(max_units_per_sandbox >= 0),
  max_ttl bigint CHECK(max_ttl >= 0), max_idle bigint CHECK(max_idle >= 0),
  max_exec_timeout bigint CHECK(max_exec_timeout >= 0), max_snapshots bigint CHECK(max_snapshots >= 0),
  max_snapshot_bytes bigint CHECK(max_snapshot_bytes >= 0), max_egress_ports bigint CHECK(max_egress_ports >= 0),
  allowed_images jsonb, expires_at timestamptz
);
INSERT INTO projects(id,name) SELECT project,project FROM (
  SELECT project FROM keys UNION SELECT project FROM sandboxes UNION SELECT project FROM templates UNION SELECT 'default'
) legacy;
ALTER TABLE keys ADD FOREIGN KEY(project) REFERENCES projects(id);
ALTER TABLE sandboxes ADD FOREIGN KEY(project) REFERENCES projects(id);
ALTER TABLE templates ADD FOREIGN KEY(project) REFERENCES projects(id);
ALTER TABLE sandboxes ADD COLUMN quota_running boolean NOT NULL DEFAULT false;
UPDATE sandboxes SET quota_running=true WHERE deleted_at IS NULL AND state IN ('creating','starting','restoring','running','sleeping','failed');
ALTER TABLE sandbox_snapshots ADD COLUMN size_bytes bigint;
UPDATE sandbox_snapshots s SET size_bytes=a.size_bytes FROM fleet_archives a WHERE a.id=s.id;
CREATE TABLE project_snapshot_reservations(id uuid PRIMARY KEY, project text NOT NULL REFERENCES projects(id), sandbox text NOT NULL REFERENCES sandboxes(id));
CREATE INDEX sandboxes_project_live ON sandboxes(project) WHERE deleted_at IS NULL;
