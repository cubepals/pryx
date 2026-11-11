// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Internal fleet engine types. The public sandbox model is in sandboxes.ts; node wire types are in wire.ts. */

declare const brand: unique symbol
type Opaque<T, B extends string> = T & { readonly [brand]: B }

/** The platform's stable id for a runtime (the server id). Adapters may embed it in names. */
export type RuntimeKey = Opaque<string, 'RuntimeKey'>
/** Issued by an adapter, stored verbatim, passed back. Changes on restore and relocate. */
export type RuntimeHandle = Opaque<string, 'RuntimeHandle'>
export type SnapshotHandle = Opaque<string, 'SnapshotHandle'>

export const runtimeKey = (sandboxId: string) => sandboxId as RuntimeKey

/** A product region key ("eu-central"); the adapter maps it to its own placement. */
export interface Placement {
  regionKey: string
}

export type Audience = 'edge' | 'control'

interface PortSpec {
  name: string
  port: number
  protocol: 'tcp' | 'udp'
  /** Who must be able to reach this port. */
  audience: readonly Audience[]
}

export interface RuntimeSpec {
  require?: readonly import('./guarantees.ts').Guarantee[]
  fuseMounts?: boolean
  isolation?: import('./isolation.ts').Isolation
  agent?: import('./wire.ts').WorkloadSpec['agent']
  sparePorts?: number | null
  network?: import('./wire.ts').WorkloadSpec['network']
  image: string
  packages?: readonly string[]
  /** Replaces the image's own entrypoint; absent, the image starts as it was built to. */
  entrypoint?: readonly string[]
  workdir?: string
  user?: string
  egress?: { allow: string[]; deny?: string[] }
  env: Readonly<Record<string, string>>
  secrets: Readonly<Record<string, string>>
  resources: {
    memoryMb: number
    allowUnenforcedMemory?: boolean | null
    cpuMillis?: number | null
    cpuReservationMillis?: number | null
    cpuWeight?: number | null
    pidsLimit?: number | null
  }
  restart?: { policy?: 'no' | 'on-failure'; maxRetries?: number }
  storage: {
    mountPath: string
    sizeGb: number
    /**
     * Names at the top of the mount path that the image makes again when they are missing (a
     * server jar and what it unpacks): a runtime moving a server's storage may leave them behind,
     * and the server's next start makes them. Absent or empty, everything is carried.
     */
    reconstructible?: readonly string[]
  }
  ports: readonly PortSpec[]
  stop: { signal: 'SIGTERM' | 'SIGINT'; timeoutSeconds: number }
  labels: Readonly<Record<string, string>>
  /**
   * Which specs a workload asleep may wake on as it slept: two with the same value differ only in
   * what the workload reads as it starts (a server's message, its picture, a plan's limits). A
   * runtime that keeps a sleeping workload's memory resumes one asleep on either, and the other
   * waits for its next boot. No digest sees it.
   */
  resumesOn?: string
}

/**
 * A server's install: files the image downloads that are the same for every server with the same
 * `key`. A runtime may make them once, running the spec's image with `env` alone against storage
 * of its own, and copy `paths` into a new server's storage before it first starts, so the image
 * finds them there and downloads nothing. It is never part of what a server is, so no digest sees
 * it, and a runtime that ignores it only makes a first start slower.
 */
export interface InstallSeed {
  key: string
  env: Readonly<Record<string, string>>
  /** Names at the top of the storage's mount path. */
  paths: readonly string[]
}

export interface Endpoint {
  host: string
  port: number
  /**
   * The runtime that issued it, where a deployment runs several: the console and readiness probe
   * that speak to it are that runtime's. Set by the router, never by an adapter.
   */
  provider?: string
}

export type ObservedState = 'absent' | 'stopped' | 'starting' | 'running' | 'stopping' | 'crashed' | 'unknown'

export interface RuntimeObservation {
  state: ObservedState
  /** When the state last changed, where the provider says; else when it was looked at. */
  at: Date
  exit?: { code: number; oom: boolean }
  /**
   * When it last stopped on its own, failing, where the provider started it again rather than
   * leave it stopped: the same configuration, which usually stops the same way.
   */
  failedAt?: Date
  /** The provider can't reach the host the compute is on; its storage may be gone with it. */
  hostLost?: boolean
  /**
   * The handle to keep, from `observe`, where the provider brought the compute back at another
   * address without being asked. Only an observation of the handle a server holds carries one.
   */
  handle?: RuntimeHandle
}

/** `resuming`: compute asleep with its memory kept is started as it was, and nothing else is done. */
type RuntimeStep = 'allocating' | 'waiting_for_room' | 'storage' | 'compute' | 'booting' | 'resuming'

/**
 * Why a start boots rather than resumes, where a runtime can tell: `not_asleep`, its compute
 * stopped rather than asleep with its memory; `other_spec`, asleep on another spec than the one
 * asked for; `no_compute`, nothing there to start yet.
 */
export type RuntimeMark = 'not_asleep' | 'other_spec' | 'no_compute'

export interface ProgressSink {
  step(name: RuntimeStep): Promise<void>
  /** Persist immediately, so a crash mid-provision leaves nothing unfindable. */
  handle(handle: RuntimeHandle): Promise<void>
  /** Kept on the operation's record beside its steps, for the platform; a runtime may say none. */
  mark?(mark: RuntimeMark): Promise<void>
}

export type RestoreSource =
  | { kind: 'snapshot'; snapshot: SnapshotHandle }
  | {
      kind: 'archive'
      download: DownloadTarget
      /**
       * The sha256 the archive was written with, where its backup recorded one: a runtime that can
       * check what it downloaded refuses other bytes. Absent or null, nothing is checked.
       */
      sha256?: string | null
    }

/** What a restore waits on before it replaces anything (`SandboxRuntime.restoresBeside`). */
export interface RestoreOptions {
  /**
   * Settles once the data the restore replaces is kept elsewhere, its snapshot taken. Until then
   * the server's storage stays as it is; if it rejects, the restore throws that, and the server
   * is as it was.
   */
  kept?: Promise<unknown>
}

/** Presigned by the ArchiveStore; consumed inside one operation, never persisted. */
export interface UploadTarget {
  url: string
  headers: Record<string, string>
}
export interface DownloadTarget {
  url: string
}

/** A part of an upload in parts as its sender put it: its number, from 1, and the store's ETag. */
export interface PutPart {
  number: number
  etag: string
}

/**
 * One object put in parts, as an S3 multipart upload. Every part but the last is `partSize`
 * bytes, part n goes to `urls[n - 1]` with `headers`, and an archive takes as many as it needs.
 * `complete` joins the parts that were put into the object; `abort` drops what was put, and is
 * safe to call after `complete`, or twice.
 */
export interface PartsUpload {
  partSize: number
  urls: string[]
  headers: Record<string, string>
  complete(parts: readonly PutPart[]): Promise<void>
  abort(): Promise<void>
}

/**
 * Where a runtime writes an archive: in one PUT (`put`) when it is no larger than `maxPutBytes`,
 * the most the store takes in one (R2: 5 GiB less 5 MiB), else in parts (`inParts`), asked for
 * with the archive's size, or more, once that is known. `most` asks for fewer, larger parts, for
 * a runtime that hands every link to a command line.
 */
export interface ArchiveTarget {
  put: UploadTarget
  maxPutBytes: number
  inParts(sizeBytes: number, most?: number): Promise<PartsUpload>
}

export interface SandboxRuntime {
  /** Recorded beside each handle it issues, so it is never handed another runtime's. */
  readonly provider: string
  /**
   * The most sandboxes this runtime can hold at once, counting what each needs at its busiest,
   * where its provider limits that; null where nothing does. The platform's server cap is held
   * to it (§19.12).
   */
  readonly serverCeiling: number | null
  /**
   * A restore fills new storage beside the server's own and replaces nothing until its `kept`
   * has settled, so the data it replaces can be saved while the backup comes. Without it, a
   * restore may overwrite the server's storage at once, and is asked only once `kept` is.
   */
  readonly restoresBeside?: boolean

  /** Pure: whether this runtime issued `handle`. Every handle it issues says so of itself. */
  owns(handle: RuntimeHandle): boolean
  /** Pure: whether this runtime took `snapshot`. */
  ownsSnapshot(snapshot: SnapshotHandle): boolean
  /**
   * Whether a server of this size could be placed where `placement` maps now: false where the
   * runtime doesn't serve the region, or has no room there. A hint for choosing among runtimes;
   * `ensureProvisioned` still throws RuntimeFull when the room went meanwhile.
   */
  hasRoom(placement: Placement, size: { memoryMb: number; storageGb: number }): Promise<boolean>

  /**
   * Idempotent and convergent: creates whatever is missing for `key`, and leaves it started.
   * `storage`: the handle the server holds now, whose storage is its data, and storage a rebuild
   * kept (`relocate`'s `keepStorage`), which is never booted.
   */
  ensureProvisioned(
    key: RuntimeKey,
    placement: Placement,
    spec: RuntimeSpec,
    progress: ProgressSink,
    install?: InstallSeed | null,
    storage?: HeldStorage,
  ): Promise<RuntimeHandle>
  /** New configuration; restarts the workload if it is running. May return a new handle. */
  apply(handle: RuntimeHandle, spec: RuntimeSpec): Promise<RuntimeHandle>
  /**
   * Idempotent. Returns the handle to keep, saved as `apply`'s is: compute a provider brings back
   * at another address comes with a new one.
   */
  start(handle: RuntimeHandle): Promise<RuntimeHandle>
  /**
   * Stops the workload as `stop` does, killing it where it won't stop, and starts it again on the
   * same compute, so its handle and endpoint stay as they are.
   */
  restart(handle: RuntimeHandle): Promise<void>
  /**
   * Graceful, idempotent. A workload asleep (`sleep`) is stopped too, and what its memory held is
   * gone: what is on its storage is what it last saved.
   */
  stop(handle: RuntimeHandle): Promise<void>
  /**
   * Puts the workload to sleep with its memory kept, where the provider can, so the next `start`
   * resumes it where it was instead of booting it; where it can't this time, it stops as `stop`
   * does. Says which it did. Nothing is saved on the way: whoever asks saves first, since a
   * provider may drop the memory it kept (a host move, maintenance) and boot from storage instead.
   * Idempotent. A runtime without it only ever stops.
   */
  sleep?(handle: RuntimeHandle): Promise<'suspended' | 'stopped'>
  /**
   * Kills the workload at once, without the grace a stop gives it to save, and leaves it stopped;
   * idempotent. For a stop that failed for good (§9).
   */
  forceStop(handle: RuntimeHandle): Promise<void>
  waitRunning(handle: RuntimeHandle, signal: AbortSignal): Promise<void>

  /** How long the provider keeps a snapshot, when it decides that; null when it keeps them until deleted. */
  readonly snapshotLifetimeDays: number | null
  /**
   * A snapshot of the server's storage, once the provider has finished writing it. `taken` is
   * called as soon as the provider holds the moment it copies, which a provider that writes the
   * copy afterwards (Fly, for a minute or so) says long before this returns: from then on the
   * storage may change, a start included, without changing the snapshot. One that copies before
   * answering need not call it.
   */
  snapshot(
    handle: RuntimeHandle,
    taken?: () => void,
  ): Promise<{ snapshot: SnapshotHandle; sizeBytes: number; at: Date }>
  /**
   * Which of `snapshots` the provider no longer holds, told by its own identity: a stored handle
   * need not match how the provider would write it today, and may name storage the server has
   * since left behind (a restore or a move replaces it). One it can't tell about, including
   * another provider's, isn't gone; its `expiresAt` still ends it.
   */
  goneSnapshots(snapshots: readonly SnapshotHandle[]): Promise<ReadonlySet<SnapshotHandle>>
  /**
   * Frees a snapshot's storage. Providers whose snapshots only expire throw RuntimeUnsupported;
   * those snapshots live out `snapshotLifetimeDays`. Missing snapshots count as deleted.
   */
  deleteSnapshot(snapshot: SnapshotHandle): Promise<void>
  /**
   * Replaces storage, and compute where the provider requires it. Returns the new handle. From an
   * archive it also works on a handle `release` returned: fresh storage filled from the archive,
   * fresh compute on it, left stopped. A handle that `release` only got partway through works the
   * same way, and whatever the release left behind goes. `options.kept`: `restoresBeside`.
   */
  restore(
    handle: RuntimeHandle,
    from: RestoreSource,
    spec: RuntimeSpec,
    progress: ProgressSink,
    options?: RestoreOptions,
  ): Promise<RuntimeHandle>
  /**
   * Moves compute and storage to where `to` maps now. `from`: the host is lost, or can't run the
   * server, and the volume with it; the new storage comes from this snapshot instead, and the old
   * compute is left for the provider to clear. `keepStorage`: the old storage stays where it is
   * rather than going, a full copy of the data beside the new one, until `clearLeftovers` is no
   * longer told to keep it.
   */
  relocate(
    handle: RuntimeHandle,
    to: Placement,
    spec: RuntimeSpec,
    progress: ProgressSink,
    from?: SnapshotHandle,
    options?: { keepStorage?: boolean },
  ): Promise<RuntimeHandle>
  /**
   * Packs a snapshot's data into the archive store, at `target`: in one PUT, or in parts when it
   * is larger than one PUT carries. RuntimeUnsupported when this data can't be packed at all,
   * which no retry changes.
   */
  exportSnapshot(
    snapshot: SnapshotHandle,
    target: ArchiveTarget,
  ): Promise<{ sizeBytes: number; sha256: string }>
  /** Removes compute and keeps storage and snapshots (soft delete). */
  decommission(handle: RuntimeHandle): Promise<void>
  /**
   * Lets go of compute and storage once a server's data is safe elsewhere. The
   * handle it returns names neither, and still serves `endpoint`, since a resting server keeps its
   * route; `restore` from an archive, which brings both back; and `destroy`. Idempotent: what is
   * already gone counts as let go, so a release that stopped partway can simply be asked again.
   */
  release(handle: RuntimeHandle): Promise<RuntimeHandle>
  /**
   * Deletes storage held for a server that nothing uses: what a wake, a restore, a move or an
   * export that failed partway made, or a delete that gave up on; and compute beside the server's
   * that `handle` doesn't name, which such a restore or move made. Storage a machine mounts is kept, and so is
   * what `handle` names, unless `dataElsewhere`: a resting server's data is in the archive store.
   * Asked with the server's current handle while nothing else is in flight for it, since a restore
   * makes storage before any handle names it; a runtime may clear the same as its own restore
   * ends. One attempt at each, no waiting: what can't go yet stays for the next call. Returns what
   * it deleted, by the provider's names. A runtime that never leaves storage behind has no need of it.
   * `kept`: storage a rebuild kept (`relocate`'s `keepStorage`), by the handles that named it, which
   * stays too.
   */
  clearLeftovers?(
    handle: RuntimeHandle,
    dataElsewhere: boolean,
    kept?: readonly RuntimeHandle[],
  ): Promise<readonly string[]>
  /**
   * A handle for a server this runtime holds nothing for, as `release` leaves one: it serves
   * `endpoint`, `restore` from an archive, which makes storage and compute, and `destroy`. How a
   * data in the archive store comes to a runtime it never ran on (a move between runtimes).
   * Idempotent; makes nothing that bills.
   */
  adopt(key: RuntimeKey, placement: Placement, spec: RuntimeSpec): Promise<RuntimeHandle>
  /** Removes everything. Missing resources count as done. */
  destroy(target: RuntimeKey | RuntimeHandle): Promise<void>

  observe(handle: RuntimeHandle): Promise<RuntimeObservation>
  /**
   * What changed at the provider since `since`, from one listing of the deployment's compute
   * rather than a call per server; the adapter widens the window, since changes can be listed
   * late. Compute that is gone is reported `absent`. When part of the listing couldn't be read,
   * it throws after yielding the rest.
   */
  observeChanged(since: Date): AsyncIterable<{
    key: RuntimeKey
    handle: RuntimeHandle
    observation: RuntimeObservation
  }>
  /**
   * Pure: whether the compute a handle names sits where `placement` maps now. A product region
   * remapped after its provider region was deprecated no longer does (§2): the server moves.
   */
  isPlaced(handle: RuntimeHandle, placement: Placement): boolean
  /**
   * Pure: whether two handles name the same compute. A restore or move may replace it, and an
   * observation of what it replaced says nothing about the server any more.
   */
  sameCompute(a: RuntimeHandle, b: RuntimeHandle): boolean
  /** Everything this deployment owns at the provider. */
  inventory(): AsyncIterable<{ key: RuntimeKey; handle: RuntimeHandle }>

  /**
   * Pure, from the handle alone: where `audience` reaches `port`. It answers for every handle this
   * runtime issued, the one `release` returns included: a resting server keeps its route, since a
   * join is what wakes it. The edge's routes are read from the current handle on every poll, and
   * a wake answers with the destination once the server is up, so an endpoint may change when a
   * restore, a move or a release issues a new handle; one that stays put, as FlyRuntime's does,
   * only spares the edge a change of route.
   */
  endpoint(handle: RuntimeHandle, port: string, audience: Audience): Endpoint
  /**
   * Whether a stopped server's endpoint still reaches that server and nothing else, as a Flycast
   * address does. Where a provider may give a stopped server's address to other compute, the edge
   * routes the server nowhere until it runs again, and a wake answers with where it runs then.
   */
  readonly stableEndpoints: boolean

  /**
   * Writes what tells sandboxes apart onto what the provider lists for them, in its console and its
   * tools: whose server it is and what it's called, which the names it was given when it was made
   * can't say, since those never change. Tags are never part of what a server runs: nothing
   * restarts, no digest sees them, and compute a change or a move makes keeps them. Only a server
   * whose tags differ from what the provider holds is written, so asking with every server is the
   * backfill too, and a server with no compute has nowhere to hold them until it has some. Returns
   * how many it wrote; a runtime with nowhere to put them writes none.
   */
  tag(sandboxes: ReadonlyMap<RuntimeKey, RuntimeTags>): Promise<number>
  /**
   * Pure, from the handle: what the provider calls what it holds for a server, for an operator
   * looking for it there, and a link to it in the provider's console where it has one.
   */
  locate(handle: RuntimeHandle): RuntimeLocation
  /**
   * Pure: the provider's list prices, in US cents, for a server of this size where a handle
   * places it: an hour of compute while it runs, and a month of the storage it holds whether it
   * runs or not. Null where the provider doesn't bill by the server.
   */
  prices(handle: RuntimeHandle, size: { memoryMb: number; storageGb: number }): RuntimePrices | null
  /**
   * The machines it holds and what they cost, where it pays for capacity whether or not sandboxes
   * fill it (a fleet's nodes); null where only the sandboxes on it are billed.
   */
  capacity(): Promise<RuntimeCapacity | null>
}

/** One machine a runtime pays for whether sandboxes fill it or not. */
export interface RuntimeMachine {
  name: string
  region: string
  /** The runtime's word for how it is: active, draining… */
  state: string
  /** Its list price a month, in US cents, where its operator gave one. */
  monthlyCents: number | null
  /** Memory sandboxes may be placed in, after what the machine keeps for itself. */
  allocatableMemoryMb: number
  /** Memory the sandboxes running on it, or starting, are counted for: what a start needs room beside. */
  allocatedMemoryMb: number
  /** Memory of every server placed on it, running or asleep; where sleeping ones hold none, more than it has. */
  placedMemoryMb: number
  /** Memory it reports in use, where it reports it. */
  usedMemoryMb: number | null
  cpus: number
  /** CPU the sandboxes running on it, or starting, are counted for, in thousandths of a core. */
  allocatedCpuMillis: number
  /** CPU it reports in use, in cores, where it reports it. */
  usedCpuCores: number | null
  sandboxes: number
}

export interface RuntimeCapacity {
  machines: readonly RuntimeMachine[]
}

/**
 * Tags by name, in lowercase letters and underscores. They say who and what to an operator, and
 * never hold a secret, since the provider shows them. An adapter may shorten a long value.
 */
export type RuntimeTags = Readonly<Record<string, string>>

export interface RuntimeLocation {
  /**
   * The provider's names, in the order an operator reads them: `app`, `machine`, `region`… Where
   * the provider places compute by region, one is labelled `Region` ("fra"): the admin list
   * groups sandboxes by it.
   */
  names: ReadonlyArray<{ label: string; value: string }>
  link: string | null
}

export interface RuntimePrices {
  runningHourCents: number
  storageMonthCents: number
}

/** Thrown by an adapter for a capability its provider does not have. */
export class RuntimeUnsupported extends Error {
  constructor(provider: string, what: string) {
    super(`The ${provider} runtime does not support ${what}`)
    this.name = 'RuntimeUnsupported'
  }
}

/**
 * Thrown by an adapter when the provider has no room for the server where it belongs right now:
 * a wait passes it, and the owner is told so rather than what the provider said.
 */
export class RuntimeFull extends Error {
  constructor(
    provider: string,
    detail: string,
    readonly missingGuarantees: readonly import('./guarantees.ts').Guarantee[] = [],
    readonly missingIsolation?: import('./isolation.ts').Isolation,
  ) {
    super(`The ${provider} runtime has no room: ${detail}`)
    this.name = 'RuntimeFull'
  }
}

/** The storage a server holds, as `ensureProvisioned` is told of it. */
export interface HeldStorage {
  held?: RuntimeHandle | null
  kept?: readonly RuntimeHandle[]
}

/**
 * RuntimeFull for the one host a server's storage is tied to, refusing to run it there: the rest
 * of its region may have room, which only new storage made from a snapshot reaches.
 */
export class HostFull extends RuntimeFull {
  constructor(provider: string, detail: string) {
    super(provider, detail)
    this.name = 'HostFull'
  }
}

/**
 * RuntimeFull for the provider's limit on how much one account holds, not a host's room. A
 * machine frees as soon as another goes, so work that needs one more for a while (a restore, a
 * move, an export) waits for it, and is refused with this only once nothing freed meanwhile.
 */
export class RuntimeAtLimit extends RuntimeFull {
  constructor(provider: string, detail: string) {
    super(provider, detail)
    this.name = 'RuntimeAtLimit'
  }
}
