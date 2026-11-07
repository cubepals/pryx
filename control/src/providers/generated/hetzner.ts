// SPDX-License-Identifier: FSL-1.1-ALv2
// Generated from the pinned provider contract.
export interface paths {
    "/servers": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list_servers"];
        put?: never;
        post: operations["create_server"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/servers/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_server"];
        put?: never;
        post?: never;
        delete: operations["delete_server"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/server_types": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list_server_types"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/pricing": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_pricing"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/networks": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["create_network"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/networks/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete: operations["delete_network"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/firewalls": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["create_firewall"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/firewalls/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete: operations["delete_firewall"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/ssh_keys": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["create_ssh_key"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/ssh_keys/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete: operations["delete_ssh_key"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: never;
    responses: never;
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    list_servers: {
        parameters: {
            query?: {
                name?: string;
                label_selector?: string;
                sort?: ("id" | "id:asc" | "id:desc" | "name" | "name:asc" | "name:desc" | "created" | "created:asc" | "created:desc")[];
                status?: ("running" | "initializing" | "starting" | "stopping" | "off" | "deleting" | "migrating" | "rebuilding" | "unknown")[];
                page?: number;
                per_page?: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    "x-next"?: string;
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        servers: {
                            /** Format: int64 */
                            id: number;
                            name: string;
                            /** @enum {string} */
                            status: "running" | "initializing" | "starting" | "stopping" | "off" | "deleting" | "migrating" | "rebuilding" | "unknown";
                            /** Format: date-time */
                            created: string;
                            public_net: {
                                ipv4: {
                                    /** Format: int64 */
                                    id?: number;
                                    ip: string;
                                    blocked: boolean;
                                    dns_ptr: string;
                                } | null;
                                ipv6: {
                                    /** Format: int64 */
                                    id?: number;
                                    ip: string;
                                    blocked: boolean;
                                    dns_ptr: {
                                        ip: string;
                                        dns_ptr: string;
                                    }[] | null;
                                } | null;
                                floating_ips: number[];
                                firewalls?: {
                                    /** Format: int64 */
                                    id?: number;
                                    /** @enum {string} */
                                    status?: "applied" | "pending";
                                }[];
                            };
                            private_net: {
                                /** Format: int64 */
                                network?: number;
                                ip?: string;
                                alias_ips?: string[];
                                mac_address?: string;
                            }[];
                            server_type: {
                                /** Format: int64 */
                                id: number;
                                name: string;
                                cores: number;
                                memory: number;
                                disk: number;
                                prices: {
                                    location: string;
                                    price_hourly: {
                                        /** Format: decimal */
                                        net: string;
                                        /** Format: decimal */
                                        gross: string;
                                    };
                                    price_monthly: {
                                        /** Format: decimal */
                                        net: string;
                                        /** Format: decimal */
                                        gross: string;
                                    };
                                    /** Format: int64 */
                                    included_traffic: number;
                                    price_per_tb_traffic: {
                                        /** Format: decimal */
                                        net: string;
                                        /** Format: decimal */
                                        gross: string;
                                    };
                                }[];
                                /** @enum {string} */
                                storage_type: "local" | "network";
                                /** @enum {string} */
                                cpu_type: "shared" | "dedicated";
                                category?: string;
                                /** @enum {string} */
                                architecture: "x86" | "arm";
                                /**
                                 * DeprecationInfo
                                 * @deprecated
                                 */
                                deprecation?: {
                                    /** Format: date-time */
                                    unavailable_after: string;
                                    /** Format: date-time */
                                    announced: string;
                                } | null;
                                locations: {
                                    /** Format: int64 */
                                    id: number;
                                    name: string;
                                    /** DeprecationInfo */
                                    deprecation: {
                                        /** Format: date-time */
                                        unavailable_after: string;
                                        /** Format: date-time */
                                        announced: string;
                                    } | null;
                                    recommended: boolean;
                                    available: boolean;
                                }[];
                            };
                            location: {
                                /** Format: int64 */
                                id: number;
                                name: string;
                                country: string;
                                city: string;
                                /** Format: double */
                                latitude: number;
                                /** Format: double */
                                longitude: number;
                                network_zone: string;
                            };
                            image: {
                                /** Format: int64 */
                                id: number;
                                /** @enum {string} */
                                type: "system" | "app" | "snapshot" | "backup";
                                /** @enum {string} */
                                status: "available" | "creating" | "unavailable";
                                name: string | null;
                                image_size: number | null;
                                disk_size: number;
                                /** Format: date-time */
                                created: string;
                                created_from: {
                                    /** Format: int64 */
                                    id: number;
                                    name: string;
                                } | null;
                                /** Format: int64 */
                                bound_to: number | null;
                                /** @enum {string} */
                                os_flavor: "ubuntu" | "centos" | "debian" | "fedora" | "rocky" | "alma" | "opensuse" | "unknown";
                                os_version: string | null;
                                rapid_deploy?: boolean;
                                protection: {
                                    delete: boolean;
                                };
                                /** DeprecationInfo */
                                deprecation: {
                                    /** Format: date-time */
                                    unavailable_after: string;
                                    /** Format: date-time */
                                    announced: string;
                                } | null;
                                deleted: string | null;
                                labels: {
                                    [key: string]: string;
                                };
                                /** @enum {string} */
                                architecture: "x86" | "arm";
                            } | null;
                            iso: {
                                /** Format: int64 */
                                id: number;
                                name: string | null;
                                /** @enum {string|null} */
                                type: "public" | "private" | null;
                                /** DeprecationInfo */
                                deprecation: {
                                    /** Format: date-time */
                                    unavailable_after: string;
                                    /** Format: date-time */
                                    announced: string;
                                } | null;
                                /** @enum {string|null} */
                                architecture: "x86" | "arm" | null;
                            } | null;
                            rescue_enabled: boolean;
                            locked: boolean;
                            backup_window: string | null;
                            /** Format: int64 */
                            outgoing_traffic: number | null;
                            /** Format: int64 */
                            ingoing_traffic: number | null;
                            /** Format: int64 */
                            included_traffic: number | null;
                            protection: {
                                delete: boolean;
                                rebuild: boolean;
                            };
                            labels: {
                                [key: string]: string;
                            };
                            volumes?: number[];
                            load_balancers?: number[];
                            primary_disk_size: number;
                            /** PlacementGroupNullable */
                            placement_group?: {
                                /** Format: int64 */
                                id: number;
                                name: string;
                                labels: {
                                    [key: string]: string;
                                };
                                /** @enum {string} */
                                type: "spread";
                                /** Format: date-time */
                                created: string;
                                servers: number[];
                            } | null;
                        }[];
                        /** ListMeta */
                        meta: {
                            pagination: {
                                /**
                                 * Format: int64
                                 * @default 1
                                 */
                                page: number;
                                /**
                                 * Format: int64
                                 * @default 25
                                 */
                                per_page: number;
                                /** Format: int64 */
                                previous_page: number | null;
                                /** Format: int64 */
                                next_page: number | null;
                                /** Format: int64 */
                                last_page: number | null;
                                /** Format: int64 */
                                total_entries: number | null;
                            };
                        };
                    };
                };
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
    create_server: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": {
                    name: string;
                    location?: string;
                    server_type: string;
                    /** @default true */
                    start_after_create?: boolean;
                    image: string;
                    /** Format: int64 */
                    placement_group?: number;
                    ssh_keys?: string[];
                    volumes?: number[];
                    networks?: number[];
                    firewalls?: {
                        /** Format: int64 */
                        firewall: number;
                    }[];
                    user_data?: string;
                    labels?: {
                        [key: string]: string;
                    };
                    automount?: boolean;
                    public_net?: {
                        /** @default true */
                        enable_ipv4?: boolean;
                        /** @default true */
                        enable_ipv6?: boolean;
                        ipv4?: number | null;
                        ipv6?: number | null;
                    };
                };
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        server: {
                            /** Format: int64 */
                            id: number;
                            name: string;
                            /** @enum {string} */
                            status: "running" | "initializing" | "starting" | "stopping" | "off" | "deleting" | "migrating" | "rebuilding" | "unknown";
                            /** Format: date-time */
                            created: string;
                            public_net: {
                                ipv4: {
                                    /** Format: int64 */
                                    id?: number;
                                    ip: string;
                                    blocked: boolean;
                                    dns_ptr: string;
                                } | null;
                                ipv6: {
                                    /** Format: int64 */
                                    id?: number;
                                    ip: string;
                                    blocked: boolean;
                                    dns_ptr: {
                                        ip: string;
                                        dns_ptr: string;
                                    }[] | null;
                                } | null;
                                floating_ips: number[];
                                firewalls?: {
                                    /** Format: int64 */
                                    id?: number;
                                    /** @enum {string} */
                                    status?: "applied" | "pending";
                                }[];
                            };
                            private_net: {
                                /** Format: int64 */
                                network?: number;
                                ip?: string;
                                alias_ips?: string[];
                                mac_address?: string;
                            }[];
                            server_type: {
                                /** Format: int64 */
                                id: number;
                                name: string;
                                cores: number;
                                memory: number;
                                disk: number;
                                prices: {
                                    location: string;
                                    price_hourly: {
                                        /** Format: decimal */
                                        net: string;
                                        /** Format: decimal */
                                        gross: string;
                                    };
                                    price_monthly: {
                                        /** Format: decimal */
                                        net: string;
                                        /** Format: decimal */
                                        gross: string;
                                    };
                                    /** Format: int64 */
                                    included_traffic: number;
                                    price_per_tb_traffic: {
                                        /** Format: decimal */
                                        net: string;
                                        /** Format: decimal */
                                        gross: string;
                                    };
                                }[];
                                /** @enum {string} */
                                storage_type: "local" | "network";
                                /** @enum {string} */
                                cpu_type: "shared" | "dedicated";
                                category?: string;
                                /** @enum {string} */
                                architecture: "x86" | "arm";
                                /**
                                 * DeprecationInfo
                                 * @deprecated
                                 */
                                deprecation?: {
                                    /** Format: date-time */
                                    unavailable_after: string;
                                    /** Format: date-time */
                                    announced: string;
                                } | null;
                                locations: {
                                    /** Format: int64 */
                                    id: number;
                                    name: string;
                                    /** DeprecationInfo */
                                    deprecation: {
                                        /** Format: date-time */
                                        unavailable_after: string;
                                        /** Format: date-time */
                                        announced: string;
                                    } | null;
                                    recommended: boolean;
                                    available: boolean;
                                }[];
                            };
                            location: {
                                /** Format: int64 */
                                id: number;
                                name: string;
                                country: string;
                                city: string;
                                /** Format: double */
                                latitude: number;
                                /** Format: double */
                                longitude: number;
                                network_zone: string;
                            };
                            image: {
                                /** Format: int64 */
                                id: number;
                                /** @enum {string} */
                                type: "system" | "app" | "snapshot" | "backup";
                                /** @enum {string} */
                                status: "available" | "creating" | "unavailable";
                                name: string | null;
                                image_size: number | null;
                                disk_size: number;
                                /** Format: date-time */
                                created: string;
                                created_from: {
                                    /** Format: int64 */
                                    id: number;
                                    name: string;
                                } | null;
                                /** Format: int64 */
                                bound_to: number | null;
                                /** @enum {string} */
                                os_flavor: "ubuntu" | "centos" | "debian" | "fedora" | "rocky" | "alma" | "opensuse" | "unknown";
                                os_version: string | null;
                                rapid_deploy?: boolean;
                                protection: {
                                    delete: boolean;
                                };
                                /** DeprecationInfo */
                                deprecation: {
                                    /** Format: date-time */
                                    unavailable_after: string;
                                    /** Format: date-time */
                                    announced: string;
                                } | null;
                                deleted: string | null;
                                labels: {
                                    [key: string]: string;
                                };
                                /** @enum {string} */
                                architecture: "x86" | "arm";
                            } | null;
                            iso: {
                                /** Format: int64 */
                                id: number;
                                name: string | null;
                                /** @enum {string|null} */
                                type: "public" | "private" | null;
                                /** DeprecationInfo */
                                deprecation: {
                                    /** Format: date-time */
                                    unavailable_after: string;
                                    /** Format: date-time */
                                    announced: string;
                                } | null;
                                /** @enum {string|null} */
                                architecture: "x86" | "arm" | null;
                            } | null;
                            rescue_enabled: boolean;
                            locked: boolean;
                            backup_window: string | null;
                            /** Format: int64 */
                            outgoing_traffic: number | null;
                            /** Format: int64 */
                            ingoing_traffic: number | null;
                            /** Format: int64 */
                            included_traffic: number | null;
                            protection: {
                                delete: boolean;
                                rebuild: boolean;
                            };
                            labels: {
                                [key: string]: string;
                            };
                            volumes?: number[];
                            load_balancers?: number[];
                            primary_disk_size: number;
                            /** PlacementGroupNullable */
                            placement_group?: {
                                /** Format: int64 */
                                id: number;
                                name: string;
                                labels: {
                                    [key: string]: string;
                                };
                                /** @enum {string} */
                                type: "spread";
                                /** Format: date-time */
                                created: string;
                                servers: number[];
                            } | null;
                        };
                        /** Action */
                        action: {
                            /** Format: int64 */
                            id: number;
                            command: string;
                            /** @enum {string} */
                            status: "running" | "success" | "error";
                            /** Format: date-time */
                            started: string;
                            /** Format: date-time */
                            finished: string | null;
                            /** Format: int32 */
                            progress: number;
                            resources: {
                                /** Format: int64 */
                                id: number;
                                type: string;
                            }[];
                            error: {
                                code: string;
                                message: string;
                            } | null;
                        };
                        next_actions: {
                            /** Format: int64 */
                            id: number;
                            command: string;
                            /** @enum {string} */
                            status: "running" | "success" | "error";
                            /** Format: date-time */
                            started: string;
                            /** Format: date-time */
                            finished: string | null;
                            /** Format: int32 */
                            progress: number;
                            resources: {
                                /** Format: int64 */
                                id: number;
                                type: string;
                            }[];
                            error: {
                                code: string;
                                message: string;
                            } | null;
                        }[];
                        root_password: string | null;
                    };
                };
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
    get_server: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        server?: {
                            /** Format: int64 */
                            id: number;
                            name: string;
                            /** @enum {string} */
                            status: "running" | "initializing" | "starting" | "stopping" | "off" | "deleting" | "migrating" | "rebuilding" | "unknown";
                            /** Format: date-time */
                            created: string;
                            public_net: {
                                ipv4: {
                                    /** Format: int64 */
                                    id?: number;
                                    ip: string;
                                    blocked: boolean;
                                    dns_ptr: string;
                                } | null;
                                ipv6: {
                                    /** Format: int64 */
                                    id?: number;
                                    ip: string;
                                    blocked: boolean;
                                    dns_ptr: {
                                        ip: string;
                                        dns_ptr: string;
                                    }[] | null;
                                } | null;
                                floating_ips: number[];
                                firewalls?: {
                                    /** Format: int64 */
                                    id?: number;
                                    /** @enum {string} */
                                    status?: "applied" | "pending";
                                }[];
                            };
                            private_net: {
                                /** Format: int64 */
                                network?: number;
                                ip?: string;
                                alias_ips?: string[];
                                mac_address?: string;
                            }[];
                            server_type: {
                                /** Format: int64 */
                                id: number;
                                name: string;
                                cores: number;
                                memory: number;
                                disk: number;
                                prices: {
                                    location: string;
                                    price_hourly: {
                                        /** Format: decimal */
                                        net: string;
                                        /** Format: decimal */
                                        gross: string;
                                    };
                                    price_monthly: {
                                        /** Format: decimal */
                                        net: string;
                                        /** Format: decimal */
                                        gross: string;
                                    };
                                    /** Format: int64 */
                                    included_traffic: number;
                                    price_per_tb_traffic: {
                                        /** Format: decimal */
                                        net: string;
                                        /** Format: decimal */
                                        gross: string;
                                    };
                                }[];
                                /** @enum {string} */
                                storage_type: "local" | "network";
                                /** @enum {string} */
                                cpu_type: "shared" | "dedicated";
                                category?: string;
                                /** @enum {string} */
                                architecture: "x86" | "arm";
                                /**
                                 * DeprecationInfo
                                 * @deprecated
                                 */
                                deprecation?: {
                                    /** Format: date-time */
                                    unavailable_after: string;
                                    /** Format: date-time */
                                    announced: string;
                                } | null;
                                locations: {
                                    /** Format: int64 */
                                    id: number;
                                    name: string;
                                    /** DeprecationInfo */
                                    deprecation: {
                                        /** Format: date-time */
                                        unavailable_after: string;
                                        /** Format: date-time */
                                        announced: string;
                                    } | null;
                                    recommended: boolean;
                                    available: boolean;
                                }[];
                            };
                            location: {
                                /** Format: int64 */
                                id: number;
                                name: string;
                                country: string;
                                city: string;
                                /** Format: double */
                                latitude: number;
                                /** Format: double */
                                longitude: number;
                                network_zone: string;
                            };
                            image: {
                                /** Format: int64 */
                                id: number;
                                /** @enum {string} */
                                type: "system" | "app" | "snapshot" | "backup";
                                /** @enum {string} */
                                status: "available" | "creating" | "unavailable";
                                name: string | null;
                                image_size: number | null;
                                disk_size: number;
                                /** Format: date-time */
                                created: string;
                                created_from: {
                                    /** Format: int64 */
                                    id: number;
                                    name: string;
                                } | null;
                                /** Format: int64 */
                                bound_to: number | null;
                                /** @enum {string} */
                                os_flavor: "ubuntu" | "centos" | "debian" | "fedora" | "rocky" | "alma" | "opensuse" | "unknown";
                                os_version: string | null;
                                rapid_deploy?: boolean;
                                protection: {
                                    delete: boolean;
                                };
                                /** DeprecationInfo */
                                deprecation: {
                                    /** Format: date-time */
                                    unavailable_after: string;
                                    /** Format: date-time */
                                    announced: string;
                                } | null;
                                deleted: string | null;
                                labels: {
                                    [key: string]: string;
                                };
                                /** @enum {string} */
                                architecture: "x86" | "arm";
                            } | null;
                            iso: {
                                /** Format: int64 */
                                id: number;
                                name: string | null;
                                /** @enum {string|null} */
                                type: "public" | "private" | null;
                                /** DeprecationInfo */
                                deprecation: {
                                    /** Format: date-time */
                                    unavailable_after: string;
                                    /** Format: date-time */
                                    announced: string;
                                } | null;
                                /** @enum {string|null} */
                                architecture: "x86" | "arm" | null;
                            } | null;
                            rescue_enabled: boolean;
                            locked: boolean;
                            backup_window: string | null;
                            /** Format: int64 */
                            outgoing_traffic: number | null;
                            /** Format: int64 */
                            ingoing_traffic: number | null;
                            /** Format: int64 */
                            included_traffic: number | null;
                            protection: {
                                delete: boolean;
                                rebuild: boolean;
                            };
                            labels: {
                                [key: string]: string;
                            };
                            volumes?: number[];
                            load_balancers?: number[];
                            primary_disk_size: number;
                            /** PlacementGroupNullable */
                            placement_group?: {
                                /** Format: int64 */
                                id: number;
                                name: string;
                                labels: {
                                    [key: string]: string;
                                };
                                /** @enum {string} */
                                type: "spread";
                                /** Format: date-time */
                                created: string;
                                servers: number[];
                            } | null;
                        };
                    };
                };
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
    delete_server: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        /** Action */
                        action?: {
                            /** Format: int64 */
                            id: number;
                            command: string;
                            /** @enum {string} */
                            status: "running" | "success" | "error";
                            /** Format: date-time */
                            started: string;
                            /** Format: date-time */
                            finished: string | null;
                            /** Format: int32 */
                            progress: number;
                            resources: {
                                /** Format: int64 */
                                id: number;
                                type: string;
                            }[];
                            error: {
                                code: string;
                                message: string;
                            } | null;
                        };
                    };
                };
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
    list_server_types: {
        parameters: {
            query?: {
                name?: string;
                page?: number;
                per_page?: number;
            };
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
                    "application/json": {
                        server_types: {
                            /** Format: int64 */
                            id: number;
                            name: string;
                            cores: number;
                            memory: number;
                            disk: number;
                            prices: {
                                location: string;
                                price_hourly: {
                                    /** Format: decimal */
                                    net: string;
                                    /** Format: decimal */
                                    gross: string;
                                };
                                price_monthly: {
                                    /** Format: decimal */
                                    net: string;
                                    /** Format: decimal */
                                    gross: string;
                                };
                                /** Format: int64 */
                                included_traffic: number;
                                price_per_tb_traffic: {
                                    /** Format: decimal */
                                    net: string;
                                    /** Format: decimal */
                                    gross: string;
                                };
                            }[];
                            /** @enum {string} */
                            storage_type: "local" | "network";
                            /** @enum {string} */
                            cpu_type: "shared" | "dedicated";
                            category?: string;
                            /** @enum {string} */
                            architecture: "x86" | "arm";
                            /**
                             * DeprecationInfo
                             * @deprecated
                             */
                            deprecation?: {
                                /** Format: date-time */
                                unavailable_after: string;
                                /** Format: date-time */
                                announced: string;
                            } | null;
                            locations: {
                                /** Format: int64 */
                                id: number;
                                name: string;
                                /** DeprecationInfo */
                                deprecation: {
                                    /** Format: date-time */
                                    unavailable_after: string;
                                    /** Format: date-time */
                                    announced: string;
                                } | null;
                                recommended: boolean;
                                available: boolean;
                            }[];
                        }[];
                        /** ListMeta */
                        meta: {
                            pagination: {
                                /**
                                 * Format: int64
                                 * @default 1
                                 */
                                page: number;
                                /**
                                 * Format: int64
                                 * @default 25
                                 */
                                per_page: number;
                                /** Format: int64 */
                                previous_page: number | null;
                                /** Format: int64 */
                                next_page: number | null;
                                /** Format: int64 */
                                last_page: number | null;
                                /** Format: int64 */
                                total_entries: number | null;
                            };
                        };
                    };
                };
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
    get_pricing: {
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
                    "application/json": {
                        pricing: {
                            currency: string;
                        };
                    };
                };
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
    create_network: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": {
                    name: string;
                    ip_range: string;
                    labels?: {
                        [key: string]: string;
                    };
                    subnets?: {
                        /** @enum {string} */
                        type: "cloud" | "server" | "vswitch";
                        ip_range?: string;
                        network_zone: string;
                        /** Format: int64 */
                        vswitch_id?: number;
                    }[];
                    routes?: {
                        destination: string;
                        gateway: string;
                    }[];
                    expose_routes_to_vswitch?: boolean;
                };
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        network?: {
                            /** Format: int64 */
                            id: number;
                            name: string;
                            ip_range: string;
                            subnets: {
                                /** @enum {string} */
                                type: "cloud" | "server" | "vswitch";
                                ip_range?: string;
                                network_zone: string;
                                gateway: string;
                                /** Format: int64 */
                                vswitch_id?: number | null;
                            }[];
                            routes: {
                                destination: string;
                                gateway: string;
                            }[];
                            servers: number[];
                            load_balancers?: number[];
                            protection: {
                                delete: boolean;
                            };
                            labels: {
                                [key: string]: string;
                            };
                            /** Format: date-time */
                            created: string;
                            expose_routes_to_vswitch: boolean;
                        };
                    };
                };
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
    delete_network: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
    create_firewall: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": {
                    name: string;
                    labels?: {
                        [key: string]: string;
                    };
                    rules?: {
                        /** @enum {string} */
                        direction: "in" | "out";
                        source_ips?: string[];
                        destination_ips?: string[];
                        /** @enum {string} */
                        protocol: "tcp" | "udp" | "icmp" | "esp" | "gre";
                        port?: string;
                    }[];
                    apply_to?: {
                        /** @enum {string} */
                        type: "server" | "label_selector";
                        server?: {
                            /** Format: int64 */
                            id: number;
                        };
                        label_selector?: {
                            selector: string;
                        };
                    }[];
                };
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        /** FirewallResponse */
                        firewall?: {
                            /** Format: int64 */
                            id: number;
                            name: string;
                            labels?: {
                                [key: string]: string;
                            };
                            /** Format: date-time */
                            created: string;
                            rules: {
                                /** @enum {string} */
                                direction: "in" | "out";
                                source_ips: string[];
                                destination_ips: string[];
                                /** @enum {string} */
                                protocol: "tcp" | "udp" | "icmp" | "esp" | "gre";
                                port: string | null;
                            }[];
                            applied_to: {
                                /** @enum {string} */
                                type: "server" | "label_selector";
                                server?: {
                                    /** Format: int64 */
                                    id: number;
                                };
                                label_selector?: {
                                    selector: string;
                                };
                                applied_to_resources?: {
                                    /** @enum {string} */
                                    type?: "server";
                                    server?: {
                                        /** Format: int64 */
                                        id: number;
                                    };
                                }[];
                            }[];
                        };
                        actions?: {
                            /** Format: int64 */
                            id: number;
                            command: string;
                            /** @enum {string} */
                            status: "running" | "success" | "error";
                            /** Format: date-time */
                            started: string;
                            /** Format: date-time */
                            finished: string | null;
                            /** Format: int32 */
                            progress: number;
                            resources: {
                                /** Format: int64 */
                                id: number;
                                type: string;
                            }[];
                            error: {
                                code: string;
                                message: string;
                            } | null;
                        }[];
                    };
                };
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
    delete_firewall: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
    create_ssh_key: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": {
                    name: string;
                    public_key: string;
                    labels?: {
                        [key: string]: string;
                    };
                };
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        ssh_key: {
                            /** Format: int64 */
                            id: number;
                            name: string;
                            fingerprint: string;
                            public_key: string;
                            labels: {
                                [key: string]: string;
                            };
                            /** Format: date-time */
                            created: string;
                        };
                    };
                };
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
    delete_ssh_key: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            "4xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
            "5xx": {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        error: {
                            code: string;
                            message: string;
                            details?: Record<string, never> | null;
                        };
                    };
                };
            };
        };
    };
}
