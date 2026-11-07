CREATE TYPE "public"."fleet_node_lifecycle" AS ENUM('active', 'draining', 'lost', 'retired');--> statement-breakpoint
CREATE TYPE "public"."fleet_placement_state" AS ENUM('placing', 'placed', 'released', 'displaced');--> statement-breakpoint
CREATE TABLE "fleet_archives" (
	"id" uuid PRIMARY KEY NOT NULL,
	"workload" text NOT NULL,
	"purpose" text NOT NULL,
	"epoch" bigint NOT NULL,
	"node_id" uuid,
	"local_id" text,
	"local_deleted_at" timestamp with time zone,
	"object_key" text,
	"status" text NOT NULL,
	"sha256" text,
	"size_bytes" bigint,
	"consistency" text NOT NULL,
	"captured_at" timestamp with time zone NOT NULL,
	"upload_started_at" timestamp with time zone,
	"uploaded_at" timestamp with time zone,
	"attempts" integer DEFAULT 0 NOT NULL,
	"error" text,
	"deleted_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "fleet_archives_object_key_unique" UNIQUE("object_key")
);
--> statement-breakpoint
CREATE TABLE "fleet_endpoints" (
	"process_id" text PRIMARY KEY NOT NULL,
	"started_at" timestamp with time zone NOT NULL,
	"seen_at" timestamp with time zone NOT NULL
);
--> statement-breakpoint
CREATE TABLE "fleet_events" (
	"id" bigint PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "fleet_events_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 9223372036854775807 START WITH 1 CACHE 1),
	"at" timestamp with time zone DEFAULT now() NOT NULL,
	"kind" text NOT NULL,
	"node_id" uuid,
	"workload" text,
	"epoch" bigint,
	"data" jsonb DEFAULT '{}'::jsonb NOT NULL
);
--> statement-breakpoint
CREATE TABLE "fleet_node_tokens" (
	"id" uuid PRIMARY KEY NOT NULL,
	"token_sha256" text NOT NULL,
	"region_key" text NOT NULL,
	"labels" jsonb DEFAULT '{}'::jsonb NOT NULL,
	"expires_at" timestamp with time zone NOT NULL,
	"used_at" timestamp with time zone,
	"used_by" uuid,
	"node_id" uuid,
	"created_by" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "fleet_node_tokens_token_sha256_unique" UNIQUE("token_sha256")
);
--> statement-breakpoint
CREATE TABLE "fleet_nodes" (
	"id" uuid PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"region_key" text NOT NULL,
	"api_address" text NOT NULL,
	"edge_host" text NOT NULL,
	"control_host" text NOT NULL,
	"lifecycle" "fleet_node_lifecycle" DEFAULT 'active' NOT NULL,
	"lost_at" timestamp with time zone,
	"lost_reason" text,
	"fenced_by" text,
	"cert_sha256" text,
	"cert_expires_at" timestamp with time zone,
	"next_cert_sha256" text,
	"next_cert_expires_at" timestamp with time zone,
	"machine_id_sha256" text,
	"boot_id" text,
	"session_id" text,
	"session_started_at" timestamp with time zone,
	"previous_session_id" text,
	"duplicate_seen_at" timestamp with time zone,
	"heartbeat_seq" bigint DEFAULT 0 NOT NULL,
	"last_heartbeat_at" timestamp with time zone,
	"runtime_up" boolean DEFAULT false NOT NULL,
	"reconciled" boolean DEFAULT false NOT NULL,
	"health" text DEFAULT 'unavailable' NOT NULL,
	"health_since" timestamp with time zone DEFAULT now() NOT NULL,
	"capacity" jsonb DEFAULT '{}'::jsonb NOT NULL,
	"daemon_version" text,
	"features" jsonb DEFAULT '[]'::jsonb NOT NULL,
	"issues" jsonb DEFAULT '[]'::jsonb NOT NULL,
	"labels" jsonb DEFAULT '{}'::jsonb NOT NULL,
	"last_probe" jsonb,
	"enrolled_at" timestamp with time zone DEFAULT now() NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "fleet_observations" (
	"node_id" uuid NOT NULL,
	"workload" text NOT NULL,
	"epoch" bigint,
	"superseded_by" bigint,
	"state" text NOT NULL,
	"spec_digest" text,
	"report" jsonb NOT NULL,
	"observed_at" timestamp with time zone DEFAULT now() NOT NULL,
	"state_changed_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "fleet_observations_node_id_workload_pk" PRIMARY KEY("node_id","workload")
);
--> statement-breakpoint
CREATE TABLE "fleet_placement_history" (
	"workload" text NOT NULL,
	"epoch" bigint NOT NULL,
	"node_id" uuid,
	"reason" text NOT NULL,
	"started_at" timestamp with time zone DEFAULT now() NOT NULL,
	"ended_at" timestamp with time zone,
	"end_reason" text,
	CONSTRAINT "fleet_placement_history_workload_epoch_pk" PRIMARY KEY("workload","epoch")
);
--> statement-breakpoint
CREATE TABLE "fleet_placements" (
	"workload" text PRIMARY KEY NOT NULL,
	"node_id" uuid,
	"epoch" bigint NOT NULL,
	"state" "fleet_placement_state" NOT NULL,
	"region_key" text NOT NULL,
	"memory_mb" integer NOT NULL,
	"cpu_millis" integer NOT NULL,
	"disk_gb" integer NOT NULL,
	"ports" jsonb DEFAULT '{}'::jsonb NOT NULL,
	"handle" text,
	"spec_digest" text,
	"completing" text,
	"restore_from" uuid,
	"desired_power" text DEFAULT 'stopped' NOT NULL,
	"move_requested_at" timestamp with time zone,
	"move_to" uuid,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "fleet_archives" ADD CONSTRAINT "fleet_archives_node_id_fleet_nodes_id_fk" FOREIGN KEY ("node_id") REFERENCES "public"."fleet_nodes"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "fleet_node_tokens" ADD CONSTRAINT "fleet_node_tokens_used_by_fleet_nodes_id_fk" FOREIGN KEY ("used_by") REFERENCES "public"."fleet_nodes"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "fleet_node_tokens" ADD CONSTRAINT "fleet_node_tokens_node_id_fleet_nodes_id_fk" FOREIGN KEY ("node_id") REFERENCES "public"."fleet_nodes"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "fleet_observations" ADD CONSTRAINT "fleet_observations_node_id_fleet_nodes_id_fk" FOREIGN KEY ("node_id") REFERENCES "public"."fleet_nodes"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "fleet_placement_history" ADD CONSTRAINT "fleet_placement_history_node_id_fleet_nodes_id_fk" FOREIGN KEY ("node_id") REFERENCES "public"."fleet_nodes"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "fleet_placements" ADD CONSTRAINT "fleet_placements_node_id_fleet_nodes_id_fk" FOREIGN KEY ("node_id") REFERENCES "public"."fleet_nodes"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "fleet_placements" ADD CONSTRAINT "fleet_placements_restore_from_fleet_archives_id_fk" FOREIGN KEY ("restore_from") REFERENCES "public"."fleet_archives"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "fleet_placements" ADD CONSTRAINT "fleet_placements_move_to_fleet_nodes_id_fk" FOREIGN KEY ("move_to") REFERENCES "public"."fleet_nodes"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "fleet_archives_workload" ON "fleet_archives" USING btree ("workload","created_at");--> statement-breakpoint
CREATE INDEX "fleet_events_at" ON "fleet_events" USING btree ("at");--> statement-breakpoint
CREATE INDEX "fleet_events_node" ON "fleet_events" USING btree ("node_id","at");--> statement-breakpoint
CREATE INDEX "fleet_events_workload" ON "fleet_events" USING btree ("workload","at");--> statement-breakpoint
CREATE INDEX "fleet_nodes_region" ON "fleet_nodes" USING btree ("region_key");--> statement-breakpoint
CREATE INDEX "fleet_observations_workload" ON "fleet_observations" USING btree ("workload");--> statement-breakpoint
CREATE INDEX "fleet_placements_node" ON "fleet_placements" USING btree ("node_id");