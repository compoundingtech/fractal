---
title: "Scripted demo files"
description: "Inspect repeatable invented-data fixtures without starting agents or spending model tokens."
order: 12
---

# Scripted demo files

A scripted demo file describes invented agents, missions, conversations, documents, and a sequence of visible events. It is a repeatable fixture rather than work performed by model-backed agents.

## Validate a file

From a source checkout:

```sh
fractal demo check examples/demos/release-day.kdl
```

This reads the file and lists its beats without playing it, starting a daemon, or spending model tokens. Validation does not prove that a live workflow or client interaction succeeds.

## File structure

A file starts with `version 2`, then a `demo` block for its metadata, a `seed` block describing initial graph state, and `beats` describing events. Demo declarations cannot run arbitrary harnesses, commands, or gates.

Use the checked-in [demo examples](https://github.com/compoundingtech/fractal/blob/main/examples/demos) as complete authoring examples. Keep invented fixtures separate from working declarations and never reset a populated store to rehearse a scenario.

For ordinary graphical access to real agents and work, use the [web app or iOS client](web-and-ios.md). This guide does not provide a web playback command for demo files.
