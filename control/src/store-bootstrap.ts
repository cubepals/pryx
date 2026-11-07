// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** One-box bucket initialization uses the same S3 API as archive storage; credentials stay in memory. */
import { CreateBucketCommand, HeadBucketCommand, S3Client, S3ServiceException } from '@aws-sdk/client-s3'
import { StatusCodes } from 'http-status-codes'
export async function ensureBucket(options: {
  endpoint: string
  bucket: string
  accessKeyId: string
  secretAccessKey: string
}) {
  const client = new S3Client({
    endpoint: options.endpoint,
    region: 'auto',
    forcePathStyle: true,
    credentials: options,
  })
  try {
    await client.send(new HeadBucketCommand({ Bucket: options.bucket }))
  } catch (error) {
    if (!(error instanceof S3ServiceException) || error.$metadata.httpStatusCode !== StatusCodes.NOT_FOUND)
      throw error
    await client.send(new CreateBucketCommand({ Bucket: options.bucket }))
  } finally {
    client.destroy()
  }
}
