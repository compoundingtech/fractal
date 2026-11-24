import { readFileSync, statSync } from 'node:fs'

export interface SecretFiles {
  readonly readText: (path: string) => string
  /** The file's permission bits and owner. */
  readonly owner: (path: string) => { readonly mode: number; readonly uid: number }
}
const localFiles: SecretFiles = { readText: (path) => readFileSync(path, 'utf8'), owner: (path) => statSync(path) }

/** Read a private, user-owned bearer file without putting the token in the environment. */
export const readAuthorization = (
  env: NodeJS.ProcessEnv,
  name: string,
  files: SecretFiles = localFiles,
  uid: number | undefined = process.getuid?.(),
): string => {
  const path = env[name]?.trim()
  if (!path) throw new TypeError(`${name} is required`)
  const { mode, uid: fileOwner } = files.owner(path)
  if ((mode & 0o077) !== 0 || (uid !== undefined && fileOwner !== uid))
    throw new TypeError(`${name} must be owned by this user and readable by no one else (chmod 600)`)
  const token = files.readText(path).trim()
  if (token === '' || /\s/.test(token.replace(/^Bearer /, ''))) throw new TypeError(`${name} must hold one bearer token`)
  return token.startsWith('Bearer ') ? token : `Bearer ${token}`
}
