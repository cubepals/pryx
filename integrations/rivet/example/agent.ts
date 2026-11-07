// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** A small coding-agent loop: Rivet owns durable notes, pryx owns the shell, and the model credential stays in this client process. */

import OpenAI from 'openai'
import type { ChatCompletionMessageParam } from 'openai/resources/chat/completions'
import { createClient } from 'rivetkit/client'
import type { registry } from './main.ts'

const model = process.env.PRYX_AGENT_MODEL
const task = process.argv.slice(2).join(' ')
if (!model || !task || !process.env.OPENAI_API_KEY)
  throw new Error('Set PRYX_AGENT_MODEL and OPENAI_API_KEY; pass a coding task as arguments')
const client = createClient<typeof registry>({ endpoint: process.env.RIVET_ENDPOINT })
const api = new OpenAI(),
  turn = crypto.randomUUID()
const session = process.env.PRYX_AGENT_SESSION ?? crypto.randomUUID()
try {
  const agent = client.codingAgent.getOrCreate([session])
  await agent.make({
    image: process.env.PRYX_AGENT_IMAGE ?? 'alpine:3.22',
    entrypoint: ['sh', '-c', 'trap "exit 0" TERM; while :; do sleep 1; done'],
  })
  const messages: ChatCompletionMessageParam[] = [
    {
      role: 'system',
      content:
        'You are a coding agent. Use the shell tool with argv to inspect and edit files in /data. Stored notes are prior session context. Explain your final result. Do not request credentials.',
    },
    ...(await agent.memory()).notes.map((content) => ({ role: 'user' as const, content })),
    { role: 'user', content: task },
  ]
  await agent.remember(JSON.stringify({ task, turn }))
  const steps = Number(process.env.PRYX_AGENT_MAX_STEPS ?? 6)
  if (!Number.isSafeInteger(steps) || steps < 1)
    throw new Error('PRYX_AGENT_MAX_STEPS must be a positive integer')
  for (let step = 0; step < steps; step++) {
    const response = await api.chat.completions.create({
      model,
      messages,
      tools: [
        {
          type: 'function',
          function: {
            name: 'shell',
            description: 'Run argv in the sandbox; use sh -c for shell syntax.',
            strict: true,
            parameters: {
              type: 'object',
              properties: { argv: { type: 'array', items: { type: 'string' } } },
              required: ['argv'],
              additionalProperties: false,
            },
          },
        },
      ],
    })
    const message = response.choices[0]?.message
    if (!message) throw new Error('Model returned no choice')
    messages.push(message)
    if (!message.tool_calls?.length) {
      await agent.remember(message.content ?? '')
      console.log(message.content ?? '')
      break
    }
    for (const call of message.tool_calls) {
      if (call.type !== 'function' || call.function.name !== 'shell') throw new Error('Unsupported tool call')
      const request = JSON.parse(call.function.arguments) as { argv?: unknown }
      if (
        !Array.isArray(request.argv) ||
        !request.argv.length ||
        !request.argv.every((v) => typeof v === 'string')
      )
        throw new Error('Shell tool needs string argv')
      const key = `${turn}:${call.id}`
      await agent.remember(JSON.stringify({ command: request.argv, key, status: 'about to execute' }))
      const output = await agent.exec({ command: request.argv }, key)
      const content = JSON.stringify(output)
      await agent.remember(content)
      messages.push({ role: 'tool', tool_call_id: call.id, content })
    }
  }
  console.log(`Session: ${session}`)
  await agent.sleep()
} finally {
  await client.dispose()
}
