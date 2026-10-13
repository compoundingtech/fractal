# Regenerates versions/*.ts from exp1-sdk/ship-change.ts. Each version makes one kind of change.
# Run from exp4-hmr: python3 make-versions.py
src = open('../exp1-sdk/ship-change.ts').read().replace('"../sdk/mission.ts"', '"../../sdk/mission.ts"')

def head(text):
    return f"// Experiment 4 version: {text}\n// Generated from exp1-sdk/ship-change.ts by exp4-hmr/make-versions.py; edit that script, not this file.\n\n"

def w(name, desc, text):
    assert text != src or name == 'v1', f'{name}: the edit did not apply'
    open(f'versions/{name}.ts', 'w').write(head(desc) + text)

w('v1', 'the program as it shipped.', src)
w('v2-text', 'the fix step asks for a smaller fix (definition change only).',
  src.replace('goal: "Fix the failing checks."', 'goal: "Fix the failing checks with the smallest change that passes."'))
insert = '''    yield* step("security-scan", { title: "Scan the change", goal: "Run the security scan.", assignedTo: reviewer })

    // A loop:'''
w('v2-insert', 'a security scan after review, added without a guard.', src.replace('    // A loop:', insert, 1))
insert_p = '''    // Added in v2. Runs that passed review under v1 replay without it.
    if (yield* patched("security-scan")) {
      yield* step("security-scan", { title: "Scan the change", goal: "Run the security scan.", assignedTo: reviewer })
    }

    // A loop:'''
w('v2-patched', 'the security scan behind patched(), so older runs still replay.',
  src.replace('    // A loop:', insert_p, 1).replace('import { ask, escalateAfter, gate, mission, scope, step, timerName,', 'import { ask, escalateAfter, gate, mission, patched, scope, step, timerName,'))
w('v2-remove', 'the docs step removed.',
  src.replace('''    const [build, docs] = yield* Effect.all(
      [''', '''    const [build] = yield* Effect.all(
      [''').replace('''        ),
        step("docs", { title: "Update the docs", goal: `Document ${input.change}.`, assignedTo: builder })
      ],''', '''        )
      ],''').replace('context: { build: build.evidence as any, docs: docs.summary }', 'context: { build: build.evidence as any }'))
w('v2-rounds', 'five CI rounds instead of three.', src.replace('round <= 3', 'round <= 5').replace('rounds: 3 })', 'rounds: 5 })'))
