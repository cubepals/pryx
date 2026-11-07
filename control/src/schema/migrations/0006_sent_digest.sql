-- What a start last sent each fleet placement's copy, so a start with the same spec sends nothing.
ALTER TABLE "fleet_placements" ADD COLUMN "sent_digest" text;
