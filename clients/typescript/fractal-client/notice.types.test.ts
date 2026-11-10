import type { Brand, DateTime, Option } from 'effect';
import type { NoticeId, NoticeSnapshot, NoticeTimestamp, NoticeTimestampEncoded, Timestamp } from './Schema.generated.ts';

type Assert<T extends true> = T;
type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends (<T>() => T extends B ? 1 : 2) ? true : false;
type RichParity = [
    Assert<Equal<NoticeTimestampEncoded, string>>,
    Assert<Equal<NoticeTimestamp['wire'], string>>,
    Assert<Equal<NoticeTimestamp['instant'], Option.Option<DateTime.Utc>>>,
    Assert<Equal<NoticeSnapshot['seen_at'], Option.Option<NoticeTimestamp>>>,
    Assert<Equal<NoticeId, string & Brand.Brand<'st3/NoticeId'>>>,
    Assert<Equal<Timestamp, DateTime.Utc>>,
];
declare const richParity: RichParity;
void richParity;
