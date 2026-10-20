#!/usr/bin/env python3
"""Reproduce R15 script and reviewed policy source/package rewrites after directory moves.

Run from any directory: python3 scripts/r15-rename-scripts.py
Historical source readers are deliberately left for the final hand-edit layer.
"""
from pathlib import Path
import re

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
    # Source paths, not executable names, state paths or serialized fixtures.
    if name not in HISTORICAL:
        source = re.sub(r'crates/(st3(?:-[a-z0-9-]+)?|stui|st-(?:drivers|runtime|chatgpt|storage-version|ui-model|terminal-direct)|st2-(?:resource-providers|resource-wasip2|[a-z0-9-]+-component))(?=[/"\' *])',
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
