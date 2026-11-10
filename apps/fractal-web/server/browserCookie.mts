import type { IncomingHttpHeaders } from 'node:http'

export const deviceCookie = 'fractal_device'
export const pairCookie = 'fractal_pair'
export const deviceLifetime = 30 * 24 * 60 * 60
export const cookieValue = (headers: IncomingHttpHeaders, name: string): string | undefined => {
  const values = (headers.cookie ?? '').split(';').map(value => value.trim()).filter(value => value.startsWith(`${name}=`))
  return values.length === 1 ? values[0]?.slice(name.length + 1) : undefined
}
export const browserBearer = (headers: IncomingHttpHeaders): string | undefined => {
  const value = cookieValue(headers, deviceCookie)
  return value !== undefined && /^[A-Za-z0-9_-]{43}$/.test(value) ? value : undefined
}
export const browserAuthorization = (headers: IncomingHttpHeaders, fallback: string): string => {
  const bearer = browserBearer(headers)
  return bearer === undefined ? fallback : `Bearer ${bearer}`
}
export const secureCookie = (name: string, value: string, maxAge: number): string =>
  `${name}=${value}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=${maxAge}`
