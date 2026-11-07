// SPDX-License-Identifier: FSL-1.1-ALv2
/** Host image installation arguments remain separate from the ordinary process proof. */
import { expect, test } from 'bun:test'
import { imageReference, imageUnit } from '../src/up-image.ts'

test('image references cannot add commands or unit directives', () => {
  expect(imageReference(['--image', 'localhost:5000/pryx:dev'])).toBe('localhost:5000/pryx:dev')
  for (const value of ['name\nExecStart=bad', 'pryx;bad', 'pryx --privileged'])
    expect(() => imageReference(['--image', value])).toThrow('Invalid image reference')
  const unit = imageUnit('pryx:test')
  expect(unit).toContain('--env-file /etc/pryx/image.env')
  expect(unit).toContain('source=/var/lib/pryx-image,target=/var/lib/pryx-image')
  expect(unit).toContain('--cap-add NET_ADMIN')
  expect(unit).not.toContain('--privileged')
})
