// Generated from infra/fleet/node-writes.openapi.json (sha256 2f9dcc71221800e017b9be1eb891c23473f5d7d2235867e37663423ad68f5d24) by tools/openapi.
// Do not edit. Regenerate with `bun run openapi:generate`; update the spec with `bun scripts/blocklyd.ts bump <version>`.

export type paths = Record<string, never>;
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        CapacityView: {
            /**
             * Format: uint64
             * @description Memory workloads may be given: total minus what the host keeps for itself.
             */
            allocatableMemoryMb: number;
            /** Format: uint32 */
            portsAllocated: number;
            /** Format: uint32 */
            portsTotal: number;
            /**
             * Format: uint64
             * @description Memory of every workload with compute, running or not.
             */
            provisionedMemoryMb: number;
            /**
             * Format: uint64
             * @description Memory of running workloads: what a start is admitted against.
             */
            runningMemoryMb: number;
        };
        /** @description The blocklyd process itself, as distinct from the node it runs on. */
        DaemonView: {
            /** Format: double */
            cpuSeconds: number | null;
            /** Format: uint64 */
            rssBytes: number | null;
            startedAt: string;
            /** Format: uint64 */
            uptimeSeconds: number;
            version: string;
        };
        DataOutcome: "kept" | "trashed" | "absent";
        DeleteResponse: {
            data: components["schemas"]["DataOutcome"];
            /** @description False when nothing by this id existed: a repeated delete is a no-op. */
            existed: boolean;
            removedContainer: boolean;
            trashPath: string | null;
        };
        DockerView: {
            apiVersion: string | null;
            cgroupDriver: string | null;
            cgroupVersion: string | null;
            liveRestore: boolean | null;
            reachable: boolean;
            securityOptions: string[];
            storageDriver: string | null;
            version: string | null;
        };
        Endpoints: {
            control: string[];
            edge: string[];
        };
        EnrollRequest: {
            /** @description PKCS#10, PEM. The key never leaves the node. */
            csrPem: string;
            facts: components["schemas"]["NodeFacts"];
            token: string;
        };
        EnsureOutcome: "created" | "unchanged" | "replaced" | "deferred";
        EnsureResponse: {
            outcome: components["schemas"]["EnsureOutcome"];
            restarted: boolean;
            workload: components["schemas"]["WorkloadView"];
        };
        ErrorBody: {
            error: components["schemas"]["ErrorDetail"];
        };
        ErrorDetail: {
            code: string;
            details?: unknown;
            message: string;
        };
        ExecResponse: {
            /** Format: uint64 */
            durationMs: number;
            /**
             * Format: int64
             * @description None when it timed out and was killed before reporting one.
             */
            exitCode: number | null;
            /**
             * @description blocklyd killed the process (by its host pid) after the timeout. Docker alone can't:
             *     its API has no way to stop an exec (moby/moby#35703).
             */
            killed: boolean;
            stderr: string;
            stderrTruncated: boolean;
            stdout: string;
            stdoutTruncated: boolean;
            timedOut: boolean;
        };
        ExitInfo: {
            at: string | null;
            /** Format: int64 */
            code: number;
            oomKilled: boolean;
        };
        ExportResponse: {
            /** Format: uint64 */
            durationMs: number;
            /**
             * Format: uint64
             * @description Directories, files and links in the archive.
             */
            entries: number;
            /** @description `tar.gz`: paths relative to the data root, like every archive Blockly keeps. */
            format: string;
            /** @description Present when the archive went in parts: what finishes the upload. */
            parts?: components["schemas"]["PutPart"][] | null;
            /** @description Of the archive as uploaded: what a restore checks. */
            sha256: string;
            /** Format: uint64 */
            sizeBytes: number;
        };
        FenceResponse: {
            /** @description False when this copy wasn't older than the current epoch, or was already fenced. */
            changed: boolean;
            /** @description The copy was running and has been stopped. */
            stopped: boolean;
            workload: components["schemas"]["WorkloadView"];
        };
        FleetView: {
            /** @description When the node last renewed its certificates, in this run. */
            certificateRenewedAt: string | null;
            controlPlane: string;
            /** Format: uint64 */
            heartbeatsFailed: number;
            /** Format: uint64 */
            heartbeatsOk: number;
            /** Format: uint64 */
            lastContactAgeSeconds: number | null;
            lastContactAt: string | null;
            lastError: string | null;
            /** Format: uint64 */
            lastLatencyMs: number | null;
            /**
             * Format: uint64
             * @description How long this node may still restart a failed workload on its own (see
             *     `fleet::heartbeat`). None until the control plane has granted a lease.
             */
            leaseRemainingSeconds: number | null;
            /** @description active, draining, lost or retired, as the control plane last said. */
            lifecycle: string | null;
            nodeId: string;
        };
        /** @enum {string} */
        Health: "starting" | "healthy" | "unhealthy";
        HeartbeatRequest: {
            /**
             * @description Where the node is reached now; the control plane follows a change.
             * @default null
             */
            addresses: components["schemas"]["NodeAddresses"] | null;
            bootId: string | null;
            capacity: components["schemas"]["NodeCapacity"];
            daemonVersion: string;
            features: string[];
            issues: components["schemas"]["Issue"][];
            nodeId: string;
            protocol: components["schemas"]["ProtocolVersions"];
            /** @description A full pass has looked since the runtime last didn't answer: the states below are current. */
            reconciled: boolean;
            /** @description Docker answers. */
            runtimeUp: boolean;
            /**
             * Format: uint64
             * @description Counts up within a session, so a late or replayed beat can be told apart.
             */
            seq: number;
            /**
             * @description New each time blocklyd starts. Two sessions beating for one node id at once are two
             *     hosts with one identity.
             */
            sessionId: string;
            /** @description An upgrade this node gave up on, going back to the binary it had (cli/upgrade.rs). */
            upgradeFailed?: components["schemas"]["UpgradeFailure"] | null;
            workloads: components["schemas"]["WorkloadReport"][];
        };
        /** @description Something reconciliation or an operation found that the control plane should know about. */
        Issue: {
            code: string;
            detail: string;
        };
        ListResponse: {
            observedAt: string;
            workloads: components["schemas"]["WorkloadView"][];
        };
        /** @description Where an operator finds it on the host. Informational; never a contract. */
        Locate: {
            containerId: string | null;
            containerName: string;
            dataDir: string;
        };
        /** @description One line of `GET …/logs` (NDJSON): a log line, or the event that ends a stream. */
        LogRecord: {
            line: string;
            stream: string;
            ts: string | null;
        } | {
            event: string;
            reason: string;
        };
        Method: "reflink" | "copy";
        /** @description Where the node is reached, as its configuration says now. */
        NodeAddresses: {
            /** @description blocklyd's API, `host:port`. */
            api: string;
            /** @description Where the control plane reaches console and status ports. */
            control: string;
            /** @description Where the edge reaches game ports. */
            edge: string;
        };
        /**
         * @description Physical, reserved and allocated capacity, and what is actually used. Placement decides on
         *     the control plane's own ledger; this is what the node sees, to check it against.
         */
        NodeCapacity: {
            /** Format: uint64 */
            allocatableMemoryMb: number;
            /** Format: uint32 */
            cpus: number;
            /** Format: uint64 */
            diskAvailableBytes: number | null;
            /** Format: uint64 */
            diskTotalBytes: number | null;
            loadAverage: number[] | null;
            /** Format: uint64 */
            memoryTotalMb: number;
            /** Format: uint64 */
            minFreeDiskMb: number;
            /** Format: uint32 */
            portsAllocated: number;
            /** Format: uint32 */
            portsTotal: number;
            /**
             * Format: uint64
             * @description Memory of every workload that holds compute here, running or not.
             */
            provisionedMemoryMb: number;
            /**
             * @description The data's filesystem shares blocks between copies: snapshots are nearly free.
             * @default null
             */
            reflink: boolean | null;
            /** Format: uint64 */
            reservedCpuMillis: number;
            /** Format: uint64 */
            reservedMemoryMb: number;
            /**
             * Format: uint64
             * @description Memory of running workloads: what a start is admitted against.
             */
            runningMemoryMb: number;
            /**
             * Format: uint64
             * @description What local snapshots hold on the data disk, as their files count it (shared blocks are
             *     counted in full, so it overstates what reflinked snapshots really take).
             * @default null
             */
            snapshotBytes: number | null;
            /**
             * Format: double
             * @description Cores running workloads use, likewise.
             */
            usedCpuCores: number | null;
            /**
             * Format: uint64
             * @description The working set of running workloads, as the kernel counts it, from each one's latest
             *     sample. A workload that runs but hasn't been sampled yet is left out; null only while
             *     workloads run and none has been.
             */
            usedMemoryBytes: number | null;
        };
        /**
         * @description What the node says about itself when it joins. The control plane records it; the token, not
         *     these facts, decides whether the node may join.
         */
        NodeFacts: {
            /** @description Where the control plane dials this node's API. */
            apiAddress: string;
            /** @description The kernel's boot id: a new one means the host rebooted. */
            bootId: string | null;
            capacity: components["schemas"]["NodeCapacity"];
            controlIps: string[];
            daemonVersion: string;
            /** @description Where the edge reaches game ports, and the control plane console and status ports. */
            edgeIps: string[];
            features: string[];
            hostname: string;
            labels: {
                [key: string]: string;
            };
            /** @description sha256 of /etc/machine-id. Two nodes with the same one are clones of one image. */
            machineIdSha256: string | null;
            protocol: components["schemas"]["ProtocolVersions"];
        };
        NodeHealth: {
            /** @description blocklyd's version: what a control plane reads before relying on a feature. */
            daemonVersion: string;
            deploymentId: string;
            docker: boolean;
            features: string[];
            nodeId: string;
            protocol: components["schemas"]["ProtocolVersions"];
            reconciled: boolean;
            status: string;
        };
        NodeStatus: {
            capacity: components["schemas"]["CapacityView"];
            /** Format: uint32 */
            cpus: number;
            daemon: components["schemas"]["DaemonView"];
            deploymentId: string;
            /** Format: uint64 */
            diskAvailableBytes: number | null;
            diskPath: string;
            /** Format: uint64 */
            diskTotalBytes: number | null;
            docker: components["schemas"]["DockerView"];
            /** @description In fleet mode: the control plane this node reports to, and when it last answered. */
            fleet: components["schemas"]["FleetView"] | null;
            hostname: string;
            issues: components["schemas"]["Issue"][];
            kernel: string;
            loadAverage: number[] | null;
            /** Format: uint64 */
            memoryAvailableBytes: number | null;
            /** Format: uint64 */
            memoryTotalBytes: number | null;
            nodeId: string;
            reconcile: components["schemas"]["ReconcileView"] | null;
            /** Format: uint64 */
            trashBytes: number | null;
            workloadsByState: {
                [key: string]: number;
            };
        };
        PortView: {
            /** Format: uint16 */
            containerPort: number;
            /**
             * @description `host:port` per audience, `[v6]:port` for IPv6: where the edge and the control plane
             *     reach this port. Blockly's `endpoint(handle, port, audience)`.
             */
            endpoints: components["schemas"]["Endpoints"];
            /** Format: uint16 */
            hostPort: number;
            name: string;
            protocol: components["schemas"]["Proto"];
        };
        PowerResponse: {
            /** @description False when it was already in the asked-for state: a repeat is a no-op, not an error. */
            changed: boolean;
            /**
             * @description For stop: the workload didn't exit within the grace and was killed.
             * @default false
             */
            forced: boolean;
            /**
             * @description For a start of a suspended workload: true when it came back from memory, false when the
             *     resume didn't come back in time and it was started afresh on its data. Absent otherwise.
             */
            resumed?: boolean | null;
            workload: components["schemas"]["WorkloadView"];
        };
        /** @enum {string} */
        Proto: "tcp" | "udp";
        ProtocolVersions: {
            /** Format: uint32 */
            current: number;
            supported: number[];
        };
        /**
         * @description A part the node put: its number, from 1, and the ETag the store answered with, which the
         *     control plane needs to finish the upload.
         */
        PutPart: {
            etag: string;
            /** Format: uint64 */
            number: number;
        };
        ReconcileView: {
            /** Format: uint64 */
            adopted: number;
            /** Format: uint64 */
            durationMs: number;
            error: string | null;
            finishedAt: string;
            /** Format: uint64 */
            issues: number;
            /** Format: uint64 */
            workloads: number;
        };
        /**
         * @description `POST /fleet/v1/nodes/{id}/renew`, over the node's current client certificate: a CSR for a new
         *     key. The node keeps its id; only its certificates and key change.
         */
        RenewRequest: {
            csrPem: string;
            nodeId: string;
        };
        Resources: {
            /**
             * Format: uint32
             * @description CPU ceiling in thousandths of a core (CFS quota). Also what the JVM counts as its
             *     processors. Absent: no ceiling, weight only.
             * @default null
             */
            cpuMillis: number | null;
            /**
             * Format: uint32
             * @description Share of CPU under contention, relative (cgroup weight). Absent: proportional to memory.
             * @default null
             */
            cpuWeight: number | null;
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
            pidsLimit: number | null;
        };
        RestoreResponse: {
            /** Format: uint64 */
            durationMs: number;
            /** Format: uint64 */
            entries: number;
            /** @description Where the data it replaced went (the trash, for its retention). */
            previousData: string | null;
            /** @description The archive's, for a restore from a URL; none for a snapshot. */
            sha256: string | null;
            /** Format: uint64 */
            sizeBytes: number;
            /**
             * Format: uint64
             * @description Links, devices and the like, which are never unpacked.
             */
            skipped: number;
            /** Format: uint64 */
            unpackedBytes: number;
        };
        SnapshotDeleteResponse: {
            existed: boolean;
        };
        /**
         * @description A local snapshot's id: the control plane's archive id (a UUID). It names a directory, so it is
         *     held to a workload id's rules.
         */
        SnapshotId: string;
        SnapshotList: {
            snapshots: components["schemas"]["SnapshotView"][];
        };
        SnapshotResponse: {
            /** @description False when a snapshot with this id already existed; it is returned as it was made. */
            created: boolean;
            snapshot: components["schemas"]["SnapshotView"];
        };
        SnapshotView: {
            createdAt: string;
            /** Format: uint64 */
            durationMs: number;
            /**
             * Format: uint64
             * @description The epoch of the copy it was taken from.
             */
            epoch: number | null;
            /** Format: uint64 */
            files: number;
            id: components["schemas"]["SnapshotId"];
            method: components["schemas"]["Method"];
            /** @description The workload was running, with its saving paused, when it was taken. */
            quiesced: boolean;
            /**
             * Format: uint64
             * @description The files' bytes. With shared blocks, most of them aren't new on disk.
             */
            sizeBytes: number;
            specDigest: string;
            workload: components["schemas"]["WorkloadId"];
        };
        StatsView: {
            at: string;
            /**
             * Format: double
             * @description Cores in use over the last sampling interval (1.0 = one full core).
             */
            cpuCores: number | null;
            /** Format: uint32 */
            cpuLimitMillis: number | null;
            /** Format: double */
            cpuSecondsTotal: number | null;
            /** Format: uint64 */
            cpuThrottledPeriods: number | null;
            /** Format: uint64 */
            dataUsedBytes: number | null;
            /** Format: uint64 */
            memoryBytes: number | null;
            /** Format: uint64 */
            memoryLimitBytes: number | null;
            /**
             * Format: uint64
             * @description Usage minus inactive page cache: what the kernel would have to reclaim to fit.
             */
            memoryWorkingSetBytes: number | null;
            /** Format: uint64 */
            networkRxBytes: number | null;
            /** Format: uint64 */
            networkTxBytes: number | null;
            /** Format: uint64 */
            pids: number | null;
            /** Format: uint64 */
            pidsLimit: number | null;
            state: components["schemas"]["WorkloadState"];
            /** Format: uint64 */
            uptimeSeconds: number | null;
        };
        StorageView: {
            measuredAt: string | null;
            mountPath: string;
            /** Format: uint32 */
            sizeGb: number;
            /** Format: uint64 */
            usedBytes: number | null;
        };
        UpgradeFailure: {
            reason: string;
            version: string;
        };
        /**
         * @description What the control plane calls a workload: Blockly's server id (a UUID) in practice, but any
         *     lowercase DNS label will do. The character set is the whole defence against path traversal
         *     and injection: a valid id is always one safe path component, one safe container-name suffix
         *     and one safe label value, so nothing downstream needs to escape it.
         */
        WorkloadId: string;
        WorkloadReport: {
            changedAt: string;
            /** Format: uint64 */
            epoch: number | null;
            exit: components["schemas"]["ExitInfo"] | null;
            /** Format: uint64 */
            generation: number;
            id: string;
            /**
             * @description What the node found wrong with the workload and can't mend itself: a restart refused for
             *     want of room, data over the size its spec promised. Left out when there is none.
             */
            issues?: components["schemas"]["Issue"][];
            lastFailureAt: string | null;
            /** Format: uint32 */
            memoryMb: number;
            /** @description Port name → host port. */
            ports: {
                [key: string]: number;
            };
            /** Format: uint32 */
            restartCount: number;
            specDigest: string;
            state: components["schemas"]["WorkloadState"];
            /** Format: uint64 */
            supersededBy: number | null;
        };
        /**
         * @description What a workload is doing, normalized. How it maps onto Blockly's `ObservedState`:
         *     created/stopped → stopped · running → running · stopping → stopping · crashed → crashed ·
         *     restarting → starting (with `lastFailureAt`) · missing/retained → absent · creating → stopped
         *     · unknown → unknown · fenced → stopped (this copy lost its placement; the current one is
         *     elsewhere).
         */
        WorkloadState: "running" | "creating" | "created" | "stopping" | "stopped" | "crashed" | "restarting" | "missing" | "retained" | "unknown" | "fenced" | "suspended";
        WorkloadView: {
            /** @description When the state last changed, as the runtime recorded it (survives blocklyd restarts). */
            changedAt: string;
            createdAt: string;
            /**
             * Format: uint64
             * @description The placement epoch this copy belongs to, if the control plane gave one.
             */
            epoch: number | null;
            exit: components["schemas"]["ExitInfo"] | null;
            finishedAt: string | null;
            /**
             * Format: uint64
             * @description Bumped each time the container is made again for a new spec.
             */
            generation: number;
            health: components["schemas"]["Health"] | null;
            id: components["schemas"]["WorkloadId"];
            image: string;
            issues: components["schemas"]["Issue"][];
            labels: {
                [key: string]: string;
            };
            /** @description When it last failed and the host started it again (Blockly's `failedAt`). */
            lastFailureAt: string | null;
            locate: components["schemas"]["Locate"];
            /**
             * @description A spec a `PUT` deferred while the workload is suspended (`EnsureOutcome::Deferred`): put in
             *     place at its next boot. Absent when there is none.
             */
            pendingSpecDigest?: string | null;
            ports: components["schemas"]["PortView"][];
            resources: components["schemas"]["Resources"];
            /**
             * Format: uint32
             * @description Restarts the host made after failures since the last requested start or new spec. Kept
             *     across restarts of blocklyd.
             */
            restartCount: number;
            secretNames: string[];
            specDigest: string;
            startedAt: string | null;
            state: components["schemas"]["WorkloadState"];
            storage: components["schemas"]["StorageView"];
            /**
             * Format: uint64
             * @description Set once a newer placement exists: the epoch that superseded this copy.
             */
            supersededBy: number | null;
            updatedAt: string;
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
