# Ask a person for something

An invented catalog import can validate every input automatically, but its final output format
needs a person to choose between CSV and JSON.

## Failure first: a terminal question is not durable coordination

If the agent prints a blocking prompt in its terminal and waits, the responsible person has no
durable inbox item, the graph does not say what is blocked, and a restarted harness can lose the
question. A push notification would have the same ownership problem: it may attract attention, but
it is not the request or its lifecycle.

Do not stop unrelated work while waiting for an answer.

## Supported recovery: ask through person work

The agent names what it needs, what the answer unblocks, and what it did instead:

```sh
blocked_step=step-run/GENERATION_ID/publish
fractal work ask \
  --for person/operator \
  --title 'Choose the catalog export format' \
  --reason 'CSV or JSON is needed to unblock the publish step. Input validation is complete, and I am continuing the independent checksum report meanwhile.' \
  --step "$blocked_step" \
  --as agent/example/worker \
  --idempotency-key example-catalog-export-format
```

Replace `GENERATION_ID` with the exact value printed by `fractal work ls`; it is a placeholder, not a
literal graph ID.

The stable idempotency key makes retries return the same ask instead of creating duplicates.
The ask appears in `person/operator`'s alerts and blocks only the named publish step until answered.
Independent work can continue.

Inspect the waiting step and continue everything that does not depend on the choice:

```sh
fractal work ls --as agent/example/worker
```

When the person answers the ask, the agent reads the durable outcome and resumes only the
dependent publish work. The ask and its answer remain graph history.

When the mission author already knows a person must decide, declare a human gate in the mission
instead, as [`human-review.kdl`](human-review.kdl) does. The decision then appears in the same
attention inbox without any agent asking for it. An attention request is for a need that the plan
alerts without any agent asking for it. An ask is for a need that the plan did not foresee.
