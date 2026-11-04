// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import { CreateBucketCommand, S3Client } from '@aws-sdk/client-s3'

/**
 * A bucket for one test run, on the S3 server tests point at (S3_TEST_ENDPOINT): for tests of other
 * adapters that need a store, since the SDK stays in this one.
 */
export async function createTestBucket(
  endpoint: string,
  credentials: { accessKeyId: string; secretAccessKey: string },
  bucket: string,
): Promise<void> {
  await new S3Client({ endpoint, region: 'auto', forcePathStyle: true, credentials }).send(
    new CreateBucketCommand({ Bucket: bucket }),
  )
}
