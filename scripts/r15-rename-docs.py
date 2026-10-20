#!/usr/bin/env python3
"""Rewrite source identities in documentation and source-path policy inputs.

Run after the pure directory-move commit. This deliberately leaves executable
names, runtime protocol/state strings and recorded JSON fixtures alone.
"""
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
CRATES = {
    'st3': 'fractal', 'stui': 'fractal-tui', 'st2': 'fractal-legacy-runner',
    **{'st3-' + name: 'fractal-' + name for name in (
        'terminal-direct', 'ui-model', 'telemetry', 'store', 'secrets', 'seats',
        'seats-base', 'schema', 'reconcile', 'peer', 'missions', 'migrate',
        'missions-base', 'kernel', 'github', 'github-base', 'feed', 'evidence',
        'e2e', 'delivery', 'delivery-base', 'daemon', 'conversation-ui', 'client',
        'client-codegen', 'attention', 'api-mailbox', 'api-core', 'api-client-v0',
        'client-tui',
    )},
    **{'st-' + name: 'fractal-' + name for name in (
        'storage-version', 'wire', 'runtime', 'resource-protocol', 'drivers', 'chatgpt',
        'build-stamp', 'fabric-adapter',
    )},
    'st2-resource-providers': 'fractal-resource-providers',
    'st2-resource-wasip2': 'fractal-resource-wasip2',
    'st2-resource-protocol': 'fractal-resource-protocol', 'st2-wire': 'fractal-wire',
    **{'st2-' + name + '-component': 'fractal-' + name + '-component' for name in (
        'github-issue', 'github-pr', 'pty-stats', 'vista',
    )},
}
PACKAGES = {
    '@smalltalk/st3-client': '@fractal/client',
    '@smalltalk/st3-scenarios': '@fractal/scenarios',
    '@smalltalk/st3-views': '@fractal/views',
    '@smalltalk/fractal-ui': '@fractal/ui', '@st3/sdk': '@fractal/sdk',
}
PROTECTED_EXECUTABLES = {
    'st2', 'st3', 'stui', 'st-chatgpt', 'st3-client-codegen',
    'st3-client-tui', 'st3-fixture', 'st3-migrate', 'st-fabric-adapter',
}


def rewrite(text):
    for old in PROTECTED_EXECUTABLES - {'st2', 'st3', 'stui'}:
        new = CRATES.get(old)
        if new is None:
            continue
        text = re.sub(r'(?P<flag>--bin[ =]+)' + re.escape(new) + r'(?![\w-])',
                      lambda match: match['flag'] + old, text)
        text = re.sub(r'(?P<flake>github:compoundingtech/smalltalk/[^\s`]*#)' +
                      re.escape(new) + r'(?![\w-])',
                      lambda match: match['flake'] + old, text)
    # Repair earlier output at reviewed runtime contexts, not Cargo selectors.
    text = re.sub(r'(?<![\w./-])fractal-fabric-adapter(?=\s+--)',
                  'st-fabric-adapter', text)
    text = text.replace('`fractal-fabric-adapter` is a separate process',
                        '`st-fabric-adapter` is a separate process')
    text = text.replace('~/.local/state/fractal-fabric-adapter',
                        '~/.local/state/st-fabric-adapter')
    # Historical Clippy ancestry examples name the old Git baseline, not an alias.
    historical = {}
    for old, new in CRATES.items():
        pattern = r'"' + re.escape(new) + r'"\s*:\s*"' + re.escape(old) + r'"'
        for match in re.finditer(pattern, text):
            fragment = match.group()
            token = f'R15_HISTORICAL_ANCESTRY_{len(historical)}'
            historical[token] = fragment
            text = text.replace(fragment, token)
    for old, new in sorted(CRATES.items(), key=lambda pair: -len(pair[0])):
        text = re.sub(r'crates/' + re.escape(old) + r'(?=[/`\s):*]|$)', 'crates/' + new, text)
    for stem in ('client', 'scenarios', 'views'):
        text = text.replace('clients/typescript/st3-' + stem, 'clients/typescript/fractal-' + stem)
    text = text.replace('clients/swift/St3Client', 'clients/swift/FractalClient')
    text = text.replace('apps/fractal-web/packages/st3-sdk', 'apps/fractal-web/packages/fractal-sdk')
    for old, new in PACKAGES.items():
        text = text.replace(old, new)
    text = re.sub(r'\bSt3Client\b', 'FractalClient', text)
    # Cargo selectors refer to packages; bare commands still refer to executables.
    for old, new in sorted(CRATES.items(), key=lambda pair: -len(pair[0])):
        text = re.sub(r'(?P<flag>(?:-p|--package|--exclude)[ =]+)' + re.escape(old) + r'(?![\w-])',
                      lambda match: match['flag'] + new, text)
        text = text.replace(old + '/test-support', new + '/test-support')
        if old not in PROTECTED_EXECUTABLES:
            # Do not turn socket/state names such as st3-client.sock into package names.
            text = re.sub(r'(?<![\w.-])' + re.escape(old) + r'(?![\w.-])', new, text)
        if old not in ('st3', 'st2', 'stui'):
            text = re.sub(r'\b' + re.escape(old.replace('-', '_')) + r'(?=::)', new.replace('-', '_'), text)
    text = re.sub(r'\bst3(?=::)', 'fractal_daemon', text)
    text = re.sub(r'\bst2(?=::)', 'fractal_legacy_runner', text)
    text = re.sub(r'\bstui(?=::)', 'fractal_tui', text)
    for token, fragment in historical.items():
        text = text.replace(token, fragment)
    return text


def main():
    names = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
    changed = 0
    for name in names:
        if not name or not (name.endswith('.md') or name in ('.st/risky-classes.json', '.st/crate-cut.json')):
            continue
        path = ROOT / name
        if not path.is_file():
            continue
        old = path.read_text()
        if 'DO NOT EDIT' in old[:512] or 'automatically generated' in old[:512].lower():
            continue
        new = rewrite(old)
        if new != old:
            path.write_text(new)
            changed += 1
    print(f'Documentation/source policy files rewritten: {changed}')


if __name__ == '__main__':
    main()
