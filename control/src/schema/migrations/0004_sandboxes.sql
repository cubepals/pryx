CREATE TABLE sandboxes (
  id text PRIMARY KEY,
  project text NOT NULL,
  spec jsonb NOT NULL,
  region text NOT NULL,
  handle text,
  usage jsonb,
  state text NOT NULL DEFAULT 'creating',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  activity_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz,
  deleted_at timestamptz,
  expired boolean NOT NULL DEFAULT false
);
CREATE TABLE sandbox_snapshots (
  id uuid PRIMARY KEY,
  sandbox text NOT NULL REFERENCES sandboxes(id),
  handle text NOT NULL,
  spec jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  deleted_at timestamptz
);
CREATE TABLE sandbox_sessions (
  id text PRIMARY KEY,
  sandbox text NOT NULL REFERENCES sandboxes(id),
  connected_at timestamptz NOT NULL DEFAULT now(),
  seen_at timestamptz NOT NULL DEFAULT now()
);
