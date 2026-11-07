import { ClientError } from '@fractal/client'
import { Option, Schema } from 'effect'

const rejected = Schema.Struct({
  status: Schema.Literal(403),
  response: Schema.Struct({ code: Schema.Literal('forbidden'), details: Schema.Struct({ reason: Schema.Literal('credential-rejected') }) }),
})

/** Only the structured credential discriminator distinguishes re-pairing from a missing grant. */
export const readAuthorization = (error: { readonly status: number; readonly response: unknown }): 'unauthorized' | 'ungranted' | undefined =>
  Option.isSome(Schema.decodeUnknownOption(rejected)(error)) ? 'unauthorized'
    : error.status === 403 || error.status === 401 ? 'ungranted' : undefined

export const clientReadAuthorization = (error: unknown) => error instanceof ClientError ? readAuthorization(error) : undefined
