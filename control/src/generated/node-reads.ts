// Generated from infra/fleet/node-reads.openapi.json (sha256 dac99c0205b8efe94f23fb26f7b51f1d33c3a5271ffbe2ceafad046dbeb2cc93) by tools/openapi.
// Do not edit. Regenerate with `bun run openapi:generate`; update the spec with `bun scripts/blocklyd.ts bump <version>`.

export type paths = Record<string, never>;
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        /**
         * @description Who must be able to reach a port: the edge (players, through the router) or the control
         *     plane (console, status ping). The host decides which of its addresses each audience uses.
         * @enum {string}
         */
        Audience: "edge" | "control";
        EnrollResponse: {
            /** @description Client names allowed to call the node's API. */
            allowedClients: string[];
            caPem: string;
            /** @description clientAuth, for heartbeats; the same key. */
            clientCertPem: string;
            deploymentId: string;
            /** Format: uint64 */
            heartbeatSeconds: number;
            /** @description The node's durable identity, issued by the control plane. */
            nodeId: string;
            /** @description serverAuth, for the node's API; SAN `<nodeId>.nodes.<deployment>.fleet`. */
            serverCertPem: string;
        };
        ExecRequest: {
            /**
             * @description argv, never a shell string: blocklyd adds no shell. It runs inside the workload, as
             *     the workload's user, with its limits; never on the host.
             */
            command: string[];
            /**
             * Format: uint32
             * @default 30
             */
            timeoutSeconds?: number;
        };
        /**
         * @description `POST /v1/workloads/{id}/export`: the workload's data as a gzip tarball, PUT to a presigned
         *     URL. The URL is used once, and never stored or logged.
         */
        ExportRequest: {
            /**
             * @description Names at the top of the data the archive leaves out: what the image makes again when it
             *     is missing (a server jar, the libraries it unpacks), so a move carries only the world.
             * @default []
             */
            exclude?: string[];
            /**
             * @description Sent with the PUT, as the store's presigning asked for (a content type, say).
             * @default {}
             */
            headers?: {
                [key: string]: string;
            };
            /**
             * @description Where the archive goes instead when it is larger than one PUT carries.
             * @default null
             */
            parts?: components["schemas"]["PartsTarget"] | null;
            /**
             * @description The caller paused the workload's own saving (for Minecraft: `save-off`, `save-all
             *     flush`), so its files are consistent while it runs. Without it only a stopped workload
             *     exports.
             * @default false
             */
            quiesced?: boolean;
            url: string;
        };
        Fence: {
            /** Format: uint64 */
            currentEpoch: number;
            workload: string;
        };
        /**
         * @description `POST /v1/workloads/{id}/fence`: the control plane says which epoch is current. A copy
         *     older than it is stopped and never started again; a copy at or past it is left alone, so a
         *     late or repeated fence is harmless.
         */
        FenceRequest: {
            /** Format: uint64 */
            currentEpoch: number;
        };
        HeartbeatResponse: {
            /**
             * @description Copies this node holds that a newer placement superseded. The node stops each and never
             *     starts it again.
             * @default []
             */
            fences?: components["schemas"]["Fence"][];
            /**
             * Format: uint64
             * @description A new interval, if the control plane wants one.
             * @default null
             */
            heartbeatSeconds?: number | null;
            /**
             * Format: uint64
             * @description The execution lease: for this long after sending the heartbeat, the node may restart a
             *     workload that failed on its own, or resume one the host went down under. 0 for a node the
             *     control plane holds lost.
             * @default null
             */
            leaseSeconds?: number | null;
            /** @description The node as the control plane holds it: active, draining, lost or retired. */
            lifecycle: string;
            /**
             * @description The node's certificates end soon: it should renew them.
             * @default false
             */
            renew?: boolean;
            /**
             * @description A newer blocklyd to upgrade to: offered to one node of a region at a time.
             * @default null
             */
            upgrade?: components["schemas"]["UpgradeOffer"] | null;
        };
        /**
         * @description An archive larger than one PUT carries goes to the store in parts, as its multipart upload:
         *     the control plane begins the upload and presigns a URL for each part, and finishes it with the
         *     parts the node reports. Every part but the last is `part_size` bytes, and the archive takes as
         *     many of `urls`, from the first, as it needs; a URL left over is never called.
         */
        PartsTarget: {
            /**
             * @description Sent with every part's PUT.
             * @default {}
             */
            headers?: {
                [key: string]: string;
            };
            /** Format: uint64 */
            partSize: number;
            urls: string[];
        };
        PortSpec: {
            audience: components["schemas"]["Audience"][];
            /** Format: uint16 */
            containerPort: number;
            name: string;
            /** @default tcp */
            protocol?: components["schemas"]["Proto"];
        };
        /** @enum {string} */
        Proto: "tcp" | "udp";
        RenewResponse: {
            caPem: string;
            clientCertPem: string;
            serverCertPem: string;
        };
        Resources: {
            /**
             * Format: uint32
             * @description CPU ceiling in thousandths of a core (CFS quota). Also what the JVM counts as its
             *     processors. Absent: no ceiling, weight only.
             * @default null
             */
            cpuMillis?: number | null;
            /**
             * Format: uint32
             * @description Share of CPU under contention, relative (cgroup weight). Absent: proportional to memory.
             * @default null
             */
            cpuWeight?: number | null;
            /**
             * Format: uint32
             * @description Hard memory limit (no swap). Blockly's size: 3072 for a 3 GB server.
             */
            memoryMb: number;
            /**
             * Format: uint32
             * @description Processes and threads. Absent: the host default (4096).
             * @default null
             */
            pidsLimit?: number | null;
        };
        RestartPolicy: "no" | "on-failure";
        /** @description A field left out takes its value from `Default`, whether `restart` is absent or partial. */
        RestartSpec: {
            /**
             * Format: uint32
             * @default 3
             */
            maxRetries?: number;
            /** @default on-failure */
            policy?: components["schemas"]["RestartPolicy"];
        };
        /**
         * @description `POST /v1/workloads/{id}/restore`: replaces a stopped workload's data, from an archive at a
         *     presigned URL or from one of its own snapshots on this node.
         */
        RestoreRequest: {
            /**
             * @description The archive's sha256; a download that doesn't match is refused before anything changes.
             * @default null
             */
            sha256?: string | null;
            /**
             * @description A snapshot this node holds of the workload, instead of a URL.
             * @default null
             */
            snapshot?: components["schemas"]["SnapshotId"] | null;
            /** @default null */
            url?: string | null;
        };
        /** @description A secret value: never logged, never persisted by blocklyd, never returned. */
        Secret: string;
        /**
         * @description A local snapshot's id: the control plane's archive id (a UUID). It names a directory, so it is
         *     held to a workload id's rules.
         */
        SnapshotId: string;
        /**
         * @description `POST /v1/workloads/{id}/snapshots`: a copy of the workload's data on this node, kept beside
         *     it. Sharing blocks with the data where the filesystem can, it takes moments and no space
         *     until the data changes. The id is the caller's: asking again with the same one returns the
         *     snapshot already made.
         */
        SnapshotRequest: {
            id: components["schemas"]["SnapshotId"];
            /**
             * @description As for an export: the caller paused the workload's saving, so a running one may be copied.
             * @default false
             */
            quiesced?: boolean;
        };
        StopRequest: {
            /**
             * Format: uint32
             * @description Overrides the spec's grace for this stop.
             * @default null
             */
            timeoutSeconds?: number | null;
        };
        /** @enum {string} */
        StopSignal: "SIGTERM" | "SIGINT";
        /** @description A field left out takes its value from `Default`, whether `stop` is absent or partial. */
        StopSpec: {
            /**
             * @description Sent first; the workload saves and exits on it (Blockly's image: `stop` on the console).
             * @default SIGTERM
             */
            signal?: components["schemas"]["StopSignal"];
            /**
             * Format: uint32
             * @description How long the workload gets after the signal before it is killed.
             * @default 90
             */
            timeoutSeconds?: number;
        };
        Storage: {
            /** @description Where the workload's persistent data appears inside it (Blockly: `/data`). */
            mountPath: string;
            /**
             * Format: uint32
             * @description The size the control plane promised. Reported against measured usage; not enforced in v1
             *     (no per-directory quota on a plain filesystem).
             */
            sizeGb: number;
        };
        /** @description The blocklyd the control plane wants the node to run, at `GET /fleet/v1/blocklyd`. */
        UpgradeOffer: {
            sha256: string;
            /**
             * @description The release's Ed25519 signature of `version` and `sha256`, in hex: its `blocklyd.sig`,
             *     which the control plane passes on (upgrade/trust.rs). A node that lists `signed-upgrade`
             *     installs nothing without it.
             */
            signature?: string | null;
            version: string;
        };
        /**
         * @description `POST /v1/workloads/{id}/snapshots/{snapshot}/upload`: the snapshot as a gzip tarball, PUT to
         *     a presigned URL. The snapshot doesn't change, so this needs no quiet and no epoch.
         */
        UploadRequest: {
            /** @default {} */
            headers?: {
                [key: string]: string;
            };
            /**
             * @description Where the archive goes instead when it is larger than one PUT carries.
             * @default null
             */
            parts?: components["schemas"]["PartsTarget"] | null;
            url: string;
        };
        WorkloadSpec: {
            /**
             * @description Replaces the image's entrypoint; absent, the image starts as it was built to.
             * @default null
             */
            entrypoint?: string[] | null;
            /** @default {} */
            env?: {
                [key: string]: string;
            };
            /** @description An image reference. Must match the host's `allowed_images` prefixes. */
            image: string;
            /** @default {} */
            labels?: {
                [key: string]: string;
            };
            /** @default [] */
            ports?: components["schemas"]["PortSpec"][];
            resources: components["schemas"]["Resources"];
            /**
             * @default {
             *       "maxRetries": 3,
             *       "policy": "on-failure"
             *     }
             */
            restart?: components["schemas"]["RestartSpec"];
            /**
             * @description What a suspended workload may wake into without losing its memory: a `PUT` whose spec
             *     differs but names the same `resumesOn` as the spec it was suspended on keeps it suspended,
             *     and its spec is put in place at its next boot. Not part of the spec's digest.
             * @default null
             */
            resumesOn?: string | null;
            /** @description Environment the workload needs but nobody else may see (Blockly: `RCON_PASSWORD`). */
            secrets?: {
                [key: string]: components["schemas"]["Secret"];
            };
            /**
             * @default {
             *       "signal": "SIGTERM",
             *       "timeoutSeconds": 90
             *     }
             */
            stop?: components["schemas"]["StopSpec"];
            storage: components["schemas"]["Storage"];
        };
    };
    responses: never;
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export type operations = Record<string, never>;
