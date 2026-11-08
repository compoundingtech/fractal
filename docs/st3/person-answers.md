# Person answers

Every question a person answers has one answer schema: the answers they chose, each with an
optional note, and their own words beside them. Notes and words are independent fields. Each is
recorded and delivered, or refused with a typed reason; a client never folds one into the other
and nothing typed is silently dropped.

The schema replaced `{id, text}` in one clean cut. The user approved it as HIGH risk:
"Clean cut, I approve HIGH (Recommended)".

## Input

```
Answer = { choices: [{ id, note? }], words? }
```

- `choices` names chosen answers by id, unique, in the request's order. A structured request
  takes at most one (`too_many`).
- `note` is the person's note on that one choice: trimmed, absent when blank, at most 600
  characters (`too_long`).
- `words` is the person's own text for the question: trimmed, absent when blank, at most 8,000
  characters (`too_long`).

`work.done` takes it as `answer` (`PersonAnswerInput`); the CLI as
`fractal work done STEP --choice ID[=NOTE] --words TEXT`, splitting `ID=NOTE` on the first `=`.
The old `{id, text}` is refused as an unknown field.

## What a request takes

A structured request's optional `words` says whether the person's words go with an answer:
`none`, `optional` or `required`. Absent means `required` for feedback and `optional` for a
decision or choice. An update takes none.

| Request | Answer |
| --- | --- |
| decision | one choice; its outcome is the option's. Requesting changes needs words (`words_required`) |
| choice | one choice (`selected`), or with `custom`, words and no choice (`custom`) |
| feedback | words and no choice (`feedback`) |
| update | nothing, or the choice `read` |
| any but an update | the reserved choice `dismiss` with optional words (`dismissed`), unless the request names its own `dismiss` |

Words a request does not take are refused (`words_not_allowed`), never dropped. A note on
`dismiss` is refused (`note_not_allowed`): the words say why. The server computes `outcome`; it is
never input.

## Record

The person-step completion records:

```
{ type, outcome, choices: [{ id, label, note? }], words? }
```

`label` comes from the request, so a reader needs no join. The summary line, which every client
and older member shows, carries everything typed: `Label: note — words`.

## Stored records

Claims are immutable and replicated, so stored answers are never rewritten.
`person_request::read_answer` is the one reader: a record from before the schema
(`{id, label, text}`) reads as `choices: [{id, label}]` with `text` as `words`, since legacy text
was always the person's words. Every API read, wake message and history line goes through it, so
clients see only the one schema.

## Mixed fleets

A build that reads the schema advertises `person_answer_schema=1` in its `daemon.started`.
Until every other current member advertises it (the strict barrier of
[`marks_switch`](../../crates/fractal-store/src/store/marks_switch.rs), recorded in `meta` as
`answer_schema_switched`), a recorded answer also carries the fields an older build reads: the
chosen `id` and `label`, and the note and words as `text`. Agents on an older member keep
dispatching on `id`. The reader drops these fields.

## Harness questions

Native harness questions use the same schema per question; see
[harness questions](harness-questions.md). Their mapping to each harness lands with the native
answer slices.
