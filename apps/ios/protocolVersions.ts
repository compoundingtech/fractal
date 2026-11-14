// Reader compatibility is permanent; request writers keep their own API_VERSION default.
export const isClientApiVersion = (value: unknown): boolean => value === 'st3.client.v0' || value === 'fractal.client.v0';
export const isClientErrorVersion = (value: unknown): boolean => value === 'st3.client.error.v0' || value === 'fractal.client.error.v0';
