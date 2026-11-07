// SPDX-License-Identifier: FSL-1.1-ALv2
/** A private registration handle holds the creation-only secret; collection reads omit it. */
import type { Transport } from './transport.ts'
export type WebhookOptions = import('./generated/api.ts').components['schemas']['WebhookRequest']
export type WebhookView = import('./generated/api.ts').components['schemas']['Webhook']
export class Webhook {
  constructor(
    private http: Transport,
    readonly view: WebhookView,
    readonly secret: string,
  ) {}
  get id() {
    return this.view.id
  }
  delete() {
    return this.http.request<{ existed: boolean }>(`/v1/webhooks/${encodeURIComponent(this.id)}`, 'DELETE')
  }
  async deadletters() {
    return (
      await this.http.request<{ deliveries: unknown[] }>(
        `/v1/webhooks/${encodeURIComponent(this.id)}/deadletters`,
      )
    ).deliveries
  }
}
