# PR evidence URL normalization

`evidence_url::normalize_pr_url` provides one offline citation-key contract for
observed GitHub PR URLs. It returns a canonical HTTPS URL, lowercase repository
and positive u64 number. It makes no network request and does not establish work
ownership, PR creation/edit time, or a verified/current head.

Accepted aliases are HTTP/HTTPS, ASCII case variations of the registered host and
owner/repository, default ports, positive decimal leading zeros, `pull`/`pulls`,
one trailing slash, `files`/`commits`/`checks` subpages and fragments. Display
queries are bounded to `w=0|1` and `tab=conversation|files|commits|checks`, without
duplicate keys. Outer whitespace is trimmed; control characters are rejected.

Only the literal github.com authority is registered. Userinfo, nondefault/empty
ports, percent or Unicode host aliases, embedded whitespace, backslashes,
percent-encoded or dot-segment paths, arbitrary tails, issue/shorthand identities,
invalid numbers and unknown query parameters are rejected before a URL parser
can silently change the identity. Error messages contain no submitted URL or
credential.

The library does not modify existing watch parsing or evidence attachments.
Later association and lookup callers share this function; generic unsupported
attachments retain their original behavior. Originals remain in their existing
immutable claims. Association records cite claim ID and evidence ordinal instead
of duplicating raw URLs. A foreign actor can cite any PR URL, so a match is an
observation and an ambiguity must show its citing actors and attempts.

The pinned fixture contains the approved 42 normalization vectors. Tests also
cover hostile authority spellings and queries, maximum u64 numbers, distinct
repository identities and idempotent canonicalization. This slice adds no index,
backfill, lookup route, new claim kind or binding-contract field.
