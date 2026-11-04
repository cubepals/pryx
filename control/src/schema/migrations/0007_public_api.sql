-- SPDX-License-Identifier: FSL-1.1-ALv2
CREATE TABLE keys (id text PRIMARY KEY, hash text NOT NULL UNIQUE, project text NOT NULL, name text NOT NULL, created_at timestamptz NOT NULL DEFAULT now(), revoked_at timestamptz);
CREATE TABLE api_operations (project text NOT NULL, scope text NOT NULL, key text NOT NULL, id text UNIQUE NOT NULL, fingerprint text NOT NULL, state text NOT NULL DEFAULT 'new', response jsonb, PRIMARY KEY(project,scope,key));
ALTER TABLE sandboxes ADD COLUMN public_ports jsonb NOT NULL DEFAULT '[]';
ALTER TABLE sandboxes ADD COLUMN internal boolean NOT NULL DEFAULT false;
ALTER TABLE sandboxes ADD COLUMN only_node text;
CREATE TABLE templates (id text PRIMARY KEY, project text NOT NULL, name text NOT NULL, spec jsonb NOT NULL, created_at timestamptz NOT NULL DEFAULT now(), UNIQUE(project,name));
CREATE TABLE template_copies(template text NOT NULL REFERENCES templates(id), node text NOT NULL, sandbox text NOT NULL, snapshot text, error text, PRIMARY KEY(template,node));
