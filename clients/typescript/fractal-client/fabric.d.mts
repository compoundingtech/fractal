import type { ClientOptions, CollectionSocketFactory, FractalClient } from './Client.generated.ts';

export class FabricTransportError extends Error {
    readonly code: string;
    constructor(code: string, message: string);
}
export type FabricDialOptions = { program?: string; signal?: AbortSignal };
export class FabricEndpoint {
    constructor(value: string);
    static parse(value: string): FabricEndpoint;
    static validatePath(path: string): void;
    toString(): string;
    dial(options?: FabricDialOptions): Promise<string>;
}
export type FabricTransport = { fetchImpl: typeof fetch; socket: CollectionSocketFactory };
export type FabricClientOptions = Omit<ClientOptions, 'baseUrl' | 'fetchImpl' | 'socket'> & {
    endpoint: string;
    /** Fabric CLI executable. Defaults to `fabric` on PATH. */
    program?: string;
};
export function createFabricTransport(endpoint: string, options?: { program?: string }): Promise<FabricTransport>;
export function createFabricClient(options: FabricClientOptions): Promise<FractalClient>;
