// SPDX-License-Identifier: FSL-1.1-ALv2
// Generated from the pinned provider contract.
export interface paths {
    "/sandboxes": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["sandboxes"];
        put?: never;
        post: operations["create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/sandboxes/{sandboxId}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get"];
        put?: never;
        post?: never;
        delete: operations["deleteSandbox"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/sandboxes/{sandboxId}/commands": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["command"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/sandboxes/{sandboxId}/files": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put: operations["writeFile"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        Sandbox: {
            id: string;
            name: string;
            /** @enum {string} */
            state: "init" | "provisioning" | "provisioned" | "cloning" | "ready" | "idle" | "running" | "archiving" | "archived" | "error" | "cancelled";
            labels?: {
                [key: string]: string;
            };
            /** @enum {string} */
            type?: "small" | "default" | "large" | "xlarge";
            vcpu?: number;
            memoryGB?: number;
            ip?: string | null;
            desktopAvailable: boolean;
            snapshotAvailable: boolean;
        };
        CreateSandboxRequest: {
            /**
             * @default default
             * @enum {string}
             */
            type?: "small" | "default" | "large" | "xlarge";
            /** @default 3600 */
            ttlSeconds?: number | null;
            labels?: {
                [key: string]: string;
            };
            /** @default false */
            noEnv?: boolean;
            name?: string;
        };
        SandboxListResponse: components["schemas"]["SuccessBase"] & {
            /** @constant */
            type?: "sandbox.list";
            sandboxes: components["schemas"]["Sandbox"][];
            pageInfo?: components["schemas"]["PageInfo"];
            deletedCost?: {
                /** Format: date-time */
                since: string;
                seconds: number;
                dollars: number;
            };
        };
        CreateSandboxResponse: components["schemas"]["SuccessBase"] & {
            /** @constant */
            type?: "sandbox.created";
            /** @enum {string} */
            status: "provisioning" | "ready";
            ttlSeconds: number | null;
            sandbox: components["schemas"]["Sandbox"];
            sandboxes?: components["schemas"]["Sandbox"][];
            hostedPorts?: ({
                port: number;
                /** Format: uri */
                url?: string;
                isProtected?: boolean;
                /** @enum {string} */
                access?: "private" | "public";
                localhost?: boolean;
                error?: string;
                message?: string;
            } & {
                [key: string]: unknown;
            })[];
            giftLimitNotice?: string;
        };
        SandboxInfoResponse: components["schemas"]["SuccessBase"] & {
            type?: string;
            sandbox: components["schemas"]["Sandbox"];
            giftLimitNotice?: string;
        };
        DeletionOperationResponse: components["schemas"]["SuccessBase"] & {
            /** @enum {string} */
            type?: "deletion.operation" | "sandbox.deleting" | "snapshot.deleting";
            operation: components["schemas"]["DeletionOperation"];
        };
        CommandRequest: {
            command: string;
            cwd?: string;
            /** @default 30 */
            timeoutSeconds?: number;
            /** @default false */
            detached?: boolean;
            /** @default false */
            stream?: boolean;
        };
        CommandResponse: components["schemas"]["SuccessBase"] & {
            /** @constant */
            type?: "command.finished";
            success: boolean;
            commandId?: string;
            exitCode: number | null;
            signal?: string | null;
            oomKilled?: boolean;
            stdout: string;
            stderr: string;
            stdoutTruncated?: boolean;
            stderrTruncated?: boolean;
            timedOut: boolean;
            cwd?: string;
            /** Format: date-time */
            startedAt?: string;
            /** Format: date-time */
            finishedAt?: string;
        };
        CommandStartedResponse: components["schemas"]["SuccessBase"] & {
            /** @constant */
            type?: "command.started";
            success: boolean;
            commandId?: string;
            processId: number;
            pid: number;
            command: string;
            cwd?: string;
            /** Format: date-time */
            startedAt: string;
            logPath?: string;
            errLogPath?: string;
        };
        CommandStreamFrame: {
            /** @enum {string} */
            type: "started" | "stdout" | "stderr" | "exit" | "error";
            commandId?: string;
            data?: string;
            exitCode?: number | null;
            success?: boolean;
            timedOut?: boolean;
            error?: string;
            message?: string;
            retryable?: boolean;
        };
        FileWriteRequest: {
            path: string;
            content: string;
            /**
             * @default utf8
             * @enum {string}
             */
            encoding?: "utf8" | "base64";
        };
        FileWriteResponse: components["schemas"]["SuccessBase"] & {
            /** @constant */
            type?: "file.written";
            success: boolean;
            path: string;
            /** @enum {string} */
            encoding: "utf8" | "base64";
            size: number;
        };
        SuccessBase: {
            ok: boolean;
            type: string;
        };
        PageInfo: {
            nextCursor: string | null;
            hasMore: boolean;
            limit: number;
        };
        ErrorEnvelope: {
            ok: boolean;
            type: string;
            status: number;
            code: string;
            message: string;
            requestId: string;
            error: {
                code: string;
                message: string;
                status: number;
                details?: {
                    [key: string]: unknown;
                };
            };
        };
        DeletionOperation: {
            id: string;
            /** @enum {string} */
            kind: "sandbox" | "snapshot";
            targetId: string;
            /** @enum {string} */
            reason: "explicit" | "zdr" | "account";
            /** @enum {string} */
            status: "pending" | "processing" | "blocked" | "completed";
            /** @enum {string} */
            stage?: "removing" | "waiting_for_uploads" | "retrying" | "completed";
            /** Format: date-time */
            expectedBy?: string | null;
            attemptCount: number;
            /** Format: date-time */
            requestedAt: string;
            /** Format: date-time */
            completedAt: string | null;
        };
    };
    responses: {
        Unauthorized: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        PaymentRequired: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        EnvironmentForbidden: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        Conflict: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        RateLimited: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        NoReadyMachine: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        NotFound: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        Forbidden: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        BadRequest: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
    };
    parameters: {
        OrgId: string;
        OrgHeader: string;
        Limit: number;
        Cursor: string | null;
        Sort: "asc" | "desc";
        IdempotencyKey: string;
        ConfirmDelete: string;
        SandboxId: string;
    };
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    sandboxes: {
        parameters: {
            query?: {
                org?: components["parameters"]["OrgId"];
                limit?: components["parameters"]["Limit"];
                cursor?: components["parameters"]["Cursor"];
                sort?: components["parameters"]["Sort"];
                state?: string;
                namePrefix?: string;
                stoppedBefore?: string;
                dataExpired?: boolean;
                label?: string[];
                include?: "cost";
            };
            header?: {
                "X-Boat-Org"?: components["parameters"]["OrgHeader"];
            };
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SandboxListResponse"];
                };
            };
            401: components["responses"]["Unauthorized"];
        };
    };
    create: {
        parameters: {
            query?: {
                org?: components["parameters"]["OrgId"];
            };
            header?: {
                "Idempotency-Key"?: components["parameters"]["IdempotencyKey"];
                "X-Boat-Org"?: components["parameters"]["OrgHeader"];
            };
            path?: never;
            cookie?: never;
        };
        requestBody?: {
            content: {
                "application/json": components["schemas"]["CreateSandboxRequest"];
            };
        };
        responses: {
            202: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CreateSandboxResponse"];
                };
            };
            401: components["responses"]["Unauthorized"];
            402: components["responses"]["PaymentRequired"];
            403: components["responses"]["EnvironmentForbidden"];
            409: components["responses"]["Conflict"];
            429: components["responses"]["RateLimited"];
            503: components["responses"]["NoReadyMachine"];
        };
    };
    get: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SandboxInfoResponse"];
                };
            };
            401: components["responses"]["Unauthorized"];
            404: components["responses"]["NotFound"];
        };
    };
    deleteSandbox: {
        parameters: {
            query?: never;
            header: {
                "X-Ascii-Confirm-Delete": components["parameters"]["ConfirmDelete"];
            };
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            202: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DeletionOperationResponse"];
                };
            };
            401: components["responses"]["Unauthorized"];
            403: components["responses"]["Forbidden"];
            404: components["responses"]["NotFound"];
            409: components["responses"]["Conflict"];
        };
    };
    command: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                sandboxId: components["parameters"]["SandboxId"];
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CommandRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CommandResponse"] | components["schemas"]["CommandStartedResponse"];
                    "application/x-ndjson": components["schemas"]["CommandStreamFrame"];
                };
            };
            400: components["responses"]["BadRequest"];
            401: components["responses"]["Unauthorized"];
            404: components["responses"]["NotFound"];
            409: components["responses"]["Conflict"];
        };
    };
    writeFile: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                sandboxId: components["parameters"]["SandboxId"];
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["FileWriteRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["FileWriteResponse"];
                };
            };
            400: components["responses"]["BadRequest"];
        };
    };
}
