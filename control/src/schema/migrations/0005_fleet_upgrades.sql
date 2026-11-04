ALTER TABLE "fleet_nodes" ADD COLUMN "upgrade_version" text;--> statement-breakpoint
ALTER TABLE "fleet_nodes" ADD COLUMN "upgrade_state" text;--> statement-breakpoint
ALTER TABLE "fleet_nodes" ADD COLUMN "upgrade_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "fleet_nodes" ADD COLUMN "upgrade_error" text;