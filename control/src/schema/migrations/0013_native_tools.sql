-- SPDX-License-Identifier: FSL-1.1-ALv2
CREATE TABLE sandbox_mounts(sandbox text PRIMARY KEY,project text NOT NULL,sealed text NOT NULL);
CREATE TABLE sandbox_ssh(id uuid PRIMARY KEY,sandbox text NOT NULL,project text NOT NULL,epoch bigint NOT NULL,slot integer NOT NULL,target_port integer NOT NULL,username text NOT NULL,host_key text NOT NULL,expires_at timestamptz NOT NULL,revoked_at timestamptz,sleep_started_at timestamptz);
CREATE INDEX sandbox_ssh_active ON sandbox_ssh(sandbox,expires_at) WHERE revoked_at IS NULL;

CREATE FUNCTION sandbox_ssh_idle_bound() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.state IN ('sleeping','resting') THEN
    UPDATE sandbox_ssh SET sleep_started_at=COALESCE(sleep_started_at,clock_timestamp()),
      expires_at=LEAST(expires_at,COALESCE(sleep_started_at,clock_timestamp())+((NEW.spec->>'idle')::double precision*interval '1 second'))
      WHERE sandbox=NEW.id AND revoked_at IS NULL AND NEW.spec->>'idle' IS NOT NULL;
  ELSIF NEW.state='running' THEN
    UPDATE sandbox_ssh SET sleep_started_at=NULL WHERE sandbox=NEW.id AND sleep_started_at IS NOT NULL;
  END IF;
  RETURN NEW;
END;
$$;
CREATE TRIGGER sandbox_ssh_idle_bound AFTER UPDATE OF state,spec ON sandboxes FOR EACH ROW EXECUTE FUNCTION sandbox_ssh_idle_bound();
