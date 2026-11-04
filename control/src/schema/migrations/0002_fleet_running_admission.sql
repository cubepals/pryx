ALTER TABLE "fleet_placements" ADD COLUMN "power_changed_at" timestamp with time zone DEFAULT now() NOT NULL;--> statement-breakpoint
-- The last power command's time was not kept until now: the row's last change stands in for it,
-- so a copy its node reported stopped since then holds no memory.
UPDATE "fleet_placements" SET "power_changed_at" = "updated_at";
