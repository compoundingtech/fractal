import type { Plugin } from 'vite'
import type { DevAdmissionOptions } from '../server/devAdmission.mts'

export function webfractalGateway(options?: Pick<DevAdmissionOptions, 'edgeHost' | 'edgeAliases'>): Plugin
