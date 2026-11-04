-- SPDX-License-Identifier: FSL-1.1-ALv2
CREATE TABLE provider_acquisitions(id uuid PRIMARY KEY,provider text NOT NULL,region text NOT NULL,shape jsonb NOT NULL,state text NOT NULL DEFAULT 'new',host jsonb,token_id uuid REFERENCES fleet_node_tokens(id),node_id uuid REFERENCES fleet_nodes(id),reason text NOT NULL,cancel_requested boolean NOT NULL DEFAULT false,error text,requested_at timestamptz NOT NULL DEFAULT now(),acquired_at timestamptz,ready_at timestamptz);
CREATE UNIQUE INDEX provider_host_identity ON provider_acquisitions(provider,(host->>'id')) WHERE host IS NOT NULL AND state<>'released';
CREATE TABLE provider_orphans(provider text NOT NULL,host text NOT NULL,discovered_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(provider,host));
CREATE TABLE provider_node_state(node_id uuid PRIMARY KEY REFERENCES fleet_nodes(id),idle_since timestamptz,release_requested boolean NOT NULL DEFAULT false);
CREATE TABLE autoscale_regions(region text PRIMARY KEY,refusals integer NOT NULL DEFAULT 0,required jsonb NOT NULL);

CREATE FUNCTION pryx_provider_heartbeat() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.runtime_up AND NEW.reconciled AND NEW.duplicate_seen_at IS NULL AND NEW.lifecycle='active' AND NEW.last_heartbeat_at IS NOT NULL THEN
    UPDATE provider_acquisitions a SET ready_at=COALESCE(a.ready_at,NEW.last_heartbeat_at),node_id=NEW.id
    FROM fleet_node_tokens t WHERE a.token_id=t.id AND t.used_by=NEW.id;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER pryx_provider_healthy AFTER UPDATE OF last_heartbeat_at,runtime_up,reconciled ON fleet_nodes FOR EACH ROW EXECUTE FUNCTION pryx_provider_heartbeat();
