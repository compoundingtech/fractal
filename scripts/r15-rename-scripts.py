#!/usr/bin/env python3
"""Reproduce R15 script and reviewed policy source/package rewrites after directory moves.

Run from any directory: python3 scripts/r15-rename-scripts.py
Historical source readers are deliberately left for the final hand-edit layer.
"""
from pathlib import Path
import re
import ast
import importlib.util

ROOT = Path(__file__).resolve().parents[1]
SOURCE_TOOLS = {
    'module_layers.py', 'check-module-layers-test', 'crate_cut.py',
    'crate-cut-test', 'st_boundary.py', 'check-st-boundary-test',
}
HISTORICAL = {'release_notes.py', 'profile-mailbox-wakes'}
# These policies declare source inputs and probes for the script-backed lanes.
# Keep their paths aligned with the reviewed list in evidence-verify-test.
POLICY_PATHS = (
    '.st/evidence-skip-policy.json',
    '.st/required-lanes.json',
)


def package(name):
    if name == 'st3':
        return 'fractal'
    if name == 'st2':
        return 'fractal-legacy-runner'
    if name == 'stui':
        return 'fractal-tui'
    return re.sub(r'^st(?:3|2)?-', 'fractal-', name)


FIXTURE_DIRECTORIES = {
    path.name for path in (ROOT / "scripts").iterdir()
    if path.is_dir() and path.name.startswith("st3-")
}


def moved_directory_names():
    spec = importlib.util.spec_from_file_location('r15_directory_names', ROOT / 'scripts/r15-rename-docs.py')
    docs = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(docs)
    return {**docs.CRATES, 'st3-client': 'fractal-client',
            'st3-scenarios': 'fractal-scenarios', 'st3-views': 'fractal-views',
            'st3-sdk': 'fractal-sdk', 'St3Client': 'FractalClient'}


def path_directory_literals(source, directories):
    """Find basename literals flowing into Python path expressions.

    Names are followed backwards through assignments and loop iterators within
    one lexical scope. Package labels used only in commands or data stay intact.
    """
    try:
        module = ast.parse(source)
    except SyntaxError:
        # Shell scripts also own Python fixture/path code in heredocs.
        results = []
        for match in re.finditer(
                r'''(?m)^.*\bpython3?\b[^\n]*<<-?\s*['"]?(?P<marker>[A-Za-z_]\w*)['"]?[^\n]*\n''',
                source):
            end = re.search(r'(?m)^' + re.escape(match['marker']) + r'\s*$', source[match.end():])
            if end is None:
                continue
            start = match.end()
            for begin, finish, line, old, new in path_directory_literals(
                    source[start:start + end.start()], directories):
                results.append((start + begin, start + finish,
                                source.count('\n', 0, start) + line, old, new))
        return results
    scopes = [module] + [node for node in ast.walk(module)
                         if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda))]
    found = {}
    for scope in scopes:
        nodes, pending = [], [scope]
        while pending:
            node = pending.pop()
            nodes.append(node)
            pending.extend(child for child in ast.iter_child_nodes(node)
                           if not isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef,
                                                     ast.Lambda, ast.ClassDef)))
        paths = [node for node in nodes if (
            isinstance(node, ast.BinOp) and isinstance(node.op, ast.Div)
            or isinstance(node, ast.JoinedStr) and any(
                isinstance(part, ast.Constant) and isinstance(part.value, str) and '/' in part.value
                for part in node.values)
            or isinstance(node, ast.Call) and (
                isinstance(node.func, ast.Name) and node.func.id == 'Path'
                or isinstance(node.func, ast.Attribute) and (
                    node.func.attr == 'joinpath'
                    or node.func.attr == 'join' and isinstance(node.func.value, ast.Attribute)
                    and node.func.value.attr == 'path')))]
        # A same-named local/test folder is not the moved workspace directory.
        # Anchor the flow at real moved parent paths before following variables.
        def moved_parent(node):
            strings = [child.value for child in ast.walk(node)
                       if isinstance(child, ast.Constant) and isinstance(child.value, str)]
            joined = '/'.join(strings)
            return any(parent in joined for parent in (
                'crates', 'clients/typescript', 'clients/swift', 'apps/fractal-web/packages'))
        anchored = {node for node in paths if moved_parent(node)}
        path_names = set()
        while True:
            previous = len(anchored), len(path_names)
            for node in nodes:
                if isinstance(node, (ast.Assign, ast.AnnAssign)) and node.value in anchored:
                    targets = node.targets if isinstance(node, ast.Assign) else [node.target]
                    path_names.update(child.id for target in targets for child in ast.walk(target)
                                      if isinstance(child, ast.Name))
            anchored.update(node for node in paths if any(
                child.id in path_names for child in ast.walk(node) if isinstance(child, ast.Name)))
            if previous == (len(anchored), len(path_names)):
                break
        paths = list(anchored)
        names = {node.id for path in paths for node in ast.walk(path) if isinstance(node, ast.Name)}
        values = list(paths)
        while True:
            previous = len(names)
            for node in nodes:
                if isinstance(node, (ast.Assign, ast.AnnAssign)):
                    targets = node.targets if isinstance(node, ast.Assign) else [node.target]
                    value = node.value
                elif isinstance(node, (ast.For, ast.AsyncFor)):
                    targets, value = [node.target], node.iter
                else:
                    continue
                if value is not None and any(
                        child.id in names for target in targets for child in ast.walk(target)
                        if isinstance(child, ast.Name)):
                    values.append(value)
                    names.update(child.id for child in ast.walk(value) if isinstance(child, ast.Name))
            if len(names) == previous:
                break
        for value in values:
            for node in ast.walk(value):
                if isinstance(node, ast.Constant) and isinstance(node.value, str) and node.value in directories:
                    found[node.lineno, node.col_offset] = node
    lines = source.splitlines(keepends=True)
    offsets, offset = [], 0
    for line in lines:
        offsets.append(offset)
        offset += len(line)
    result = []
    for node in found.values():
        start = offsets[node.lineno - 1] + len(lines[node.lineno - 1].encode()[:node.col_offset].decode())
        end = offsets[node.end_lineno - 1] + len(lines[node.end_lineno - 1].encode()[:node.end_col_offset].decode())
        result.append((start, end, node.lineno, node.value, directories[node.value]))
    return sorted(result)


def rewrite_path_directory_literals(source):
    for start, end, _, old, new in reversed(path_directory_literals(source, moved_directory_names())):
        literal = source[start:end]
        quote = '"' if literal.startswith('"') else "'"
        source = source[:start] + quote + new + quote + source[end:]
    return source


def rewrite(relative, source):
    name = relative.name
    if name == 'release_notes_test.py':
        # Historical-source regressions intentionally contain old crate names.
        # Repair earlier script output, then leave this hand-owned test intact.
        source = source.replace(
            "for legacy in (\n            'crates/fractal-store/",
            "for legacy in (\n            'crates/st3-store/")
        source = source.replace(
            "            'crates/fractal-daemon/src/store/checkpoint_rules.rs',",
            "            'crates/st3-daemon/src/store/checkpoint_rules.rs',")
        marker = '    def test_storage_crate_rename_preserves_historical_version_discovery(self):'
        if marker in source:
            start = source.index(marker)
            end = source.find('\n    def ', start + len(marker))
            end = len(source) if end < 0 else end
            fixture = source[start:end]
            fixture = fixture.replace("self.write('crates/fractal-storage-version/",
                                      "self.write('crates/st-storage-version/", 1)
            fixture = fixture.replace("self.git('mv', 'crates/fractal-storage-version/",
                                      "self.git('mv', 'crates/st-storage-version/", 1)
            source = source[:start] + fixture + source[end:]
        return source
    # This extracted library has no executable or persisted runtime identity.
    # Its source fixtures, Cargo selectors and Rust imports move together.
    source = re.sub(r'\bst-build-stamp\b', 'fractal-build-stamp', source)
    source = re.sub(r'\bst_build_stamp\b', 'fractal_build_stamp', source)
    # A literal "\n" ends in a word character, so \b cannot see the first
    # dependency key in a single-line Python-embedded Cargo manifest.
    source = source.replace(r'\nst-build-stamp', r'\nfractal-build-stamp')
    if name == 'profile-mailbox-wakes':
        # This reader measures arbitrary Git refs, including pre-rename refs.
        source = source.replace(
            'for name in ("fractal-api-mailbox", "fractal-daemon", "fractal",\n'
            '                 "fractal-api-mailbox", "fractal-daemon", "fractal"):',
            'for name in ("fractal-api-mailbox", "fractal-daemon", "fractal",\n'
            '                 "st3-api-mailbox", "st3-daemon", "st3"):')
    if name not in HISTORICAL:
        source = rewrite_path_directory_literals(source)
    # Source paths, not executable names, state paths or serialized fixtures.
    if name not in HISTORICAL:
        source = re.sub(r'crates/(st3(?:-[a-z0-9-]+)?|stui|st-(?:drivers|runtime|chatgpt|storage-version|ui-model|terminal-direct|fabric-adapter)|st2-(?:resource-providers|resource-wasip2|[a-z0-9-]+-component))(?=[/"\' *])',
                        lambda match: 'crates/' + package(match[1]), source)
    for old, new in (
        ('clients/typescript/st3-client', 'clients/typescript/fractal-client'),
        ('clients/typescript/st3-scenarios', 'clients/typescript/fractal-scenarios'),
        ('clients/typescript/st3-views', 'clients/typescript/fractal-views'),
        ('clients/swift/St3Client', 'clients/swift/FractalClient'),
        ('apps/fractal-web/packages/st3-sdk', 'apps/fractal-web/packages/fractal-sdk'),
        ('@smalltalk/st3-client', '@fractal/client'),
        ('@smalltalk/st3-scenarios', '@fractal/scenarios'),
        ('@smalltalk/st3-views', '@fractal/views'),
        ('@smalltalk/fractal-ui', '@fractal/ui'),
        ('@st3/sdk', '@fractal/sdk'),
    ):
        source = source.replace(old, new)
    # Cargo selectors, never --bin operands or target/debug executable paths.
    source = re.sub(r'((?:-p|--package|--exclude)[ =]+)(st3(?:-[a-z0-9-]+)?|st2|stui)(?=\s|\\n|["\'])',
                    lambda match: match[1] + package(match[2]), source)
    source = re.sub(r'(["\'](?:-p|--package|--exclude)["\'],\s*["\'])(st3(?:-[a-z0-9-]+)?|st2|stui)(["\'])',
                    lambda match: match[1] + package(match[2]) + match[3], source)
    source = re.sub(r'(package\(=?)(st3(?:-[a-z0-9-]+)?|st2|stui)(\))',
                    lambda match: match[1] + package(match[2]) + match[3], source)
    if name in SOURCE_TOOLS:
        # These files contain source parser rules and synthetic source/manifest
        # fixtures, rather than persisted protocol data or executable commands.
        source = re.sub(r'\bst3-([a-z0-9-]+)', r'fractal-\1', source)
        source = re.sub(r'\bst3_([a-z0-9_]+)', r'fractal_\1', source)
        source = re.sub(r'\bst3::', 'fractal_daemon::', source)
        source = re.sub(r'\bstui\b', 'fractal-tui', source)
        source = source.replace(r'\nstui', r'\nfractal-tui')
        source = source.replace(r'\nst3-', r'\nfractal-')
        source = source.replace('"crates", "st3"', '"crates", "fractal"')
        source = source.replace('"st3", "fractal-tui"', '"fractal", "fractal-tui"')
        source = source.replace('("st3",)', '("fractal",)')
        source = source.replace('name = "st3"', 'name = "fractal"')
        source = source.replace('package st3', 'package fractal')
        source = source.replace('"st-drivers"', '"fractal-drivers"')
        source = source.replace('../st-drivers', '../fractal-drivers')
        source = source.replace('st-drivers={', 'fractal-drivers={')
        source = source.replace(' -> st-drivers', ' -> fractal-drivers')
        # Script fixture identities are not crate/package names. Their directories
        # do not move in R15; repair earlier broad substitutions as well.
        for fixture in FIXTURE_DIRECTORIES:
            source = source.replace(package(fixture), fixture)
    if name == 'st_boundary.py':
        source = source.replace('(?:r#)?st2', '(?:r#)?fractal_legacy_runner')
        source = source.replace('target == "st2"', 'target == "fractal-legacy-runner"')
        source = source.replace('"st2 import"', '"fractal_legacy_runner import"')
        source = source.replace('}: st2 import', '}: fractal_legacy_runner import')
        source = source.replace('"st2 dependency: "', '"fractal-legacy-runner dependency: "')
    if name == 'check-st-boundary-test':
        # Only the source/dependency fixture portion: runtime state and program
        # fixtures below it must keep their historical st2 tokens.
        begin = source.index('class BoundaryCheck')
        end = source.index('    def test_every_st2_environment_name_fails', begin)
        fixture = source[begin:end]
        fixture = fixture.replace('"st2")', '"fractal-legacy-runner")')
        fixture = fixture.replace('use st2::', 'use fractal_legacy_runner::')
        fixture = fixture.replace('use r#st2 ', 'use r#fractal_legacy_runner ')
        fixture = fixture.replace('extern crate st2 ', 'extern crate fractal_legacy_runner ')
        fixture = fixture.replace('st2 import', 'fractal_legacy_runner import')
        fixture = fixture.replace('package="st2"', 'package="fractal-legacy-runner"')
        fixture = fixture.replace('../st2', '../fractal-legacy-runner')
        fixture = fixture.replace(' -> st2', ' -> fractal-legacy-runner')
        fixture = fixture.replace("product == 'st3'", "product == 'fractal'")
        source = source[:begin] + fixture + source[end:]
    if name == 'ci-nextest-targets':
        source = source.replace("package['name'] == 'st2'", "package['name'] == 'fractal-legacy-runner'")
    if name == 'ci-test-archive-test':
        # All st2 tokens here identify synthetic Cargo packages, not executables.
        source = source.replace("'st2'", "'fractal-legacy-runner'")
        source = source.replace("name='st-drivers'", "name='fractal-drivers'")
    if name == 'ci-test-partitions-test':
        source = source.replace('"st2"', '"fractal-legacy-runner"')
    return source


def main():
    paths = sorted((ROOT / 'scripts').rglob('*'))
    paths.extend(ROOT / relative for relative in POLICY_PATHS)
    for path in paths:
        if not path.is_file() or '__pycache__' in path.parts:
            continue
        if path.name.startswith('r15-rename-'):
            continue
        try:
            source = path.read_text()
        except UnicodeError:
            continue
        updated = rewrite(path.relative_to(ROOT), source)
        if updated != source:
            path.write_text(updated)
            print(path.relative_to(ROOT))


if __name__ == '__main__':
    main()
