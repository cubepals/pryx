-- SPDX-License-Identifier: FSL-1.1-ALv2
ALTER TABLE edges ADD COLUMN features jsonb NOT NULL DEFAULT '[]'::jsonb;
