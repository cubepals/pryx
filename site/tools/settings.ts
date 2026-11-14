// SPDX-License-Identifier: FSL-1.1-ALv2
/** Version and address ownership for builds and publication; no default public hostname. */

import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { siteAddress } from '../../control/src/site-address.ts'

export const version = PRYX_VERSION
export const address = siteAddress(process.env.PRYX_SITE_URL)
export const base = `${address.prefix}v${version}/`
export const routePath = (id: string) => `${base}${id === 'index' ? '' : `${id.replace(/\/index$/, '')}/`}`
