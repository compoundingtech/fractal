import { Option, Schema } from 'effect'

const decodeJson = Schema.decodeUnknownOption(Schema.fromJsonString(Schema.Unknown))

/** Final fixture-source restriction for BOTH ask title and request.question. No normalization. */
export const isFixturePlainText = (text: string): boolean => text.length > 0 && text.length <= 60
  && /^[\x20-\x7e]+$/.test(text) && text.trim() === text && !text.includes('  ')
  && !/[*_#\[\]`>|]/.test(text) && Option.isNone(decodeJson(text))
