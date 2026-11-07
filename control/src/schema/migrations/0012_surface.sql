-- SPDX-License-Identifier: FSL-1.1-ALv2
CREATE UNIQUE INDEX sandbox_project_name ON sandboxes(project,(spec->>'name')) WHERE deleted_at IS NULL AND spec->>'name' IS NOT NULL;
CREATE INDEX sandbox_labels ON sandboxes USING gin((spec->'labels')) WHERE deleted_at IS NULL AND NOT internal;
CREATE TABLE sandbox_metrics(sandbox text NOT NULL,at timestamptz NOT NULL DEFAULT now(),metrics jsonb,PRIMARY KEY(sandbox,at));
CREATE INDEX sandbox_metrics_at ON sandbox_metrics(at);
CREATE TABLE template_versions(template text NOT NULL REFERENCES templates(id),version integer NOT NULL,snapshot uuid NOT NULL REFERENCES sandbox_snapshots(id),start text,plan jsonb NOT NULL,created_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(template,version));
CREATE TABLE webhooks(id uuid PRIMARY KEY,project text NOT NULL,url text NOT NULL,kinds jsonb NOT NULL,secret text NOT NULL,created_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE webhook_deliveries(id uuid PRIMARY KEY,webhook uuid NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,event bigint NOT NULL,payload text NOT NULL,attempts integer NOT NULL DEFAULT 0,next_at timestamptz NOT NULL DEFAULT now(),lease_until timestamptz,delivered_at timestamptz,dead_at timestamptz,last_error text,UNIQUE(webhook,event));
CREATE INDEX webhook_due ON webhook_deliveries(next_at) WHERE delivered_at IS NULL AND dead_at IS NULL;
