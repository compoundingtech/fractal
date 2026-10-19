#!/usr/bin/env python3
"""Cut a domain out of a crate at function level.

A crate's modules are not a layering: store.rs calls into the domain files and the domain files call
back, so at file level nearly everything is one strongly connected component. At item level it is not.
The functions, types and constants the fold, the schema and the open path reach (the knot, and what it
depends on) must stay in the lower crate; everything else only calls down and can move up. This tool
computes that partition and applies it:

    crate_cut.py plan  --src crates/st3-store/src --config .st/crate-cut.json [--pins FILE] [--json OUT]
    crate_cut.py apply --src crates/st3-store/src --config .st/crate-cut.json --layer evidence \\
                       --crate st3-evidence --lower st3-store [--pins FILE]

Items are the unit: free fns, types, consts, statics, traits, macros and inline modules; the fns of
an `impl Store` block (each moves on its own and becomes a method of an extension trait implemented for
Store); an inherent impl of any other type, or a trait impl, is bound to its type and moves with it.
References are found by name with a little qualification (`.m(`, `T::m`, `Self::m`) and err towards
more references: an item wrongly kept in the lower crate is harmless, one wrongly moved fails to
compile and is then pinned (see --pins).
"""
import argparse
import collections
import json
import os
import re
import sys
import subprocess
from pathlib import Path
import threading

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import module_layers as ml  # noqa: E402
import rust_items as ri  # noqa: E402

TEST_FILE = re.compile(r"(_tests?\.rs$|/tests?\.rs$|/tests/|_tests/|^test_support\.rs$|/test_support/|^test_support/)")
FREE_KINDS = ("fn", "struct", "enum", "union", "const", "static", "type", "trait", "macro_rules", "mod")
IDENT = re.compile(r"[A-Za-z_][A-Za-z_0-9]*")
# Method names the standard library and common crates define too: a `.name(` call may be theirs.
STD_METHODS = frozenset("""as_str as_ref as_mut as_bytes as_slice as_deref to_string to_owned to_vec into from clone default source
metadata description kind name id get set new len is_empty iter next push pop insert remove contains entry take replace swap
map filter fold find any all collect join split trim parse read write flush lock unlock send recv run build finish close open
add sub mul div neg not cmp partial_cmp eq ne hash fmt drop deref borrow cloned copied unwrap expect ok err and_then or_else
unwrap_or unwrap_or_else unwrap_or_default is_some is_none is_ok is_err bytes chars lines keys values extend append clear drain
first last min max sum count position rev zip skip step_by chain enumerate peekable sort sort_by dedup retain truncate resize
starts_with ends_with strip_prefix strip_suffix to_lowercase to_uppercase get_mut get_or_insert_with and_modify or_insert
or_insert_with or_default borrow_mut try_lock elapsed duration_since as_secs as_millis context with_context to_vec_pretty""".split())


class Node:
    __slots__ = ("id", "file", "item", "owner", "kind", "name", "refs", "label", "eff", "core", "pinned", "size", "merge", "fixed", "names", "test", "container", "copy_of")

    def __init__(self, nid, file, item, owner, kind, name):
        self.id = nid
        self.file = file
        self.item = item
        self.owner = owner  # for a method of `impl Store`: the impl Item
        self.kind = kind  # free kinds, "method", "impl"
        self.name = name
        self.refs = set()
        self.label = None
        self.eff = None
        self.core = False
        self.pinned = False
        self.size = len(item.text)
        self.merge = None  # name of the type an impl is bound to
        self.fixed = None  # layer index of a node that already lives in a higher crate
        self.names = ()  # the statics a macro invocation (thread_local!) defines
        self.test = False  # test-only code: it moves to the highest crate among the items it refers to
        self.container = ()  # inline modules (names) the item sits in
        self.copy_of = None

    def key(self):
        o = "Store::" if self.kind == "method" else ""
        c = "::".join(self.container) + "::" if self.container else ""
        t = "test:" if self.test else ""
        return f"{t}{self.file}::{c}{o}{self.name}"

    def __repr__(self):
        return f"<{self.kind} {self.key()}>"


def code_only(text):
    """Text with comments removed and string/char literal contents blanked (newlines kept)."""
    out = []
    i = 0
    n = len(text)
    start = 0
    while i < n:
        k = ml._skip_literal(text, i)
        if k != i:
            out.append(text[start:i])
            seg = text[i:k]
            out.append(re.sub(r"[^\n]", " ", seg) if seg[:1] in "\"'rb" else seg)
            i = start = k
            continue
        if text.startswith("//", i):
            out.append(text[start:i])
            j = text.find("\n", i)
            i = start = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            out.append(text[start:i])
            i = ri._skip_block_comment(text, i)
            start = i
            continue
        i += 1
    out.append(text[start:])
    return "".join(out)


def module_files(src):
    out = []
    for dp, _, fs in os.walk(src):
        for f in sorted(fs):
            if f.endswith(".rs"):
                out.append(os.path.relpath(os.path.join(dp, f), src))
    return sorted(out)


def test_file_set(parsed):
    """Files that only exist for tests: `#[cfg(test)] mod x;` targets, files named like tests, and what they include."""
    test_files = set()
    for rel, (text, items) in parsed.items():
        stem = rel[:-3]
        for it in items:
            if it.kind == "mod" and it.cfg_test and it.text.rstrip().endswith(";"):
                test_files.add(f"{stem}/{it.name}.rs")
    for rel, (text, items) in parsed.items():
        if TEST_FILE.search(rel) or rel in test_files:
            for m in re.finditer(r'include!\("([^"]+)"\)', text):
                test_files.add(os.path.normpath(os.path.join(os.path.dirname(rel), m.group(1))))
    changed = True
    while changed:
        changed = False
        for rel, (text, items) in parsed.items():
            if rel in test_files or TEST_FILE.search(rel):
                for it in items:
                    if it.kind == "mod" and it.text.rstrip().endswith(";") and f"{rel[:-3]}/{it.name}.rs" not in test_files:
                        test_files.add(f"{rel[:-3]}/{it.name}.rs")
                        changed = True
    return test_files


CONTAINERS = {}  # (file, container) -> (attrs text of the inline mod, use items inside it)
FILE_USES = {}  # file -> the text of its top-level `use` items


def add_test_nodes(rel, text, items, whole, container, nodes, store_type, record=True):
    for it in items:
        if it.kind == "use":
            continue
        in_test = whole or it.cfg_test
        if it.kind == "mod":
            if in_test and it.body is not None and len(container) > 0:
                # deeper modules are one unit
                n = Node(len(nodes), rel, it, None, "mod", it.name)
                n.test = True
                n.container = container
                nodes.append(n)
                continue
            if in_test and it.body is not None:
                sub = ri.parse_items(text, it.body[0], it.body[1])
                if record:
                    CONTAINERS[(rel, container + (it.name,))] = ("\n".join(it.attrs) + ("\n" if it.attrs else "") + ("pub " if it.is_pub else ""), [c.text for c in sub if c.kind == "use"])
                add_test_nodes(rel, text, sub, True, container + (it.name,), nodes, store_type, record)
            continue
        if not in_test:
            continue
        if it.kind in FREE_KINDS:
            n = Node(len(nodes), rel, it, None, it.kind, it.name)
        elif it.kind == "macro_call":
            n = Node(len(nodes), rel, it, None, "macro_call", None)
            n.names = re.findall(r"\bstatic\s+(?:ref\s+)?([A-Za-z_]\w*)", it.text)
            n.name = n.names[0] if n.names else None
        elif it.kind == "impl":
            n = Node(len(nodes), rel, it, None, "impl", it.name)
            n.merge = it.impl_type
        else:
            continue
        n.test = True
        n.container = container
        nodes.append(n)


def load(src, store_type="Store", nodes=None, fixed=None):
    """Parse the crate; return (nodes, files, tfiles). files maps path -> (text, items) for the production
    files, tfiles the same for test-only files. Test items are nodes too (`test`); they are placed after the
    production code, wherever the highest crate they refer to is.
    With `fixed` (a layer index) the nodes are those of a crate that was already cut: they only refer to
    the lower crates, which fixes how low what they refer to must stay."""
    files = {}
    tfiles = {}
    nodes = [] if nodes is None else nodes
    first = len(nodes)
    parsed = {}
    for rel in module_files(src):
        text = open(os.path.join(src, rel)).read()
        parsed[rel] = (text, ri.parse(text))
    test_files = test_file_set(parsed)
    for rel, (text, items) in parsed.items():
        for it in items:
            if it.kind == "mod" and it.text.rstrip().endswith(";"):
                for at in it.attrs:
                    pm = re.fullmatch(r'#\[path\s*=\s*"([^"]+)"\s*\]', at.strip())
                    if pm:
                        target = os.path.normpath(os.path.join(os.path.dirname(rel), pm.group(1)))
                        if fixed is None:
                            MODULE_PATH_OVERRIDE[target] = module_path(rel) + [it.name]
    for rel, (text, items) in parsed.items():
        for it in items:
            if it.kind == "mod" and any(re.fullmatch(r"#\[cfg\(test\)\]", at.strip()) for at in it.attrs):
                if fixed is None:
                    TEST_ONLY_MODULES.add(tuple(module_path(rel)) + (it.name,))
    for rel in module_files(src):
        if fixed is None:
            FILE_USES[rel] = "\n".join(it.text for it in parsed[rel][1] if it.kind == "use")
    for rel in module_files(src):
        text, items = parsed[rel]
        if TEST_FILE.search(rel) or rel in test_files:
            tfiles[rel] = (text, items)
            add_test_nodes(rel, text, items, True, (), nodes, store_type, fixed is None)
            continue
        files[rel] = (text, items)
        for it in items:
            if it.cfg_test:
                add_test_nodes(rel, text, [it], False, (), nodes, store_type, fixed is None)
                continue
            if it.kind in FREE_KINDS:
                nodes.append(Node(len(nodes), rel, it, None, it.kind, it.name))
            elif it.kind == "macro_call":
                n = Node(len(nodes), rel, it, None, "macro_call", None)
                n.names = re.findall(r"\bstatic\s+(?:ref\s+)?([A-Za-z_]\w*)", it.text)
                n.name = n.names[0] if n.names else None
                n.pinned = True
                nodes.append(n)
            elif it.kind == "impl":
                if it.impl_type == store_type and it.impl_trait is None:
                    for c in it.children:
                        if c.cfg_test:
                            if c.kind == "fn":
                                n = Node(len(nodes), rel, c, it, "method", c.name)
                                n.test = True
                                nodes.append(n)
                            continue
                        if c.kind == "fn":
                            nodes.append(Node(len(nodes), rel, c, it, "method", c.name))
                        elif c.kind in ("const", "type"):
                            n = Node(len(nodes), rel, c, it, "assoc", c.name)
                            n.pinned = True
                            nodes.append(n)
                else:
                    n = Node(len(nodes), rel, it, None, "impl", it.name)
                    n.merge = it.impl_type
                    nodes.append(n)
    if fixed is not None:
        for n in nodes[first:]:
            n.fixed = fixed  # a crate already cut is a referrer like any other, tests included: what its tests name stays below
    return nodes, files, tfiles


def _sees(n, t):
    """Whether code in `n` can name the item `t` at all: a private item is visible in its own module and below it."""
    if t.kind not in ("fn", "const", "static", "struct", "enum", "union", "type", "trait") or t.item.is_pub or t.test or n.test:
        return True
    if n.fixed is not None:
        return False  # another crate cannot name a private item of this one
    mp, tp = module_path(n.file), module_path(t.file)
    return mp[: len(tp)] == tp


def _bound_locally(body, name):
    """Whether `name` is a local binding (a `let`, a parameter, a closure parameter or a pattern) in `body`, so
    that a bare use of it is not a fn passed as a value."""
    n = re.escape(name)
    return bool(
        re.search(r"\b(?:let|mut|for|ref|Some|Ok|Err)\s*\(?\s*(?:mut\s+)?" + n + r"\b", body)
        or re.search(r"[(,]\s*(?:mut\s+)?" + n + r"\s*:\s*[&\w\[(]", body)
        or re.search(r"\|[^|\n]*\b" + n + r"\b[^|\n]*\|", body)
        or re.search(r"\b" + n + r"\s*(?:=>|@)", body)
    )


def build_refs(nodes, store_type="Store"):
    free = collections.defaultdict(list)
    methods = collections.defaultdict(list)
    type_methods = collections.defaultdict(list)  # (Type, name) -> impl nodes of that type
    impls_of = collections.defaultdict(list)
    tfree = collections.defaultdict(list)
    tmethods = collections.defaultdict(list)
    for n in nodes:
        if n.fixed is not None:
            continue
        if n.test:
            if n.kind in FREE_KINDS:
                tfree[n.name].append(n)
            elif n.kind == "macro_call":
                for nm in n.names:
                    tfree[nm].append(n)
            elif n.kind == "impl":
                for c in n.item.children:
                    if c.name:
                        tmethods[c.name].append(n)
            elif n.kind == "method":
                tmethods[n.name].append(n)
            continue
        if n.kind in FREE_KINDS:
            free[n.name].append(n)
        elif n.kind == "macro_call":
            for nm in n.names:
                free[nm].append(n)
        elif n.kind == "method":
            methods[n.name].append(n)
        elif n.kind == "impl":
            impls_of[n.merge].append(n)
            for c in n.item.children:
                if c.name:
                    type_methods[(n.merge, c.name)].append(n)
                    methods[c.name].append(n)
    # a file's items need the `mod x;` that declares the file wherever they stay
    decls = {(n.file, n.name): n for n in nodes if n.fixed is None and not n.test and n.kind == "mod" and n.item.text.rstrip().endswith(";")}
    for n in nodes:
        if n.fixed is not None or n.test or n.kind == "mod" and n.item.body is not None:
            continue
        parts = module_path(n.file)
        parents = ["/".join(parts[:-1]) + ".rs", "/".join(parts[:-1]) + "/mod.rs"] if len(parts) > 1 else ["lib.rs"]
        for par in parents:
            d = decls.get((par, parts[-1]))
            if d is not None and d is not n:
                n.refs.add(d.id)
    # `#[cfg(test)] fn x` beside `#[cfg(not(test))] fn x`: the build uses one of them, so they stay together
    for name, tn_list in tfree.items():
        for tn in tn_list:
            if tn.container or tn.kind not in ("fn", "const", "static", "struct", "enum", "type", "trait"):
                continue
            for pn in free.get(name, ()):
                if pn.file == tn.file and pn.kind == tn.kind:
                    tn.refs.add(pn.id)
                    pn.refs.add(tn.id)
    local_defs = {(n.file, n.name) for n in nodes if n.fixed is not None and n.name}
    crate_modules = {"crate", "super", "self", "r#mod"}
    for n in nodes:
        for part in n.file[:-3].split("/"):
            crate_modules.add(part)
        if n.kind == "mod" and n.name:
            crate_modules.add(n.name)
    ident_free_prod = set(free)
    ident_free_all = set(free) | set(tfree)
    # what a test can see of other test files: what its file and containers `use`
    use_text = collections.defaultdict(str)
    for n in nodes:
        if n.test and n.file not in use_text:
            pass
    for key, (attrs, uses) in CONTAINERS.items():
        use_text[key[0]] += "\n".join(uses) + "\n"
    for n in nodes:
        pass

    includer_of = {}
    for m_ in nodes:
        if m_.test and m_.kind == "macro_call" and m_.fixed is None:
            for mm in re.finditer(r'include!\("([^"]+)"\)', m_.item.text):
                includer_of[os.path.normpath(os.path.join(os.path.dirname(m_.file), mm.group(1)))] = m_.file

    def visible(n, t, by_name=True):
        if t.file == n.file:
            return True
        if n.file in includer_of and (t.file == includer_of[n.file] or visible_from(includer_of[n.file], t, by_name)):
            return True  # an included file runs in the scope of the file that includes it
        # what the file imports, and through `use super::*` what each parent module imports or defines
        f = n.file
        for _ in range(8):
            text = use_text.get(f, "") + FILE_USES.get(f, "")
            stem = t.file[:-3].split("/")[-1]
            if re.search(r"\b" + re.escape(stem) + r"\b", text) or (by_name and t.name and re.search(r"\b" + re.escape(t.name) + r"\b", text)):
                return True
            parent = "/".join(f[:-3].split("/")[:-1]) + ".rs"
            if "super::*" not in text:
                return False
            if t.file == parent:
                return True
            f = parent
        return False
    def visible_from(f, t, by_name=True):
        class _N:
            file = f
        return visible(_N, t, by_name)

    placeholder = re.compile(r"\{([A-Za-z_][A-Za-z_0-9]*)(?:[:!][^{}]*)?\}")
    for n in nodes:
        ident_free = ident_free_all if n.test else ident_free_prod
        # implicit format arguments: "{INDEX}" names a const or static
        for m in placeholder.finditer(n.item.text):
            for t in list(free.get(m.group(1), ())) + (list(tfree.get(m.group(1), ())) if n.test else []):
                if t.kind in ("const", "static") and t is not n:
                    n.refs.add(t.id)
        for m in re.finditer(r"\b(?:default|with|serialize_with|deserialize_with|skip_serializing_if|from|into|try_from)\s*=\s*\"([A-Za-z_][\w:]*)\"", n.item.text):
            for t in list(free.get(m.group(1).split("::")[-1], ())) + (list(tfree.get(m.group(1).split("::")[-1], ())) if n.test else []):
                if t.kind == "fn" and t is not n:
                    n.refs.add(t.id)
        body = code_only(n.item.text)
        mine = n.name
        enclosing = n.merge if n.kind == "impl" else (store_type if n.kind == "method" else None)
        for m in IDENT.finditer(body):
            w = m.group(0)
            s = m.start()
            e = m.end()
            # what precedes / follows
            j = s - 1
            while j >= 0 and body[j] in " \t\n":
                j -= 1
            prev = body[j] if j >= 0 else ""
            prev2 = body[j - 1 : j + 1] if j >= 1 else ""
            k = e
            while k < len(body) and body[k] in " \t\n":
                k += 1
            nxt = body[k] if k < len(body) else ""
            if prev == "." and prev2 != "..":
                # method call: .w( or .w::<..>(
                if nxt == "(" or body.startswith("::<", k):
                    ts = list(methods.get(w, ())) + ([t for t in tmethods.get(w, ()) if visible(n, t)] if n.test else [])
                    for t in ts:
                        if t is n:
                            continue
                        if t.kind == "method" or (len(ts) == 1 and len(w) >= 6 and w not in STD_METHODS):
                            n.refs.add(t.id)
                continue
            qualifier = None
            if prev2 == "::":
                # qualified: find the qualifier
                q = j - 2
                while q >= 0 and body[q] in " \t\n":
                    q -= 1
                qe = q + 1
                while q >= 0 and (body[q].isalnum() or body[q] == "_"):
                    q -= 1
                qual = body[q + 1 : qe]
                qualifier = qual
                if qual in ("Self", store_type) and enclosing is not None and qual == "Self":
                    qual = enclosing
                tm = type_methods.get((qual, w))
                if tm:
                    for t in tm:
                        if t is not n:
                            n.refs.add(t.id)
                    continue
                if qual == store_type:
                    for t in methods.get(w, ()):
                        if t.kind == "method" and t is not n:
                            n.refs.add(t.id)
                    continue
                # module path to a free item, or an enum variant / assoc const: keep the free name
            if n.fixed is not None and qualifier is None and (n.file, w) in local_defs and w != mine:
                continue  # a crate already cut calls its own item of that name
            if qualifier is not None and qualifier[:1].islower() and qualifier not in crate_modules:
                continue  # `fs::read(..)`, `json::to_string(..)`: a path into another crate does not name our fn of that name
            if w in ident_free and w != mine:
                cands = list(free.get(w, ())) + (list(tfree.get(w, ())) if n.test else [])
                if n.test:
                    cands = [t for t in cands if not t.test or visible(n, t) or (qualifier and qualifier == t.file[:-3].split("/")[-1])]
                    # a name that the file imports from a module named in the import is that module's item
                    strong = [t for t in cands if visible(n, t, by_name=False)]
                    if strong and len(strong) < len(cands):
                        cands = strong
                if nxt == "!" and not body.startswith("!=", k):
                    cands = [t for t in cands if t.kind == "macro_rules"]
                elif not (w[0].isupper() or w.isupper()):
                    # a lower-case name is a fn (or module): it is a reference only when called or qualified
                    if body.startswith("::", k) and not body.startswith("::<", k) and prev2 != "::":
                        # `module::item`: the head names the module
                        cands = [t for t in cands if t.kind == "mod"]
                    else:
                        value_use = prev in "(,&" and nxt in "),"
                        if value_use and "_" not in w and _bound_locally(body, w):
                            value_use = False
                        if not (nxt == "(" or body.startswith("::<", k) or prev2 == "::" or value_use):
                            continue
                        cands = [t for t in cands if t.kind == "fn"]
                cands = [t for t in cands if _sees(n, t)]
                same_c = [t for t in cands if t.file == n.file and t.container == n.container and t.test == n.test]
                same = same_c or [t for t in cands if t.file == n.file]
                for t in (same or cands):
                    if t is not n:
                        n.refs.add(t.id)
            # a bound impl refers to its type
        if n.kind == "impl" and n.fixed is None and n.test:
            for t in tfree.get(n.merge, ()):
                if t.file == n.file and t.container == n.container:
                    n.refs.add(t.id)
                    t.refs.add(n.id)  # a test type and its impls rise together
        if n.kind == "impl" and n.fixed is None and not n.test:
            for t in free.get(n.merge, ()):
                n.refs.add(t.id)
                t.refs.add(n.id)  # the type needs its impls: bind them
            if n.item.impl_trait:
                for t in free.get(n.item.impl_trait, ()):
                    n.refs.add(t.id)
    # `Self::x` inside a method refers to Store methods
    return free, methods


def scc(nodes):
    n = len(nodes)
    sys.setrecursionlimit(1000000)
    idx = [-1] * n
    low = [0] * n
    on = [False] * n
    stack = []
    comp = [-1] * n
    cnt = [0, 0]

    def run():
        def sc(v):
            idx[v] = low[v] = cnt[0]
            cnt[0] += 1
            stack.append(v)
            on[v] = True
            for w in nodes[v].refs:
                if idx[w] < 0:
                    sc(w)
                    low[v] = min(low[v], low[w])
                elif on[w]:
                    low[v] = min(low[v], idx[w])
            if low[v] == idx[v]:
                while True:
                    w = stack.pop()
                    on[w] = False
                    comp[w] = cnt[1]
                    if w == v:
                        break
                cnt[1] += 1

        for v in range(n):
            if idx[v] < 0:
                sc(v)

    threading.stack_size(1024 * 1024 * 1024)
    t = threading.Thread(target=run)
    t.start()
    t.join()
    return comp


def load_config(path):
    return json.load(open(path))


def label_for(rel, config):
    for domain, pats in config["domains"].items():
        for p in pats:
            if re.fullmatch(p, rel):
                return domain
    return None


def plan(nodes, config, pins=(), current=None):
    order = config["order"]
    layer = {d: i + 1 for i, d in enumerate(order)}
    P = [n for n in nodes if not n.test or n.fixed is not None]
    comp = scc([n for n in nodes])
    sizes = collections.Counter(comp)
    big = max(sizes, key=sizes.get)
    pinned_keys = set(pins)
    roots = [n.id for n in P if n.fixed is None and (comp[n.id] == big or n.pinned or n.key() in pinned_keys or n.name in pinned_keys)]
    # the test items a crate already cut names stay where they are, and what they name stays with them
    if current is not None:
        roots += sorted({r for f in P if f.fixed is not None and f.fixed < current for r in f.refs if nodes[r].test and nodes[r].fixed is None})
    INF = 10**6
    referrers = collections.defaultdict(set)
    for n in P:
        for w in n.refs:
            referrers[w].add(n.id)
    for n in P:
        n.label = label_for(n.file, config)
    extra_roots = set()
    while True:
        core = set(roots) | extra_roots
        stack = list(core)
        while stack:
            v = stack.pop()
            for w in nodes[v].refs:
                if w not in core and nodes[w].fixed is None:
                    core.add(w)
                    stack.append(w)
        for n in P:
            n.core = n.id in core
        # labels for items without a file hint: the label that most of the (non-core) items they refer to carry
        changed = True
        while changed:
            changed = False
            for n in P:
                if n.core or n.label or n.fixed is not None:
                    continue
                votes = collections.Counter()
                for w in n.refs:
                    m = nodes[w]
                    if not m.core and m.label:
                        votes[m.label] += m.size
                if votes:
                    n.label = max(votes, key=votes.get)
                    changed = True
        for n in P:
            n.eff = n.fixed if n.fixed is not None else (0 if n.core else (layer[n.label] if n.label else INF))
        changed = True
        while changed:
            changed = False
            for n in P:
                if n.core or n.fixed is not None:
                    continue
                m = min((nodes[r].eff for r in referrers[n.id]), default=INF)
                if m < n.eff:
                    n.eff = m
                    changed = True
        floating = {n.id for n in P if n.fixed is None and not n.core and n.eff >= INF}
        if not floating:
            break
        extra_roots |= floating
    # tests go to the highest crate among the items they refer to; tests that refer to each other rise together
    T = [n for n in nodes if n.test and n.fixed is None]
    for t in T:
        t.eff = 0
    changed = True
    while changed:
        changed = False
        for t in T:
            if t.id in core:
                continue
            # an item of a layer above this one that is not cut yet is still down in the lower crate
            m = max((nodes[r].eff if (current is None or nodes[r].eff <= current) else 0 for r in t.refs if nodes[r].fixed is None), default=0)
            if m > t.eff:
                t.eff = m
                changed = True
    unify_includes(nodes, current)
    return comp, order


def unify_includes(nodes, current=None):
    """A test file included by `include!("x.rs")` moves together with the item that includes it."""
    T = [n for n in nodes if n.test and n.fixed is None]
    by_file = collections.defaultdict(list)
    for t in T:
        by_file[t.file].append(t)
    groups = []
    for t in T:
        for m in re.finditer(r'include!\("([^"]+)"\)', t.item.text):
            target = os.path.normpath(os.path.join(os.path.dirname(t.file), m.group(1)))
            if target in by_file:
                groups.append([t] + by_file[target])
    changed = True
    while changed:
        changed = False
        for g in groups:
            m = max(x.eff for x in g)
            for x in g:
                if x.eff < m:
                    x.eff = m
                    changed = True
    # whatever refers to a risen test rises with it
    changed = True
    while changed:
        changed = False
        for t in T:
            m = max((nodes[r].eff if (current is None or nodes[r].eff <= current) else 0 for r in t.refs if nodes[r].fixed is None), default=0)
            if m > t.eff:
                t.eff = m
                changed = True


def summarize(nodes, order):
    by = collections.Counter()
    cnt = collections.Counter()
    tests = collections.Counter()
    for n in nodes:
        if n.fixed is not None:
            continue
        if n.test:
            tests[n.eff] += n.size
            continue
        by[n.eff] += n.size
        cnt[n.eff] += 1
    names = ["core"] + order
    tot = sum(by.values())
    rows = []
    for k in sorted(by):
        rows.append((names[k] + f" (+{tests[k]} B tests)", cnt[k], by[k], 100 * by[k] / tot))
    return rows



WIDEN = re.compile(r"pub\((?:crate|super|self|in [A-Za-z_:]+)\)")


MODULE_PATH_OVERRIDE = {}  # file -> module path, for files declared with #[path = "..."]
TEST_ONLY_MODULES = set()  # module paths (tuples) declared with a plain `#[cfg(test)]`: they do not exist in a test-support build


def module_path(rel):
    return list(MODULE_PATH_OVERRIDE.get(rel) or rel[:-3].split("/"))


def camel(stem):
    return "".join(w.capitalize() for w in re.split(r"[_\W]+", stem) if w)


def split_method(text):
    """(docs, attrs, plain, sig, body) of one fn item text, where sig runs up to the body brace."""
    n = len(text)
    meta_start, code, _ = ri._leading_meta(text, 0, n)
    meta = text[:code]
    docs, attrs, plain = [], [], []
    k = 0
    for line in meta.split("\n"):
        t = line.strip()
        if not t:
            continue
        if t.startswith("///") or t.startswith("/**"):
            docs.append(t)
        elif t.startswith("#["):
            attrs.append(t)
        else:
            plain.append(t)
    # attributes may span lines: re-collect through the parser's own attribute list
    attrs = []
    j = 0
    while j < len(meta):
        m = re.compile(r"\s*#\[").match(meta, j)
        if m:
            e = ri._match_delim(meta, meta.index("[", j))
            attrs.append(meta[m.end() - 2 : e])
            j = e
        else:
            j += 1
    body_code = text[code:]
    m = ri._VIS.match(body_code)
    vis_end = m.end() if m else 0
    mm = re.match(r"((?:(?:default|async|unsafe|safe)\s+|const\s+(?=(?:async|unsafe|extern|fn)\b))*(?:extern\s+(?:\"[^\"]*\"\s+)?)?)fn\s+", body_code[vis_end:])
    bi, ch = ri._find_body_or_semi(body_code, vis_end + mm.end())
    sig = body_code[vis_end:bi].rstrip()
    body = body_code[bi:]
    return docs, attrs, plain, sig, body


def trait_decl_sig(sig):
    sig = re.sub(r"^(?:const\s+)", "", sig)
    sig = re.sub(r"(?<=[(,\s])mut\s+(?=[A-Za-z_]\w*\s*:)", "", sig)
    return sig


def make_trait(trait, methods):
    """The extension trait and its impl for Store, from the `impl Store` fn items of one file."""
    tl = ["pub trait %s {" % trait]
    il = ["impl %s for Store {" % trait]
    is_async = False
    for m in methods:
        docs, attrs, plain, sig, body = split_method(m.text)
        if re.search(r"\basync\s+fn\b", sig):
            is_async = True
        tsig = trait_decl_sig(sig)
        isig = re.sub(r"^(?:const\s+)", "", sig)
        t = "".join("    " + d + "\n" for d in docs) + "".join("    " + a + "\n" for a in attrs) + "    " + tsig.replace("\n", "\n    ") + ";"
        i = "".join("    " + a + "\n" for a in attrs) + "".join("    " + c + "\n" for c in plain) + "    " + isig.replace("\n", "\n    ") + " " + body
        tl.append(t)
        il.append(i)
    head = "#[allow(async_fn_in_trait)]\n" if is_async else ""
    return head + "\n".join(tl) + "\n}\n\n" + "\n\n".join([il[0]] + il[1:]) + "\n}\n"


def remove_spans(text, spans):
    for a, b in sorted(spans, reverse=True):
        # take the rest of the line and one blank line with it
        while b < len(text) and text[b] in " \t":
            b += 1
        if b < len(text) and text[b] == "\n":
            b += 1
        while a > 0 and text[a - 1] in " \t":
            a -= 1
        if a >= 1 and text[a - 1] == "\n" and b < len(text) and text[b] == "\n":
            b += 1
        text = text[:a] + text[b:]
    return text



def use_paths(text):
    """The `use` item as (is_pub_prefix_text, [(segments, alias)]) or None if it is not a plain use."""
    text = re.sub(r"\A(?:\s*//[^\n]*\n|\s*/\*.*?\*/)+", "", text, flags=re.S)
    m = re.match(r"((?:#\[[^\]]*\]\s*)*)(pub(?:\([^)]*\))?\s+)?use\s+(.*?);\s*$", text.strip(), re.S)
    if not m:
        return None
    attrs, vis, tree = m.group(1), m.group(2) or "", m.group(3)
    pos = [0]

    def ws():
        while pos[0] < len(tree) and tree[pos[0]].isspace():
            pos[0] += 1

    def parse(prefix):
        out = []
        ws()
        # a path: seg(::seg)* [::{...}] [as alias]
        segs = []
        while True:
            ws()
            if tree[pos[0]] == "{":
                pos[0] += 1
                while True:
                    out += parse(prefix + segs)
                    ws()
                    if tree[pos[0]] == ",":
                        pos[0] += 1
                        ws()
                        if tree[pos[0]] == "}":
                            break
                        continue
                    break
                ws()
                assert tree[pos[0]] == "}"
                pos[0] += 1
                return out
            m2 = re.compile(r"[A-Za-z_][A-Za-z_0-9]*|\*").match(tree, pos[0])
            segs.append(m2.group(0))
            pos[0] = m2.end()
            ws()
            if tree.startswith("::", pos[0]):
                pos[0] += 2
                continue
            break
        alias = None
        if re.compile(r"as\s+").match(tree, pos[0]):
            pos[0] += 2
            ws()
            m3 = re.compile(r"[A-Za-z_][A-Za-z_0-9]*").match(tree, pos[0])
            alias = m3.group(0)
            pos[0] = m3.end()
        return [(prefix + segs, alias)]

    try:
        paths = parse([])
    except Exception:
        return None
    return attrs, vis, paths


def render_use(attrs, vis, paths):
    if not paths:
        return ""
    groups = collections.OrderedDict()
    for segs, alias in paths:
        key = tuple(segs[:-1])
        leaf = segs[-1] + (f" as {alias}" if alias else "")
        groups.setdefault(key, []).append(leaf)
    lines = []
    for key, leaves in groups.items():
        base = "::".join(key)
        if len(leaves) == 1:
            lines.append(f"{attrs}{vis}use {base + '::' if base else ''}{leaves[0]};")
        else:
            lines.append(f"{attrs}{vis}use {base + '::' if base else ''}{{{', '.join(leaves)}}};")
    return "\n".join(lines)


_EXTERNAL = None


def _external_crates():
    """Identifiers of the crates a `use` can start with that are not this workspace's modules: std and the dependencies."""
    global _EXTERNAL
    if _EXTERNAL is None:
        names = {"std", "core", "alloc"}
        for manifest in Path("crates").glob("*/Cargo.toml"):
            for m in re.finditer(r"(?m)^([A-Za-z0-9_-]+)\s*=", manifest.read_text()):
                names.add(m.group(1).replace("-", "_"))
        names |= {p.name.replace("-", "_") for p in Path("crates").iterdir()}
        _EXTERNAL = names
    return _EXTERNAL


def prune_uses(text, items, moved_pairs):
    """Drop from the `use` items of a lower file the paths that name moved items: (module, name) pairs."""
    spans = []

    def walk(its):
        for it in its:
            if it.kind == "mod" and it.body:
                walk(ri.parse_items(text, it.body[0], it.body[1]))  # inline modules (`mod tests { use ..; }`)
                continue
            if it.kind != "use":
                continue
            parsed = use_paths(it.text)
            if not parsed:
                continue
            attrs, vis, paths = parsed
            keep = []
            for segs, alias in paths:
                name = segs[-1]
                mod = segs[-2] if len(segs) >= 2 else None
                if (mod, name) in moved_pairs and segs[0] not in _external_crates():
                    continue
                keep.append((segs, alias))
            if len(keep) != len(paths):
                spans.append((_attr_start(text, it), it.end, render_use(attrs, vis, keep) if keep else ""))

    walk(items)
    for a, b, rep in sorted(spans, reverse=True):
        text = text[:a] + rep + text[b:]
    return text



def _without_unused_relative(use_text, body):
    """A `use super::...` import of names the text below never mentions (it served code that stayed behind)."""
    parsed = use_paths(use_text)
    if not parsed:
        return use_text
    attrs, vis, paths = parsed
    keep = [(segs, alias) for segs, alias in paths
            if segs[0] != "super" or segs[-1] == "*" or re.search(r"\b" + re.escape(alias or segs[-1]) + r"\b", body)]
    if len(keep) == len(paths):
        return use_text
    return render_use(attrs, vis, keep)


def _without_names(use_text, names):
    """A `use` item without the paths that import one of `names` (they are defined next to the user)."""
    parsed = use_paths(use_text)
    if not parsed:
        return use_text
    attrs, vis, paths = parsed
    keep = [(segs, alias) for segs, alias in paths if (alias or segs[-1]) not in names]
    if len(keep) == len(paths):
        return use_text
    return render_use(attrs, vis, keep)


def _copy_text(u):
    """The text of a test item copied next to the tests that use it: a local helper (no `pub`), and
    allowed to have parts nobody here uses."""
    text = u.item.text
    code = ri._leading_meta(text, 0, len(text))[1]
    if u.kind == "fn":
        m = ri._VIS.match(text, code)
        if m:
            text = text[:code] + text[m.end():]
    elif u.kind in ("struct", "enum", "union", "impl"):
        text = text[:code] + "#[allow(dead_code)]\n" + text[code:]
    return text


def is_vacant(text):
    """Nothing in the file but `use` items and modules that hold only the same."""
    for it in ri.parse(text):
        if it.kind == "use":
            continue
        if it.kind == "mod" and it.body is not None and is_vacant(text[it.body[0] : it.body[1]]):
            continue
        return False
    return True


def _shared_import_lines(lower, mk, shares, prefix="", vis=""):
    """`use` lines importing a shared module into a destination: the glob, and its names explicitly (an explicit
    import wins over the globs of the surrounding scope, which the helpers used to shadow)."""
    path = "::".join([lower] + _shared_path(*mk))
    names = set()
    for n in shares.get(mk, ()):
        if n.kind in ("impl",):
            continue
        names |= set(n.names) if n.kind == "macro_call" else ({n.name} if n.name else set())
    names = sorted(names)
    lines = []
    for k in range(0, len(names), 10):
        lines.append("%s%suse %s::{%s};" % (prefix, vis, path, ", ".join(names[k : k + 10])))
    return lines


def place_tests(nodes, mtests, files, tfiles, lower):
    """Where the moved tests land and what they leave behind.

    Returns (facade_blocks, new_files, decls, removals, copied): the text appended to the facades of
    production files, the new test files (rel -> text), the `mod x;` declarations the facades gain, the
    spans to remove from the lower crate (rel -> [(a, b)]), and the lower test items copied along (a test
    that moves needs the helpers it calls, which also stay for the tests that remain)."""
    moved_ids = {t.id for t in mtests}
    groups = collections.defaultdict(lambda: collections.defaultdict(list))  # rel -> container -> [(key, text)]
    removals = collections.defaultdict(list)
    for t in mtests:
        groups[t.file][t.container].append(((t.file, t.item.start), t.item.text))
        removals[t.file].append((t.item.start, t.item.end))
    copied = []
    seen = set()
    test_types = {(n.file, n.container, n.name): n for n in nodes if n.test and n.fixed is None and n.kind in ("struct", "enum", "union", "type", "trait")}

    def pub_support(rel):
        parts = module_path(rel)
        parent = ("/".join(parts[:-1]) + ".rs") if len(parts) > 1 else "lib.rs"
        for it in (files.get(parent) or tfiles.get(parent) or (None, []))[1]:
            if it.kind == "mod" and it.name == parts[-1] and it.text.rstrip().endswith(";"):
                return it.is_pub and any("test-support" in a for a in it.attrs)
        return False

    def shared_pub(n):
        """Whether code outside this test module can name the type: pub, in modules that are pub (a
        prod file, a pub test-support file, or inline modules declared pub)."""
        if not n.item.is_pub or any(re.fullmatch(r"#\[cfg\(test\)\]", at.strip()) for at in n.item.attrs):
            return False
        if any(not CONTAINERS.get((n.file, n.container[: k + 1]), ("", []))[0].rstrip().endswith("pub") or "#[cfg(test)]" in CONTAINERS.get((n.file, n.container[: k + 1]), ("", []))[0] for k in range(len(n.container))):
            return False
        return n.file in files or pub_support(n.file)
    defs_by_name = collections.defaultdict(list)
    for n in nodes:
        if n.test and n.fixed is None and n.name:
            defs_by_name[n.name].append(n)
    dest_names = collections.defaultdict(set)  # (file, container) -> names already there
    for t in mtests:
        if t.name:
            dest_names[(t.file, t.container)].add(t.name)
    shares = collections.defaultdict(list)  # (file, container) -> lower test nodes extracted into a shared module
    share_ids = set()
    imports = collections.defaultdict(set)  # dest (file, container) -> shared modules to glob-import
    share_imports = collections.defaultdict(set)  # shared module key -> other shared modules it needs
    dev = _dev_dependencies(lower)

    def mod_text(n):
        return (files.get(n.file) or tfiles.get(n.file))[0]

    def usable(n):
        return not any(re.search(r"\b" + re.escape(d) + r"(?:::|!)", n.item.text) for d in dev)

    work = [(t, (t.file, t.container), "dest") for t in mtests]
    while work:
        t, k, kind = work.pop()
        for r in t.refs:
            u = nodes[r]
            if not u.test or u.kind == "method" or u.id in moved_ids or u.fixed is not None:
                continue
            if u.kind in ("struct", "enum", "union", "type", "trait", "fn", "const", "static") and shared_pub(u):
                continue  # pub in a module others can reach already
            if u.kind == "impl" and test_types.get((u.file, u.container, u.merge)) is not None and shared_pub(test_types[(u.file, u.container, u.merge)]):
                continue
            if u.name and (u.file != t.file or u.container != t.container) and sum(1 for x in t.refs if nodes[x].test and nodes[x].name == u.name) > 1:
                continue  # several visible candidates: a name that probably names something else
            mk = (u.file, u.container)
            if kind == "dest":
                imports[k].add(mk)
            elif mk != k:
                share_imports[k].add(mk)
            if u.id in share_ids:
                continue
            if not usable(u):
                print("WARNING: %s uses a dev-dependency and cannot be shared with %s" % (u.key(), t.key()))
                continue
            share_ids.add(u.id)
            shares[mk].append(u)
            work.append((u, mk, "share"))
    for mk, ns in shares.items():
        for u in ns:
            removals[u.file].append((u.item.start, u.item.end))
    copied = []

    defined = collections.defaultdict(set)  # (file, container) -> names defined there by moved or copied items
    for t in mtests:
        if t.name:
            defined[(t.file, t.container)].add(t.name)

    def render(rel, prefix, g):
        out = []
        for _, text in sorted(g.get(prefix, ()), key=lambda x: x[0]):
            out.append(("    " * len(prefix)) + text if prefix else text)
        names = sorted({c[len(prefix)] for c in g if len(c) > len(prefix) and c[: len(prefix)] == prefix})
        for nm in names:
            c = prefix + (nm,)
            attrs, uses = CONTAINERS.get((rel, c), ("#[cfg(test)]\n", []))
            inner = render(rel, c, g)
            pad = "    " * len(prefix)
            uses = [u2 for u2 in (_without_names(u, defined[(rel, c)] | {x.name for x in copied if x.name}) for u in uses) if u2]
            for mk in sorted(imports.get((rel, c), ())):
                uses += _shared_import_lines(lower, mk, shares)
            if attrs.rstrip().endswith("pub") and "test-support" in attrs:
                uses.append("pub use %s::%s::*;" % (lower, "::".join(module_path(rel) + list(c))))
            body = "\n".join(pad + "    " + u for u in uses) + ("\n\n" if uses else "") + inner
            out.append(f"{pad}{attrs}mod {nm} {{\n{body}\n{pad}}}")
        return "\n\n".join(x for x in out if x)

    included = set()
    for n in nodes:
        if n.test and n.fixed is None and n.kind == "macro_call":
            for m in re.finditer(r'include!\("([^"]+)"\)', n.item.text):
                included.add(os.path.normpath(os.path.join(os.path.dirname(n.file), m.group(1))))
    facade_blocks, new_files, decls = {}, {}, collections.defaultdict(list)
    pub_test_mods = set()
    helper_names = {u.name for u in copied if u.name}
    # a new test file needs its test-file parents too
    for rel in list(groups):
        r = rel
        while r in tfiles:
            parts = module_path(r)
            parent = "/".join(parts[:-1]) + ".rs"
            if parent in tfiles:
                groups[parent]
                r = parent
            else:
                break
    for rel in groups:
        if rel in files:
            continue
        parts = module_path(rel)
        parent = ("/".join(parts[:-1]) + ".rs") if len(parts) > 1 else "lib.rs"
        if parent not in files and parent not in tfiles:
            print("WARNING: no parent module file %s for test file %s" % (parent, rel))
            continue
        ptext, pitems = files.get(parent) or tfiles.get(parent)
        for it in pitems:
            if it.kind == "mod" and it.name == parts[-1] and it.text.rstrip().endswith(";"):
                decls[parent].append(it.text)
                if it.is_pub and any("test-support" in a for a in it.attrs):
                    pub_test_mods.add(rel)
    for rel, g in groups.items():
        body = render(rel, (), g)
        body = re.sub(r"\b(?:super::)+(?:tests|\w+_tests)::(" + "|".join(map(re.escape, sorted(helper_names))) + r")\b", r"\1", body) if helper_names else body
        if rel in files:
            globs = [ln for mk in sorted(imports.get((rel, ()), ())) for ln in _shared_import_lines(lower, mk, shares, '#[cfg(any(test, feature = "test-support"))]\n', "pub ")]
            facade_blocks[rel] = ("\n".join(globs) + "\n" if globs else "") + body
        else:
            text, items = tfiles[rel]
            local = {u.name for u in copied if u.name} | {t.name for t in mtests if t.name}
            uses = [_without_unused_relative(_without_names(it.text, local), body) for it in items if it.kind == "use"]
            uses = [u2 for u2 in _shared_uses(uses, rel, (), items, shares, [], set(), set(tfiles), root=lower, dest=True, lower_files={**files, **tfiles}) if u2]
            globs = [ln for mk in sorted(imports.get((rel, ()), ())) for ln in _shared_import_lines(lower, mk, shares)]
            new_files[rel] = ("" if rel in included else "#![allow(unused_imports)]\n") + "\n".join(u for u in uses + globs if u) + "\n" + "\n".join(decls.get(rel, [])) + "\n\n" + body + "\n"
    return facade_blocks, new_files, decls, removals, copied, pub_test_mods, shares, share_imports, imports



def _dev_dependencies(lower_ident):
    """The dev-dependency crate names (as identifiers) of the lower crate."""
    manifest = Path("crates") / lower_ident.replace("_", "-") / "Cargo.toml"
    if not manifest.exists():
        return []
    text = manifest.read_text()
    if "[dev-dependencies]" not in text:
        return []
    section = text.split("[dev-dependencies]", 1)[1].split("\n[", 1)[0]
    names = [m.group(1).replace("-", "_") for m in re.finditer(r"(?m)^([A-Za-z0-9_-]+)\s*=", section)]
    normal = text.split("[dependencies]", 1)[1].split("\n[", 1)[0] if "[dependencies]" in text else ""
    normal_names = {m.group(1).replace("-", "_") for m in re.finditer(r"(?m)^([A-Za-z0-9_-]+)\s*=", normal)}
    return [n for n in names if not n.endswith("_self") and not n.startswith("st3") and n not in normal_names]


TEST_SUPPORT_CFG = '#[cfg(any(test, feature = "test-support"))]'


def _vis_position(text, start, code):
    """Where `pub` goes for an item whose code starts at `code`: before the block comments in front of it (a
    SQLite inventory marker has to stay adjacent to the `fn`)."""
    pos = code
    while True:
        head = text[start:pos].rstrip()
        if not head.endswith("*/"):
            break
        k = text.rfind("/*", start, pos)
        if k < 0:
            break
        pos = k
    return pos


def _widen_item_edits(text, it):
    """Edits that make one item pub and, if it is gated by plain `cfg(test)`, gated by test-support too."""
    edits = []
    code = ri._leading_meta(text, it.start, len(text))[1]
    for m in re.finditer(r"#\[cfg\(test\)\]", text[it.start : code]):
        edits.append((it.start + m.start(), it.start + m.end(), TEST_SUPPORT_CFG))
    vis = ri._VIS.match(text, code)
    if vis is None:
        pos = _vis_position(text, it.start, code)
        edits.append((pos, pos, "pub "))
    elif vis.group(0).strip() != "pub":
        edits.append((code, vis.end(), "pub "))
    return edits


def _available(path):
    """Whether the module exists in a test-support build: no module on the way is `#[cfg(test)]` alone."""
    return not any(tuple(path[:k]) in TEST_ONLY_MODULES for k in range(1, len(path) + 1))


def _shared_base(rel):
    """The module a file's shared module sits in: next to the file when that exists in a test-support build,
    else in the nearest ancestor that does. A top-level file's shared module is its child (a sibling would be
    a second top-level module that depends on it both ways)."""
    p = module_path(rel)
    if len(p) == 1:
        return p
    base = p[:-1]
    while base and not _available(base):
        base = base[:-1]
    return base


SHARED_SUFFIX = ""  # a cut on top of another names its shared modules apart from that cut's


def _shared_name(rel, container):
    p = module_path(rel)
    base = _shared_base(rel)
    suffix = ("_" + "_".join(container)) if container else ""
    if len(p) == 1:
        return "shared" + suffix + SHARED_SUFFIX
    return "_".join(p[len(base):]) + suffix + "_shared" + SHARED_SUFFIX


def _shared_path(rel, container):
    return _shared_base(rel) + [_shared_name(rel, container)]


def _insert_position(text, container):
    """Where a `use` line goes in the module (file level or the inline module `container`)."""
    scope_a, items = 0, ri.parse(text)
    for name in container:
        m = next((x for x in items if x.kind == "mod" and x.name == name and x.body), None)
        if m is None:
            return None
        scope_a = m.body[0]
        items = ri.parse_items(text, m.body[0], m.body[1])
    uses = [x for x in items if x.kind == "use"]
    if uses:
        return uses[0].start
    return items[0].start if items else scope_a


def _resolve_use(segs, base):
    """Absolute segments (below `crate`) of a use path written in a module whose own path is `base`."""
    if segs[0] == "crate":
        return list(segs[1:])
    cur = list(base)
    i = 0
    while i < len(segs) and segs[i] in ("super", "self"):
        if segs[i] == "super":
            cur = cur[:-1]
        i += 1
    if i == 0:
        return None  # an external crate or a name from scope: leave it
    return cur + list(segs[i:])


def _prior_shared(lower_files, module):
    """[(module path, names it defines)] of the shared modules an earlier cut made for the test-only `module`."""
    out = []
    # a shared module sits in the nearest ancestor that exists in a test-support build, named after the rest of the path
    want = module[:-1]
    while want and not _available(want):
        want = want[:-1]
    stem = "_".join(module[len(want) :]) + "_shared"
    for rel2, (text2, items2) in lower_files.items():
        mp = module_path(rel2)
        if mp[:-1] == want and (mp[-1] == stem or mp[-1].startswith(stem + "_")):
            names = set()
            for it in items2:
                if it.name and it.is_pub:
                    names.add(it.name)
                if it.kind == "macro_call":
                    names |= set(re.findall(r"static\s+(\w+)", it.text))
            out.append((mp, names))
    return out


def _shared_uses(uses, rel, cont, items, shares, dev, pairs, test_rels, root="crate", dest=False, lower_files=None, depth=0):
    shared_names = {tuple(module_path(r2)): {n.name for n in ns if n.name} for (r2, c2), ns in shares.items() if not c2}
    """The `use` items of the file a shared module is cut from, adjusted to where the module sits: dev
    dependencies dropped, child modules named absolutely, modules that only exist for tests replaced by
    their shared module (or dropped), and names that moved away pruned."""
    f_path = module_path(rel)
    children = {it.name for it in items if it.kind == "mod"}
    shared_of = {tuple(module_path(r2)): _shared_path(r2, c2) for (r2, c2) in shares if not c2 and r2 in test_rels}
    out = []
    extra_uses = []
    for u in uses:
        parsed = use_paths(u)
        if not parsed:
            continue
        attrs, vis, paths = parsed
        attrs = re.sub(r"#\[cfg\(test\)\]\s*", "", attrs)
        keep = []
        for segs, alias in paths:
            orig_segs = segs
            if segs[0] in dev:
                continue
            if not dest and segs[0] in children:
                segs = ["crate"] + f_path + segs
            elif not dest and segs[0] == "self":
                segs = ["crate"] + f_path + segs[1:]
            absolute = _resolve_use(segs, f_path[:-1] + ["_"])
            if absolute is not None and segs[0] in ("super", "self", "crate") and not dest:
                segs = ["crate"] + absolute  # the shared module may not sit where the file does
            extra = None
            if absolute is not None:
                hit = None
                for k in range(len(absolute), 0, -1):
                    if tuple(absolute[:k]) in shared_of:
                        hit = (k, shared_of[tuple(absolute[:k])])
                        break
                test_only = any(seg == "tests" or seg.endswith("_tests") for seg in absolute[:-1]) or (len(absolute) > 1 and not _available(absolute[:-1]))
                if hit is not None:
                    leaf = segs[-1]
                    if leaf == "*":
                        if len(absolute) == hit[0] + 1:
                            extra = [root] + hit[1] + absolute[hit[0]:]  # the shared module's items too
                    elif len(absolute) == hit[0] + 1 and leaf in shared_names.get(tuple(absolute[: hit[0]]), set()):
                        segs = [root] + hit[1] + absolute[hit[0]:]
                        test_only = False
                    elif len(absolute) == hit[0] + 1:
                        # not extracted now: an earlier cut may have put it in another shared module of that module
                        prior = next((mp for mp, defined in _prior_shared(lower_files or {}, list(absolute[:-1])) if leaf in defined), None)
                        if prior is None:
                            continue  # it stays in the cfg(test) module
                        segs = [root] + prior + [leaf]
                        test_only = False
                    # deeper than the module (a child module): the path stays as it is
                if test_only and lower_files:
                    # an earlier cut already moved this module's helpers into its shared module
                    prior = _prior_shared(lower_files, list(absolute[:-1]))
                    leaf = segs[-1]
                    if leaf == "*":
                        for mod_path, defined in prior:
                            keep.append(([root] + mod_path + ["*"], None))
                    else:
                        for mod_path, defined in prior:
                            if leaf in defined:
                                segs = [root] + mod_path + [leaf]
                                test_only = False
                                break
                if test_only:
                    if segs[-1] == "*" and lower_files and depth < 6 and not dest:
                        # the glob served names that module imported itself: take its imports instead
                        rel2 = next((r for r in lower_files if module_path(r) == list(absolute[:-1])), None)
                        if rel2 is not None:
                            items2 = lower_files[rel2][1]
                            extra_uses.extend(_shared_uses([it.text for it in items2 if it.kind == "use"], rel2, (), items2, shares, dev, pairs, test_rels, root, dest, lower_files, depth + 1))
                    segs = None  # exists only for cfg(test)
            if extra is not None:
                keep.append((extra, None))
            if segs is None:
                if dest:
                    keep.append((orig_segs, alias))  # a destination file may still have that module itself
                continue
            mod = segs[-2] if len(segs) >= 2 else None
            if (mod, segs[-1]) in pairs:
                continue
            keep.append((segs, alias))
        if keep:
            out.append(render_use(attrs, vis, keep))
    return out + [u for u in extra_uses if u not in out]


def _make_dev_optional(manifest, idents):
    """Each dev-dependency in `idents` also becomes an optional dependency that the test-support feature enables."""
    text = manifest.read_text()
    dev_section = text.split("[dev-dependencies]", 1)[1].split("\n[", 1)[0] if "[dev-dependencies]" in text else ""
    for ident in sorted(idents):
        m = re.search(r"(?m)^(" + re.escape(ident).replace("_", "[-_]") + r")\s*=\s*(.+)$", dev_section)
        if not m:
            continue
        name, spec = m.group(1), m.group(2).strip()
        deps = text.split("[dependencies]", 1)[1].split("\n[", 1)[0] if "[dependencies]" in text else ""
        if re.search(r"(?m)^" + re.escape(name) + r"\s*=", deps):
            continue
        if spec.startswith("{"):
            spec = spec.rstrip("}").rstrip() + ", optional = true }"
        else:
            spec = "{ version = %s, optional = true }" % spec
        text = text.replace("[dependencies]\n", "[dependencies]\n%s = %s\n" % (name, spec), 1)
        text = re.sub(r'(?m)^(test-support\s*=\s*\[)', r'\1"dep:%s", ' % name, text, count=1)
    manifest.write_text(text)


def _attr_start(text, it):
    """Where the attributes of a `use` item begin (its comments stay): `render_use` writes the attributes itself."""
    meta, code, _ = ri._leading_meta(text, it.start, len(text))
    at = text[meta:code].find("#[")
    return meta + at if at >= 0 else code


def _pub_tuple_fields(text):
    """`struct First(i32);` -> `struct First(pub i32);`"""
    m = re.search(r"\bstruct\s+\w+(?:<[^>(]*>)?\s*\(", text)
    if not m:
        return text
    end = ri._match_delim(text, m.end() - 1)
    inner = text[m.end() : end - 1]
    parts, depth, cur = [], 0, ""
    for ch in inner:
        if ch in "<([":
            depth += 1
        elif ch in ">)]":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += ch
    parts.append(cur)
    parts = [p if p.strip() == "" or p.lstrip().startswith("pub") else re.sub(r"^(\s*)", r"\1pub ", p) for p in parts]
    return text[: m.end()] + ",".join(parts) + text[end - 1 :]


def _pub_fields(text):
    """A shared struct's named fields are public: the tests that use it live in another crate."""
    if re.search(r"\bstruct\s+\w+(?:<[^>(]*>)?\s*\(", text) and not re.search(r"\bstruct\b[^{;(]*\{", text):
        return _pub_tuple_fields(text)
    m = re.search(r"\bstruct\b[^{;]*\{", text)
    end = ri._match_delim(text, m.end() - 1)
    body = text[m.end() : end - 1]
    nb = re.sub(r"(?m)^(\s*)(?!pub\b)(?!//)(?!#)(\w+)\s*:", r"\1pub \2:", body)
    return text[: m.end()] + nb + text[end - 1 :]


def _drop_use_names(text, names, keep_path):
    """Remove the explicit imports of `names` (except from `keep_path`): they are provided by a re-export."""
    items = [it for it in ri.parse(text) if it.kind == "use"]
    for it in sorted(items, key=lambda i: -i.start):
        parsed = use_paths(it.text)
        if not parsed:
            continue
        attrs, vis, paths = parsed
        keep = [(segs, alias) for segs, alias in paths if segs[-1] == "*" or (alias or segs[-1]) not in names or "::".join(segs).startswith("crate::" + keep_path)]
        if len(keep) != len(paths):
            code = _attr_start(text, it)
            text = text[:code] + (render_use(attrs, vis, keep) if keep else "") + text[it.end :]
    return text


def _dedupe_uses(uses):
    """The same path imported twice (the file's and its container's `use`) is an error: keep the first. A shared
    module only exists under test-support, so an import gated by that feature repeats an ungated one."""
    gate = re.compile(r"\s*#\[cfg\(any\(test, feature = \"test-support\"\)\)\]\s*")
    ungated = set()
    for u in uses:
        parsed = use_paths(u)
        if parsed and not parsed[0].strip():
            ungated |= {("::".join(segs), alias) for segs, alias in parsed[2]}
    seen = set()
    out = []
    for u in uses:
        parsed = use_paths(u)
        if not parsed:
            out.append(u)
            continue
        attrs, vis, paths = parsed
        plain = gate.fullmatch(attrs) is not None
        keep = []
        for segs, alias in paths:
            key = ("" if plain or not attrs.strip() else attrs, "::".join(segs), alias)
            if segs[-1] != "*" and (key in seen or (attrs.strip() and plain and ("::".join(segs), alias) in ungated)):
                continue
            seen.add(key)
            keep.append((segs, alias))
        if keep:
            out.append(render_use(attrs, vis, keep) if len(keep) != len(paths) else u)
    return out


def apply_shares(shares, share_imports, src_dir, lower_files, pairs, dev, test_rels, includers, dest_src=None, dest_root=None):
    """Extract the lower test items the moved tests need into a shared module next to their file, gated by
    test-support, and re-export it where the items were (`pub use`), so old paths keep resolving."""
    added_decls = collections.defaultdict(list)
    needed_dev = set()
    for (rel, cont), ns in sorted(shares.items()):
        name = _shared_name(rel, cont)
        parts = module_path(rel)
        directory = "/".join(_shared_base(rel))
        path = os.path.join(src_dir, directory, name + ".rs") if directory else os.path.join(src_dir, name + ".rs")
        text, items = lower_files[rel]
        file_uses = [it.text for it in items if it.kind == "use"]
        inc = includers.get(rel) if not cont else None
        if inc is not None:
            # an included file runs in the scope of the module that includes it
            irel, icont = inc
            iitems = lower_files[irel][1]
            ifile_uses = [it.text for it in iitems if it.kind == "use"]
            uses = list(ifile_uses) + [u for u in file_uses]
            for u in CONTAINERS.get((irel, icont), ("", []))[1]:
                if re.fullmatch(r"\s*use super::\*;\s*", u):
                    uses.append("use crate::%s::*;" % "::".join(module_path(irel)))
                else:
                    uses.append(re.sub(r"\buse super::", "use crate::%s::" % "::".join(module_path(irel)), u))
        elif not cont:
            uses = list(file_uses)
            if rel not in test_rels:
                # a production file's own items (the ones that stay) are neighbours of the shared module;
                # named explicitly, because a glob import of the same name elsewhere would make them ambiguous
                uses.append("use crate::%s::*;" % "::".join(parts))
                groups_ = collections.defaultdict(set)
                cur_path = os.path.join(src_dir, rel)
                cur_items = ri.parse(Path(cur_path).read_text()) if os.path.exists(cur_path) else []
                for it in cur_items:
                    if it.kind in FREE_KINDS and it.kind != "mod" and it.is_pub and it.name and not any(x.startswith("#[test]") for x in it.attrs):
                        groups_[tuple(x for x in it.attrs if x.startswith("#[cfg"))].add(it.name)
                for cfgs, names_ in sorted(groups_.items()):
                    own = sorted(names_)
                    for k in range(0, len(own), 12):
                        uses.append("".join(c + "\n" for c in cfgs) + "use crate::%s::{%s};" % ("::".join(parts), ", ".join(own[k : k + 12])))
        else:
            # inside the inline module `super` is the file's module; here that module is a sibling
            uses = list(file_uses)
            for u in CONTAINERS.get((rel, cont), ("", []))[1]:
                if re.fullmatch(r"\s*use super::\*;\s*", u):
                    uses.append("use crate::%s::*;" % "::".join(parts))
                else:
                    uses.append(re.sub(r"\buse super::", "use crate::%s::" % "::".join(parts), u))
        # a dev-dependency the shared helpers use has to exist in a test-support build: it becomes optional
        kept_dev = {d for d in dev if any(re.search(r"\buse\s+" + d + r"\b|\b" + d + r"::", u) for u in uses)}
        needed_dev |= kept_dev
        uses = _dedupe_uses(_shared_uses(uses, rel, cont, items, shares, [d for d in dev if d not in kept_dev], pairs, test_rels, lower_files=lower_files))
        globs = ["use %s::*;" % "::".join(["crate"] + _shared_path(*mk)) for mk in sorted(share_imports.get((rel, cont), ()))]
        body = []
        for u in sorted(ns, key=lambda n: n.item.start):
            t = re.sub(r"#\[cfg\(test\)\]\s*", "", u.item.text)
            code = ri._leading_meta(t, 0, len(t))[1]
            vis = ri._VIS.match(t, code)
            if u.kind in ("fn", "const", "static", "struct", "enum", "type", "trait", "union"):
                if vis:
                    t = t[:code] + "pub " + t[vis.end():]
                else:
                    pos = _vis_position(t, 0, code)
                    t = t[:pos] + "pub " + t[pos:]
            if u.kind == "fn" and re.search(r"\bpub\s+unsafe\b|\bunsafe\s+(?:extern\s+\"C\"\s+)?fn\b", t) and "missing_safety_doc" not in t:
                code_ = ri._leading_meta(t, 0, len(t))[1]
                t = t[:code_] + "#[allow(clippy::missing_safety_doc)]\n" + t[code_:]
            if u.kind == "struct" and (re.search(r"\bstruct\b[^{;]*\{", t) or re.search(r"\bstruct\s+\w+(?:<[^>(]*>)?\s*\(", t)):
                t = _pub_fields(t)
            if u.kind == "macro_call" and t.lstrip().startswith(("thread_local!", "#[")) and "thread_local!" in t:
                t = re.sub(r"(?m)^(\s*)(?:pub(?:\([^)]*\))?\s+)?static\b", r"\1pub static", t)
            body.append(t)
        # imports of the parent module that only exist for cfg(test) are not there in a test-support build
        extras = []
        parent_rel = ("/".join(parts[:-1]) + ".rs") if len(parts) > 1 else None
        body_text = "\n".join(body)
        for pit in (lower_files.get(parent_rel, (None, []))[1] if parent_rel else []):
            if pit.kind != "use" or "cfg(test)" not in pit.text.split("use ", 1)[0]:
                continue
            parsed_p = use_paths(pit.text)
            if not parsed_p:
                continue
            for segs, alias in parsed_p[2]:
                leaf = alias or segs[-1]
                if leaf == "*" or not re.search(r"\b" + re.escape(leaf) + r"\b", body_text):
                    continue
                absolute = _resolve_use(segs, module_path(parent_rel) + ["_"])
                target = (["crate"] + absolute) if absolute is not None else segs
                extras.append(render_use("", "", [(target, alias)]))
        uses = _dedupe_uses(uses + [e for e in extras if e not in uses])
        Path(path).parent.mkdir(parents=True, exist_ok=True)
        Path(path).write_text("#![allow(unused_imports)]\n" + "\n".join(uses + globs) + "\n\n" + "\n\n".join(body) + "\n")
        parent = (directory + ".rs") if directory else "lib.rs"
        added_decls[parent].append('#[cfg(any(test, feature = "test-support"))]\npub mod %s;\n' % name)
        # the original module re-exports what it gave away
        opath = os.path.join(src_dir, rel)
        if os.path.exists(opath):
            otext = Path(opath).read_text()
            pos = _insert_position(otext, cont)
            if pos is not None:
                spath = "::".join(_shared_path(rel, cont))
                line = ""
                named = sorted({x for u in ns for x in (u.names if u.kind == "macro_call" else ([u.name] if u.name else [])) if u.kind != "impl"})
                for k in range(0, len(named), 10):
                    line += '#[cfg(any(test, feature = "test-support"))]\n#[allow(unused_imports)]\npub use crate::%s::{%s};\n' % (spath, ", ".join(named[k : k + 10]))
                newtext = otext[:pos] + line + otext[pos:]
                if not cont:
                    newtext = _drop_use_names(newtext, set(named), spath)
                Path(opath).write_text(newtext)
    if needed_dev:
        _make_dev_optional(Path(src_dir).parent / "Cargo.toml", needed_dev)
    for parent, decls in added_decls.items():
        ppath = os.path.join(src_dir, parent)
        ptext = Path(ppath).read_text()
        Path(ppath).write_text(ptext.rstrip("\n") + "\n\n" + "\n".join(decls))
    # paths that go through a cfg(test)-only module to an item that now lives in its shared module
    renames = []
    for (rel, cont), ns in shares.items():
        if not cont and rel in test_rels:
            names = sorted({n.name for n in ns if n.name})
            if names:
                stem = module_path(rel)[-1]
                renames.append((re.compile(r"(?<![\w:])(?:super::)*" + re.escape(stem) + r"::(" + "|".join(map(re.escape, names)) + r")\b(?!::)"), "crate::" + "::".join(_shared_path(rel, ()))))
    if renames:
        for f in _src_files([src_dir]):
            t = Path(f).read_text()
            nt = t
            for pat, target in renames:
                nt = pat.sub(lambda m, target=target: target + "::" + m.group(1), nt)
            if nt != t:
                Path(f).write_text(nt)
        if dest_src:
            # the same paths in the new crate go through the crate below by name
            for f in _src_files([dest_src]):
                t = Path(f).read_text()
                nt = t
                for pat, target in renames:
                    nt = pat.sub(lambda m, target=target: dest_root + target[len("crate"):] + "::" + m.group(1), nt)
                if nt != t:
                    Path(f).write_text(nt)



def _named_elsewhere(stem, rel, files, tfiles):
    """Whether another file of the crate mentions the module `stem` (an import of it, a glob of it)."""
    pat = re.compile(r"\b" + re.escape(stem) + r"\b")
    for other, (text, items) in list(files.items()) + list(tfiles.items()):
        if other == rel:
            continue
        for it in items:
            if it.kind == "use" and pat.search(it.text):
                return True
    return False


def apply_cut(args, nodes, files, tfiles, order):
    layer = order.index(args.layer) + 1
    # a `mod x;` follows its file (the facade declares the module above); it is not an item that moves
    moved = [n for n in nodes if n.eff == layer and n.fixed is None and (not n.test or n.kind == "method") and not (n.kind == "mod" and n.item.text.rstrip().endswith(";"))]
    mtests = [n for n in nodes if n.eff == layer and n.fixed is None and n.test and n.kind != "method"]
    lower = args.lower.replace("-", "_")
    facade_blocks, new_test_files, test_decls, test_removals, test_copies, pub_test_mods, test_shares, test_share_imports, test_imports = place_tests(nodes, mtests, files, tfiles, lower)
    crate_dir = os.path.join("crates", args.crate)
    new_src = os.path.join(crate_dir, "src")
    by_file = collections.defaultdict(list)
    for n in moved:
        by_file[n.file].append(n)
    # module files to write: moved files and their ancestors
    mod_files = (set(by_file) | set(facade_blocks) | {r for r in test_decls if r in files}) - {"lib.rs"}
    for rel in list(mod_files):
        parts = module_path(rel)
        for k in range(1, len(parts)):
            mod_files.add("/".join(parts[:k]) + ".rs")
    children = collections.defaultdict(set)
    for rel in mod_files:
        parts = module_path(rel)
        if len(parts) > 1:
            children["/".join(parts[:-1]) + ".rs"].add(parts[-1])
    removed = collections.defaultdict(list)
    existing_traits = set()
    for f in _src_files(sorted(glob_dirs("crates/*/src"))):
        existing_traits |= set(re.findall(r"(?m)^pub trait (\w+)", Path(f).read_text()))
    traits_out = []
    out_files = {}
    vacated = []
    top_mods = sorted({module_path(rel)[0] for rel in mod_files})
    stats = collections.Counter()
    for rel in sorted(mod_files):
        text, items = files[rel]
        mp = "::".join(module_path(rel))
        mv = by_file.get(rel, [])
        free = [n for n in mv if n.kind != "method"]
        meths = collections.defaultdict(list)
        for n in mv:
            if n.kind == "method":
                meths[id(n.owner)].append(n)
        spans = []
        pieces = []
        macros = []
        for n in sorted(free, key=lambda n: n.item.start):
            spans.append((n.item.start, n.item.end))
            (macros if n.kind == "macro_rules" else pieces).append(WIDEN.sub("pub", n.item.text))
        method_nodes = [n for ms in meths.values() for n in ms]
        for owner_id, ms in meths.items():
            owner = ms[0].owner
            all_children = [c for c in owner.children if not c.cfg_test]
            if len(ms) == len([c for c in owner.children]):
                spans.append((owner.start, owner.end))
            else:
                for n in ms:
                    spans.append((n.item.start, n.item.end))
        if method_nodes:
            stem = module_path(rel)[-1]
            trait = camel(args.layer if stem == "store" else stem) + ("StoreCore" if stem == "store" else "Store")
            if trait in existing_traits:  # a split file: the trait of the crate below keeps the plain name
                trait = camel(stem if stem != "store" else "core") + camel(args.layer) + "Store"
            pieces.append(make_trait(trait, sorted([n.item for n in method_nodes], key=lambda it: it.start)))
            stats["traits"] += 1
            traits_out.append((mp, trait))
        sibling_mods = {it.name for it in items if it.kind == "mod"}
        uses = []
        for it in items:
            if it.kind != "use":
                continue
            u = it.text
            first = re.search(r"\buse\s+([A-Za-z_]\w*)", u)
            if first and first.group(1) in sibling_mods and first.group(1) not in children.get(rel, ()):
                u = re.sub(r"\buse\s+" + first.group(1), "use %s::%s::%s" % (lower, mp, first.group(1)), u, count=1)
            uses.append(u)
        moved_ids = {id(n.item) for n in mv}
        remaining = [it for it in items if it.kind in FREE_KINDS and not it.cfg_test and it.is_pub and id(it) not in moved_ids and not any(x.startswith("#[cfg") for x in it.attrs)]
        own_children = set(children.get(rel, ()))
        names = sorted({it.name for it in remaining if it.name and it.name not in own_children})
        head = ["//! The part of `%s` that moved up from %s." % (mp, args.lower), "#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`"]
        head += macros  # textual scope: before the modules that use them
        head += uses
        # a file the move empties goes away below, and the module with it
        emptied = is_vacant(remove_spans(text, spans + test_removals.get(rel, []))) and rel not in {r for (r, _) in test_shares} and rel not in facade_blocks and not _named_elsewhere(module_path(rel)[-1], rel, files, tfiles)
        if emptied:
            vacated.append(rel)
        else:
            head.append("pub use %s::%s::*;" % (lower, mp))
            for k in range(0, len(names), 12):
                head.append("pub use %s::%s::{%s};" % (lower, mp, ", ".join(names[k : k + 12])))
        for ch in sorted(children.get(rel, ())):
            # the declaration of a child keeps its attributes (`#[path]`, `#[cfg]`) where the file is found by them
            decl = next((it for it in items if it.kind == "mod" and it.name == ch and it.text.rstrip().endswith(";")), None)
            head.append("".join(a + "\n" for a in decl.attrs if a.lstrip().startswith(("#[path", "#[cfg"))) + "pub mod %s;" % ch if decl else "pub mod %s;" % ch)
        for d in test_decls.get(rel, ()):
            head.append(d)
        if rel in facade_blocks:
            pieces.append(facade_blocks[rel])
        body = "\n\n".join(pieces)
        out_files[rel] = "\n".join(head) + "\n\n" + body + ("\n" if body else "")
        removed[rel] = spans + test_removals.get(rel, [])
        stats["items"] += len(mv)
    # a file that only gives tests or shared helpers away still loses them
    for rel, spans in test_removals.items():
        if rel in files and rel not in mod_files:
            removed[rel] = list(removed.get(rel, [])) + list(spans)
    # test files the move empties are deleted, with the declaration that pulled them in
    pending_test_writes = {}
    deleted_test_files = []
    vacant_tests = set()
    for rel, spans in test_removals.items():
        if rel in files:
            continue
        text, items = tfiles[rel]
        if is_vacant(remove_spans(text, spans)) and rel not in {r for (r, _) in test_shares} and not _named_elsewhere(module_path(rel)[-1], rel, files, tfiles):
            vacant_tests.add(rel)
    extra_spans = collections.defaultdict(list)
    for rel in sorted(vacant_tests):
        deleted_test_files.append(rel)
        parts = module_path(rel)
        parent = "/".join(parts[:-1]) + ".rs"
        for pool, sink in ((files, removed), (tfiles, extra_spans)):
            if parent in pool:
                for it in pool[parent][1]:
                    if it.kind == "mod" and it.name == parts[-1] and it.text.rstrip().endswith(";"):
                        sink[parent].append((it.start, it.end))
                break
    for rel in set(test_removals) | set(extra_spans):
        if rel in files or rel in vacant_tests or rel not in tfiles:
            continue
        pending_test_writes[rel] = remove_spans(tfiles[rel][0], test_removals.get(rel, []) + extra_spans.get(rel, []))
    for rel in pub_test_mods:
        if rel in new_test_files and rel not in deleted_test_files:
            new_test_files[rel] = new_test_files[rel].replace("#![allow(unused_imports)]\n", "#![allow(unused_imports)]\npub use %s::%s::*;\n" % (lower, "::".join(module_path(rel))), 1)
    # write
    for rel, content in list(out_files.items()) + list(new_test_files.items()):
        path = os.path.join(new_src, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        open(path, "w").write(content)
    for rel in deleted_test_files:
        os.remove(os.path.join(args.src_dir, rel))
    for rel in vacated:
        parts = module_path(rel)
        parent = ("/".join(parts[:-1]) + ".rs") if len(parts) > 1 else None
        for it in (files[parent][1] if parent in files else ()):
            if it.kind == "mod" and it.name == parts[-1] and it.text.rstrip().endswith(";"):
                removed[parent].append((it.start, it.end))
    for rel, out in pending_test_writes.items():
        open(os.path.join(args.src_dir, rel), "w").write(out)
    for rel in list(by_file):
        parts = module_path(rel)
        parent = ("/".join(parts[:-1]) + ".rs") if len(parts) > 1 else None
        if parent and os.path.exists(os.path.join(args.src_dir, parent)):
            pt = open(os.path.join(args.src_dir, parent)).read()
            nt = re.sub(r"(?m)^(\s*)mod " + re.escape(parts[-1]) + r";", r"\1pub mod " + parts[-1] + ";", pt)
            if nt != pt:
                open(os.path.join(args.src_dir, parent), "w").write(nt)
    pairs = set()
    for n in moved:
        if n.name and n.kind != "method":
            pairs.add((module_path(n.file)[-1], n.name))
    # what the moved code uses and stays below must be reachable from above: make it pub
    moved_ids = {n.id for n in moved}
    widen = collections.defaultdict(dict)
    macro_exports = collections.defaultdict(dict)  # lower file -> {start: item}
    macro_imports = collections.defaultdict(set)  # destination file -> macro names
    for n in moved + mtests + test_copies + [u for ns in test_shares.values() for u in ns]:
        for r in n.refs:
            t = nodes[r]
            if t.kind == "macro_rules" and r not in moved_ids and t.fixed is None and not t.test:
                macro_exports[t.file][t.item.start] = t.item
                macro_imports[n.file].add(t.name)
                continue
            if r in moved_ids or t.test or t.fixed is not None or t.kind not in FREE_KINDS + ("method", "macro_call") or t.kind == "macro_rules":
                continue
            widen[t.file][t.item.start] = t.item
    # the types in the signature of what is now public must be public too
    queue = [it for d in widen.values() for it in d.values()]
    by_item = {id(n.item): n for n in nodes}
    seen = set(id(i) for i in queue)
    while queue:
        it = queue.pop()
        node = by_item.get(id(it))
        if node is None:
            continue
        for r in node.refs:
            t = nodes[r]
            if r in moved_ids or t.fixed is not None or t.kind not in ("struct", "enum", "union", "type", "trait") or id(t.item) in seen:
                continue
            seen.add(id(t.item))
            widen[t.file][t.item.start] = t.item
            queue.append(t.item)
    widened = 0
    for rel in vacated:
        removed.pop(rel, None)
        widen.pop(rel, None)
        macro_exports.pop(rel, None)
        if os.path.exists(os.path.join(args.src_dir, rel)):
            os.remove(os.path.join(args.src_dir, rel))
    for rel in set(removed) | set(widen) | set(macro_exports):
        text, items = files[rel]
        edits = []
        for a, b in removed.get(rel, ()):
            edits.append((a, b, ""))
        for start, it in macro_exports.get(rel, {}).items():
            if "#[macro_export]" not in it.text:
                code = ri._leading_meta(text, it.start, len(text))[1]
                edits.append((code, code, "#[macro_export]\n"))
                widened += 1
        for start, it in widen.get(rel, {}).items():
            if it.kind == "macro_call":
                new = re.sub(r"(?<!pub )\bstatic\s", "pub static ", it.text)
                if new != it.text:
                    edits.append((it.start, it.end, new))
                    widened += 1
                continue
            code = ri._leading_meta(text, it.start, len(text))[1]
            m = ri._VIS.match(text, code)
            if m is None:
                pos = _vis_position(text, it.start, code)
                edits.append((pos, pos, "pub "))
                widened += 1
            elif not m.group(0).startswith("pub ") and not m.group(0).rstrip() == "pub":
                edits.append((code, m.end(), "pub "))
                widened += 1
        if not edits:
            continue
        # removal spans drop their trailing blank lines like remove_spans does; insertions are plain
        ins = [e for e in edits if e[0] == e[1] or e[2]]
        rem = [(a, b) for a, b, r in edits if not r and a != b]
        for a, b, r in sorted(ins, reverse=True):
            text = text[:a] + r + text[b:]
        # insertions shift later offsets only for spans after them; apply removals on the original offsets
        # by redoing both from the original text in one descending pass
        text0 = files[rel][0]
        allx = sorted(edits, key=lambda e: (e[0], e[1]), reverse=True)
        out = text0
        for a, b, r in allx:
            if not r and a != b:
                out = remove_spans(out, [(a, b)])
            else:
                out = out[:a] + r + out[b:]
        open(os.path.join(args.src_dir, rel), "w").write(out)
    print("widened %d items below" % widened)
    # the test files that stay lose their imports of the test items that moved up
    tpairs = set(pairs)
    for n in mtests:
        if n.name:
            tpairs.add((module_path(n.file)[-1], n.name))
    for ns in test_shares.values():
        for u in ns:
            tpairs.discard((module_path(u.file)[-1], u.name))
    for u in test_copies:
        tpairs.discard((module_path(u.file)[-1], u.name))
    for rel, (text0, items0) in files.items():
        path = os.path.join(args.src_dir, rel)
        if rel in vacated:
            continue
        cur = open(path).read()
        base = cur
        # an earlier step may have rewritten the file (a test moved out of it): parse what is there now
        if rel in removed and removed[rel] or cur != text0:
            items_now = ri.parse(cur)
        else:
            items_now = items0
        new = prune_uses(cur, items_now, tpairs)
        if new != cur:
            open(path, "w").write(new)
    for rel in tfiles:
        path = os.path.join(args.src_dir, rel)
        if not os.path.exists(path) or rel in deleted_test_files:
            continue
        cur = open(path).read()
        new = prune_uses(cur, ri.parse(cur), tpairs)
        if new != cur:
            open(path, "w").write(new)
    includers = {}
    for n in nodes:
        if n.test and n.fixed is None and n.kind == "macro_call":
            for m in re.finditer(r'include!\("([^"]+)"\)', n.item.text):
                includers[os.path.normpath(os.path.join(os.path.dirname(n.file), m.group(1)))] = (n.file, n.container)
    apply_shares(test_shares, test_share_imports, args.src_dir, {**files, **tfiles}, pairs, _dev_dependencies(lower), set(tfiles), includers, new_src, lower)
    for rel, names in macro_imports.items():
        path = os.path.join(new_src, rel)
        if not os.path.exists(path):
            continue
        line = "\n".join("use crate::%s;" % nm for nm in sorted(names)) + "\n"
        t = Path(path).read_text()
        lines_ = t.split("\n")
        k = 0
        while k < len(lines_) and (lines_[k].startswith("//!") or lines_[k].startswith("#![")):
            k += 1
        t = "\n".join(lines_[:k] + [line.rstrip("\n")] + lines_[k:])
        open(path, "w").write(t)
    lib = ['//! %s: the %s domain of the store, cut out of %s at function level. Everything of %s is re-exported at the root;' % (args.crate, args.layer, args.lower, args.lower),
           "//! the modules below shadow the lower crate's modules of the same name and re-export them, so every old path still resolves.",
           "",
           "// Test helpers widened for st3-daemon's tests are unused when only the feature is on.",
           '#![cfg_attr(all(feature = "test-support", not(test)), allow(dead_code))]',
           "",
           '#[cfg(any(test, feature = "test-support"))]',
           "#[macro_use]",
           '#[path = "../../../scripts/ci-test-paths.rs"]',
           "#[allow(dead_code, unused_macros)]",
           "mod ci_test_paths;",
           "",
           "pub use %s::*;" % lower, ""]
    lib += ["pub mod %s;" % m for m in top_mods]
    lib += test_decls.get("lib.rs", [])
    lib += [""] + ["pub use %s::%s;" % (mp, t) for mp, t in traits_out]
    os.makedirs(new_src, exist_ok=True)
    open(os.path.join(new_src, "lib.rs"), "w").write("\n".join(lib) + "\n")
    if os.path.exists("crates/st3-daemon/build.rs"):
        open(os.path.join(crate_dir, "build.rs"), "w").write(open("crates/st3-daemon/build.rs").read())
    print("moved %d items into %s (%d files, %d traits)" % (stats["items"], args.crate, len(out_files), stats["traits"]))
    args.traits = [t for _, t in traits_out]
    return top_mods



def wire_crate(args, top_mods):
    """Manifest, workspace, flake, nextest, module-layers and daemon wiring for the new crate; the
    crate above it (--upper) now sits on the new crate instead of on --lower."""
    import tomllib

    crate, lower, upper = args.crate, args.lower, args.upper
    ident = crate.replace("-", "_")
    root = os.path.join("crates", crate)
    daemon_text = open("crates/st3-daemon/Cargo.toml").read()
    files = []
    for dp, _, fs in os.walk(os.path.join(root, "src")):
        files += [os.path.join(dp, f) for f in fs if f.endswith(".rs")]
    text = "\n".join(open(f).read() for f in files)

    def uses(name):
        n = name.replace("-", "_")
        return re.search(r"\b" + n + r"::|\buse\s+" + n + r"\b|\b" + n + r"!", text) is not None

    section = daemon_text.split("[dev-dependencies]")[0].split("[dependencies]")[1]
    dev_section = daemon_text.split("[dev-dependencies]")[1]
    lines = {m.group(1): ln for ln in section.split("\n") if (m := re.match(r"^([A-Za-z0-9_-]+)\s*=", ln))}
    devlines = {m.group(1): ln for ln in dev_section.split("\n") if (m := re.match(r"^([A-Za-z0-9_-]+)\s*=", ln))}
    skip = {"st3-github", "st3-peer", "st3-reconcile", "st3-api", lower}
    out = ["[package]", f'name = "{crate}"', 'version = "0.1.0"', 'edition = "2024"',
           f'description = "{args.description}"', 'license = "MIT"', "", "[features]",
           "# Forwarded by st3-daemon's feature of the same name; never enabled by a production build.",
           f'test-support = ["{lower}/test-support"]', "", "[dependencies]", f'{lower} = {{ path = "../{lower}" }}']
    out += [ln for k, ln in lines.items() if k not in skip and uses(k)]
    internal = sorted({m.replace("_", "-") for m in re.findall(r"\b(st3_[a-z0-9_]+)::", text)} - skip - {crate} - set(lines))
    out += [f'{n} = {{ path = "../{n}" }}' for n in internal if os.path.isdir(f"crates/{n}") and n not in {l.split(" ")[0] for l in out}]
    out += ["", "[dev-dependencies]", f'{ident}_self = {{ package = "{crate}", path = ".", features = ["test-support"] }}']
    out += [ln for k, ln in devlines.items() if k != "st3_daemon_self" and uses(k)]
    out.append("")
    open(os.path.join(root, "Cargo.toml"), "w").write("\n".join(out))

    def sub(path, a, b, count=1):
        t = open(path).read()
        if a not in t:
            print("WARNING: %s: %r not found" % (path, a[:50]))
            return
        open(path, "w").write(t.replace(a, b, count))

    sub("Cargo.toml", f'  "crates/{lower}",\n', f'  "crates/{lower}",\n  "crates/{crate}",\n')
    sub("flake.nix", f'          "--exclude"\n          "{lower}"\n', f'          "--exclude"\n          "{lower}"\n          "--exclude"\n          "{crate}"\n')
    sub("flake.nix", f'            "-p"\n            "{lower}"\n', f'            "-p"\n            "{lower}"\n            "-p"\n            "{crate}"\n')
    sub("scripts/module_layers.py", f'"crates/{lower}/src", ', f'"crates/{lower}/src", "crates/{crate}/src", ')
    nx = open(".config/nextest.toml").read()
    nx = re.sub(r"package\((=?)st3-daemon\)", lambda m: m.group(0) + f" or package({m.group(1)}{crate})" if f"package({m.group(1)}{crate})" not in nx else m.group(0), nx)
    open(".config/nextest.toml", "w").write(nx)
    dc = open("crates/st3-daemon/Cargo.toml").read()
    dc = re.sub(r"(?m)^(" + re.escape(lower) + r' = \{ path = "\.\./' + re.escape(lower) + r'" \}\n)', lambda m: m.group(1) + f'{crate} = {{ path = "../{crate}" }}\n', dc, count=1)
    dc = dc.replace(f'"{lower}/test-support"', f'"{lower}/test-support", "{crate}/test-support"', 1)
    open("crates/st3-daemon/Cargo.toml", "w").write(dc)
    if upper:
        up = f"crates/{upper}/Cargo.toml"
        t = open(up).read()
        t = t.replace(f'{lower} = {{ path = "../{lower}" }}', f'{crate} = {{ path = "../{crate}" }}', 1)
        t = t.replace(f'"{lower}/test-support"', f'"{crate}/test-support"', 1)
        open(up, "w").write(t)
        for f in _src_files([f"crates/{upper}/src"]):
            t = Path(f).read_text()
            nt = re.sub(r"\b" + re.escape(lower.replace("-", "_")) + r"::", ident + "::", t)
            if nt != t:
                Path(f).write_text(nt)
        ul = f"crates/{upper}/src/lib.rs"
        t = open(ul).read()
        t = t.replace(f"pub use {lower.replace('-', '_')}::*;", f"pub use {ident}::*;", 1)
        open(ul, "w").write(t)
    dl = open("crates/st3-daemon/src/lib.rs").read()
    # grouped re-exports (`pub(crate) use st3_store::{checkout, external_sessions};`) follow the modules too
    for it in ri.parse(dl):
        if it.kind != "use":
            continue
        parsed = use_paths(it.text)
        if not parsed:
            continue
        attrs, vis, paths = parsed
        changed = False
        new_paths = []
        for segs, alias in paths:
            if len(segs) >= 2 and segs[0] == lower.replace("-", "_") and segs[1] in top_mods:
                segs = [ident] + segs[1:]
                changed = True
            new_paths.append((segs, alias))
        if changed:
            dl = dl.replace(it.text, render_use(attrs, vis, new_paths).join(["\n" if it.text.startswith("\n") else "", ""]), 1)
    for m in top_mods:
        dl = re.sub(r"(?m)^pub use " + re.escape(lower.replace("-", "_")) + r"::" + re.escape(m) + r";$", f"pub use {ident}::{m};", dl)
    if getattr(args, "traits", None):
        dl = dl.replace("pub use st3_github::{GitChangesStore", "pub use %s::{%s};\npub use st3_github::{GitChangesStore" % (ident, ", ".join(sorted(args.traits))), 1)
    open("crates/st3-daemon/src/lib.rs", "w").write(dl)
    # code that named the crate below directly (`st3_store::store::..`) now goes through the daemon's re-export
    pat = re.compile(r"\b" + re.escape(lower.replace("-", "_")) + r"::(" + "|".join(map(re.escape, top_mods)) + r")\b")
    for f in _src_files(["crates/st3-daemon/src"]):
        if f.endswith("/lib.rs") and f.count("/") == 3:
            continue
        t = Path(f).read_text()
        nt = pat.sub(r"crate::\1", t)
        if nt != t:
            Path(f).write_text(nt)
    print("wired", crate)



def glob_dirs(pattern):
    import glob

    return glob.glob(pattern)


def _cargo_json(extra):
    cmd = ["cargo", "check", "--workspace", "--keep-going", "--message-format=json"] + extra
    out = subprocess.run(cmd, capture_output=True, text=True).stdout
    diags = []
    for line in out.split("\n"):
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") == "compiler-message" and m["message"]["level"] == "error":
            diags.append(m["message"])
    return diags


def _src_files(roots):
    out = []
    for r in roots:
        for dp, _, fs in os.walk(r):
            out += [os.path.join(dp, f) for f in fs if f.endswith(".rs")]
    return out


def _in_trait_block(lines, idx):
    """Whether line idx sits inside an `impl Trait for X` or a `trait` block (no `pub` there)."""
    for j in range(idx, -1, -1):
        t = lines[j]
        if t.startswith("}"):
            return False
        if re.match(r"(?:unsafe\s+)?impl\b", t) or re.match(r"(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\b", t):
            return bool(re.match(r"(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\b", t)) or bool(re.search(r"\bfor\b", t))
    return False


def _pub_line(path, line_no, name=None):
    if not os.path.exists(path):
        return 0
    lines = Path(path).read_text().split("\n")
    if name and re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?thread_local!", lines[line_no - 1]):
        # the statics a thread_local! declares are private until the static itself says otherwise
        t = "\n".join(lines)
        at = sum(len(l) + 1 for l in lines[: line_no - 1])
        end = ri._match_delim(t, t.index("{", at)) if "{" in t[at:] else at
        pat = re.compile(r"(?m)^(\s*)(?:pub(?:\([^)]*\))?\s+)?static(\s+" + re.escape(name) + r"\b)")
        body = t[at:end]
        nb = pat.sub(r"\1pub static\2", body, count=1)
        if nb != body:
            Path(path).write_text(t[:at] + nb + t[end:])
            return 1
        return 0
    if _in_trait_block(lines, line_no - 1):
        return 0
    ln = lines[line_no - 1]
    new = re.sub(r"^(\s*)(?!pub\b)((?:/\*.*?\*/\s*)*(?:async\s+|const\s+|unsafe\s+)*(?:fn|const|static|struct|enum|type|trait|mod|use)\b)", r"\1pub \2", ln)
    new = re.sub(r"^(\s*)pub\((?:crate|super|self|in [\w:]+)\)\s+", r"\1pub ", new)
    if re.match(r"\s*pub use .*::\*;", new) and not re.match(r"\s*pub use .*::\*;", ln):
        return 0  # a glob import that is private stays private: making it pub re-exports everything it reaches
    if new != ln:
        lines[line_no - 1] = new
        Path(path).write_text("\n".join(lines))
        return 1
    return 0


def _make_fields_pub(roots, struct, fields):
    for f in _src_files(roots):
        t = Path(f).read_text()
        mm = re.search(r"(?m)^(\s*)(?:pub(?:\([^)]*\))?\s+)?struct " + re.escape(struct) + r"\b[^{;]*\{", t)
        if not mm:
            continue
        end = ri._match_delim(t, mm.end() - 1)
        body = t[mm.end() : end - 1]
        nb = body
        for fld in fields:
            nb = re.sub(r"(?m)^(\s*)(?:pub(?:\([^)]*\))?\s+)?" + re.escape(fld) + r"\s*:", r"\1pub " + fld + ":", nb, count=1)
        if nb != body:
            Path(f).write_text(t[: mm.end()] + nb + t[end - 1 :])
            return 1
    return 0


def _daemon_root(crate_name):
    """How a crate outside the chain names the daemon library (`st3`, the alias most use, or `st3_daemon`),
    and whether it only has it for tests."""
    manifest = Path("crates") / crate_name / "Cargo.toml"
    text = manifest.read_text() if manifest.exists() else ""
    for pat, name in ((r"(?m)^st3\s*=", "st3"), (r"(?m)^st3[-_]daemon\s*=", "st3_daemon")):
        m = re.search(pat, text)
        if m:
            dev = text.find("[dev-dependencies]")
            return name, dev != -1 and m.start() > dev
    return "st3", False


def _drop_spans(dropped):
    """Remove the imports at these (byte) spans from their `use` items."""
    edits = 0
    for f, spans in dropped.items():
        raw = Path(f).read_bytes()
        t = raw.decode()
        cspans = [(len(raw[:a].decode()), len(raw[:b].decode())) for a, b in spans]
        for it in sorted(ri.parse(t), key=lambda i: -i.start):
            if it.kind != "use":
                continue
            hit = [(a, b) for a, b in cspans if it.start <= a and b <= it.end]
            parsed = use_paths(it.text) if hit else None
            if not parsed:
                continue
            attrs, vis, paths = parsed
            gone = {t[a:b].split("::")[-1].split(" as ")[-1].strip() for a, b in hit}
            whole = {t[a:b].strip() for a, b in hit}
            keep = [(segs, alias) for segs, alias in paths if (alias or segs[-1]) not in gone and "::".join(segs) not in whole]
            code = _attr_start(t, it)
            t = t[:code] + (render_use(attrs, vis, keep) if keep else "") + t[it.end :]
            edits += 1
        Path(f).write_text(t)
    return edits


def fix_once(diags, roots, traits):
    # a shared test module re-exported a name whose item has moved up: it is not this crate's to export
    dropped = collections.defaultdict(set)
    for d in diags:
        prim = [sp for sp in d["spans"] if sp["is_primary"]]
        if (d.get("code") or {}).get("code") == "E0432" and prim and "shared" in prim[0]["file_name"]:
            for sp in prim:
                dropped[prim[0]["file_name"]].add((sp["byte_start"], sp["byte_end"]))
    if dropped:
        return _drop_spans(dropped)
    edits = 0
    names = set()
    imports = collections.defaultdict(set)
    ambiguities = collections.defaultdict(set)
    for d in diags:
        msg = d["message"]
        code = (d.get("code") or {}).get("code")
        prim = [s for s in d["spans"] if s["is_primary"]]
        m = re.match(r"fields? ((?:`\w+`(?:, | and |, and )?)+) of struct `([\w:]+)` (?:is|are) private", msg)
        if m:
            edits += _make_fields_pub(roots, m.group(2).split("::")[-1], re.findall(r"`(\w+)`", m.group(1)))
            continue
        if code == "E0599" or "no method named" in msg or "no function or associated item" in msg:
            for c in d["children"]:
                if "not in scope" in c["message"] or "perhaps you want to import" in c["message"]:
                    for sp in c["spans"]:
                        rep = sp.get("suggested_replacement") or ""
                        for um in re.finditer(r"use ([\w:]+);", rep):
                            if um.group(1).split("::")[-1] in traits and prim:
                                imports[prim[0]["file_name"]].add(um.group(1))
            continue
        if code == "ambiguous_glob_imports" or msg.endswith("is ambiguous"):
            nm = re.match(r"`(\w+)` is ambiguous", msg)
            if nm and prim:
                ambiguities[prim[0]["file_name"]].add(nm.group(1))
                # the globs that clash are in the files the notes point at
                for c in d["children"]:
                    if "could refer to" in c["message"] or "could also refer to" in c["message"]:
                        for sp in c["spans"]:
                            ambiguities[sp["file_name"]].add(nm.group(1))
            continue
        ms = re.match(r"unresolved imports? `((?:super::)+)(\w+)", msg)
        if code == "E0432" and ms and prim:
            edits += _absolute_super_import(prim[0], len(ms.group(1)) // 7, ms.group(2))
            continue
        mi = re.match(r"module import `(\w+)` is private", msg)
        if code == "E0603" and mi:
            # a glob import hides the lower module that the facade's `pub use lower::path::*` also reaches: name it
            for c in d["children"]:
                for sp in c["spans"]:
                    ambiguities[sp["file_name"]].add(mi.group(1))
            continue
        if code in ("E0603", "E0624"):
            pn = re.match(r"\w+ `(\w+)` is private", msg)
            pname = pn.group(1) if pn else None
            for c in d["children"]:
                for sp in c["spans"]:
                    if "defined here" in c["message"] or "defined here" in (sp.get("label") or "") or "private" in (sp.get("label") or ""):
                        edits += _pub_line(sp["file_name"], sp["line_start"], pname)
            for sp in d["spans"]:
                if not sp["is_primary"] and "defined here" in (sp.get("label") or ""):
                    edits += _pub_line(sp["file_name"], sp["line_start"], pname)
            continue
        nm = re.match(r"cannot find (?:function|value|type|struct, variant or union type|macro) `(\w+)`", msg)
        if nm:
            names.add(nm.group(1))
        um = re.match(r"unresolved imports? (.*)", msg)
        if um:
            for path in re.findall(r"`([\w:]+)`", um.group(1)):
                names.add(path.split("::")[-1])
    for nm in names:
        pat = re.compile(r"(?m)^(\s*)(?!pub)((?:async\s+|const\s+|unsafe\s+)*(?:fn|const|static|struct|enum|type|trait)\s+" + re.escape(nm) + r"\b)")
        for f in _src_files(roots):
            t = Path(f).read_text()
            lines = t.split("\n")
            for m in pat.finditer(t):
                if _in_trait_block(lines, t.count("\n", 0, m.start())):
                    continue
                head = t[: m.start()]
                # an item that only exists for `cfg(test)` is needed from another crate's tests: share it
                head = re.sub(r"#\[cfg\(test\)\]\n(\s*)$", '#[cfg(any(test, feature = "test-support"))]\\n\\1', head)
                nt = head + m.group(1) + "pub " + m.group(2) + t[m.end() :]
                Path(f).write_text(nt)
                edits += 1
                break
    for f, uses in imports.items():
        t = Path(f).read_text()
        add = []
        crate_name = f.split("/")[1] if f.startswith("crates/") else ""
        chain = {"st3-store", "st3-github", "st3-peer", "st3-reconcile", "st3-api", "st3-daemon"} | {r.split("/")[1] for r in roots if r.startswith("crates/")}
        root, dev_only = ("crate", False) if crate_name in chain else _daemon_root(crate_name)
        for leaf in sorted({u.split("::")[-1] for u in uses}):
            if re.search(r"\buse [^;]*\b" + leaf + r"\b", t):
                continue
            add.append(("#[cfg(test)]\n" if dev_only else "") + "use %s::%s;" % (root, leaf))
        if add:
            m = re.search(r"(?m)^use ", t)
            at = m.start() if m else (t.index("\n", t.index("//!")) + 1 if t.startswith("//!") else 0)
            Path(f).write_text(t[:at] + "\n".join(add) + "\n" + t[at:])
            edits += len(add)
    for f, nms in ambiguities.items():
        t = Path(f).read_text()
        m = re.search(r"(?m)^pub use (\w+)::([\w:]+)::\*;$", t)
        if not m:
            continue
        defined = set()
        lower_file = Path("crates") / m.group(1).replace("_", "-") / "src" / (m.group(2).replace("::", "/") + ".rs")
        if lower_file.exists():
            defined = {it.name for it in ri.parse(lower_file.read_text()) if it.name}
        lines = "\n".join("pub use %s::%s::%s;" % (m.group(1), m.group(2), n) for n in sorted(nms) if ("::%s;" % n) not in t and n in defined)
        if lines:
            t = t[: m.end()] + "\n" + lines + t[m.end() :]
            Path(f).write_text(t)
            edits += 1
        # a name the file's module does not define but the crate below does: import that one explicitly
        explicit = []
        for n in sorted(nms):
            if n in defined or re.search(r"(?m)^use [\w:]*::" + re.escape(n) + r";", t):
                continue
            found = _defined_below(m.group(1), n)
            if found:
                explicit.append("use %s;" % found)
        if explicit:
            at = m.end() + 1 if lines else m.end()
            t = Path(f).read_text()
            Path(f).write_text(t[:at] + "\n" + "\n".join(explicit) + t[at:])
            edits += 1
    return edits


def _absolute_super_import(span, supers, first):
    """`use super::super::operator_error::..` in a file that moved up, where `operator_error` stayed below: name it
    through the crate below and make its declaration pub."""
    f = span["file_name"]
    m = re.match(r"crates/[^/]+/src/(.+)", f)
    if not m:
        return 0
    base = module_path(m.group(1))
    base = base[: len(base) - supers]
    raw = Path(f).read_bytes()
    start = span["byte_start"] - 7 * supers  # the span is the unresolved segment; the `super::`s precede it
    if raw[start : span["byte_start"]].decode() != "super::" * supers:
        return 0
    tail = raw[span["byte_start"] : span["byte_end"]].decode()
    lower = "st3_store"
    decl_file = Path("crates/st3-store/src") / ("/".join(base) + ".rs")
    if not decl_file.exists():
        return 0
    dt = decl_file.read_text()
    nd = re.sub(r"(?m)^(\s*)(?:pub\([^)]*\)\s+)?mod " + re.escape(first) + r";", r"\1pub mod " + first + ";", dt)
    if nd == dt and not re.search(r"(?m)^\s*pub mod " + re.escape(first) + ";", dt):
        return 0
    decl_file.write_text(nd)
    new = "::".join([lower] + base) + "::" + tail
    Path(f).write_bytes(raw[:start] + new.encode() + raw[span["byte_end"] :])
    return 1


def _defined_below(crate_ident, name):
    """`crate::module::path::name` when exactly one pub item of the crate below is called `name`."""
    root = Path("crates") / crate_ident.replace("_", "-") / "src"
    hits = []
    for rel in module_files(str(root)):
        for it in ri.parse((root / rel).read_text()):
            if it.name == name and it.is_pub and it.kind in ("fn", "const", "static", "struct", "enum", "type", "trait"):
                hits.append("::".join([crate_ident] + module_path(rel) + [name]))
    return hits[0] if len(hits) == 1 else None


def cmd_fix(args):
    roots = args.roots.split(",")
    traits = set()
    for f in _src_files(roots):
        traits |= set(re.findall(r"(?m)^pub trait (\w+)", Path(f).read_text()))
    extra = ["--all-targets"] if args.all_targets else []
    previous = None
    for rnd in range(1, args.rounds + 1):
        diags = _cargo_json(extra)
        print("round %d: %d errors" % (rnd, len(diags)))
        if not diags:
            return 0
        signature = sorted(d["rendered"] for d in diags)
        big = [f for f in _src_files(roots) if os.path.getsize(f) > 2_000_000]
        if signature == previous or big:
            for d in diags[:8]:
                print("  unfixed:", d["rendered"].split("\n")[0][:140])
            if big:
                print("  a file grew past 2 MB:", big[:3])
            return 1
        previous = signature
        if fix_once(diags, roots, traits) == 0:
            for d in diags[:8]:
                print("  unfixed:", d["rendered"].split("\n")[0][:140])
            return 1
    return 1


def cmd_sweep(args):
    """Remove the items the cut left unused in the lower crates (`dead_code` warnings): a test helper
    that was copied next to the tests that moved, or a function only a moved test called."""
    roots = [r for r in args.roots.split(",") if r]
    removed = 0
    for rnd in range(args.rounds):
        out = subprocess.run(["cargo", "check", "--workspace", "--all-targets", "--keep-going", "--message-format=json"], capture_output=True, text=True).stdout
        if any('"level":"error"' in l for l in out.split("\n") if l.startswith("{")):
            print("sweep: the workspace does not build; fix it first")
            return 1
        spans = collections.defaultdict(set)
        for line in out.split("\n"):
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if m.get("reason") != "compiler-message":
                continue
            d = m["message"]
            if (d.get("code") or {}).get("code") != "dead_code" or "never used" not in d["message"]:
                continue
            for sp in d["spans"]:
                if sp["is_primary"] and any(sp["file_name"].startswith(r) for r in roots):
                    spans[sp["file_name"]].add((sp["byte_start"], sp["byte_end"]))
        if not spans:
            break
        for f, ss in spans.items():
            text = Path(f).read_bytes().decode()
            # byte offsets -> the smallest item containing them
            raw = Path(f).read_bytes()
            items = []

            def walk(its):
                for it in its:
                    items.append(it)
                    if it.kind == "impl":
                        items.extend(it.children)
                    if it.kind == "mod" and it.body is not None:
                        walk(ri.parse_items(text, it.body[0], it.body[1]))

            walk(ri.parse(text))
            cut = []
            for b0, b1 in ss:
                c0, c1 = len(raw[:b0].decode()), len(raw[:b1].decode())
                best = None
                for it in items:
                    if it.start <= c0 and c1 <= it.end and it.kind in ("fn", "const", "static", "struct", "enum", "type"):
                        if best is None or (it.end - it.start) < (best.end - best.start):
                            best = it
                if best is not None:
                    cut.append((best.start, best.end))
            if cut:
                Path(f).write_text(remove_spans(text, sorted(set(cut))))
                removed += len(set(cut))
    print("swept %d items" % removed)
    return 0



def _manifest_text(m):
    out = ["{"]
    keys = list(m)
    for i, k in enumerate(keys):
        v = m[k]
        comma = "," if i < len(keys) - 1 else ""
        if isinstance(v, list):
            if not v:
                out.append(f"  {json.dumps(k)}: []{comma}")
                continue
            out.append(f"  {json.dumps(k)}: [")
            for j, e in enumerate(v):
                out.append("    " + json.dumps(e, ensure_ascii=False) + ("," if j < len(v) - 1 else ""))
            out.append("  ]" + comma)
        else:
            out.append(f"  {json.dumps(k)}: {json.dumps(v, ensure_ascii=False)}{comma}")
    out.append("}")
    return "\n".join(out) + "\n"


def _reanchor_inventory(slv, root, manifest):
    """Re-derive the site records from the sources. A record is fully derived from its site; what is not derivable
    is whether the site was accepted, which its adjacent marker says (by an id the inventory knows, with the same
    fingerprint). When identical sites are split between files, the occurrence counter in their ids shifts, so a
    marker can name an id the site no longer has: the marker and the record follow the site. Sites in the shared
    test modules a cut generates come from `cfg(test)` code the scanner skipped; they are accepted as they come."""
    import bisect

    cache = {}
    sites, markers = slv.scan(root, cache)
    contents = {p: v[0] for p, v in cache.items()}
    old = {e["id"]: e for e in manifest["sites"]}
    by_path = collections.defaultdict(list)
    for m in markers:
        by_path[m[0]].append(m)
    ends = {p: [m[2] for m in ms] for p, ms in by_path.items()}
    edits = collections.defaultdict(list)
    used = set()
    records = []
    for site in sites:
        idx = bisect.bisect_right(ends.get(site.path, []), site.start) - 1
        marker = by_path[site.path][idx] if idx >= 0 else None
        if marker is not None and (marker[3] is None or contents[site.path][marker[2] : site.start].strip()):
            marker = None
        known = marker is not None and marker[3]["id"] in old and old[marker[3]["id"]]["fingerprint"] == site.fingerprint
        generated = "_shared" in site.path.rsplit("/", 1)[-1]
        if not known and not generated:
            records.append(None)
            continue
        text = "/* sqlite-large-value: allow-for-now %s owner=%s TODO=%s */" % (site.id, site.owner, site.todo)
        if marker is not None:
            used.add((marker[0], marker[1]))
            if contents[site.path][marker[1] : marker[2]] != text:
                edits[site.path].append((marker[1], marker[2], text))
        else:
            edits[site.path].append((site.start, site.start, text + " "))
        records.append(slv.record(site))
    for path, start, end, marker in markers:
        if marker is not None and (path, start) not in used:
            owner_site = any(s.path == path and bisect.bisect_right(ends[path], s.start) - 1 >= 0 and by_path[path][bisect.bisect_right(ends[path], s.start) - 1][1] == start for s in sites)
            if not owner_site:
                edits[path].append((start, end + (1 if contents[path][end : end + 1] == " " else 0), ""))
    for path, es in edits.items():
        text = Path(path).read_text()
        for a, b, rep in sorted(es, reverse=True):
            text = text[:a] + rep + text[b:]
        Path(path).write_text(text)
    cache = {}
    sites, _ = slv.scan(root, cache)
    accepted = {r["id"] for r in records if r}
    manifest["sites"] = sorted((slv.record(x) for x in sites if x.id in accepted), key=lambda e: (e["path"], e["id"]))


def cmd_inventory(args):
    """Carry the SQLite large-value inventory (docs/st3/large-values.md) over a code move.

    A site's id and owner come from its path, so moving code changes them; the adjacent marker comment still
    names the old id, which says which record the site is. This rewrites the marker and the record (id, path,
    owner, line_hint), re-derives the declaration catalog and the dependency-crate list, and moves the manual
    flows that follow their function. Nothing is accepted that the guard (scripts/check-sqlite-large-values)
    does not accept; a site without a marker, or whose fingerprint changed, is reported and left alone."""
    import bisect
    import sqlite_large_values as slv

    root = Path(".")
    mpath = root / slv.MANIFEST
    manifest = json.loads(mpath.read_text())
    cache = {}
    sites, markers = slv.scan(root, cache)
    contents = {p: v[0] for p, v in cache.items()}
    old = {e["id"]: e for e in manifest["sites"]}
    by_path = collections.defaultdict(list)
    for m in markers:
        by_path[m[0]].append(m)
    ends = {p: [m[2] for m in ms] for p, ms in by_path.items()}
    remap = {}
    copies = []
    current_ids = {x.id for x in sites if x.id in old and old[x.id]["path"] == x.path}
    edits = collections.defaultdict(list)
    unmatched = []
    for site in sites:
        if site.id in old and old[site.id]["path"] == site.path:
            continue
        idx = bisect.bisect_right(ends.get(site.path, []), site.start) - 1
        marker = by_path[site.path][idx] if idx >= 0 else None
        if marker is None or marker[3] is None or contents[site.path][marker[2] : site.start].strip():
            unmatched.append(site)
            continue
        oid = marker[3]["id"]
        if oid not in old or old[oid]["fingerprint"] != site.fingerprint:
            unmatched.append(site)
            continue
        if oid in current_ids or oid in remap:
            copies.append((oid, site))  # the original is still there: this site is a copy of it
        else:
            remap[oid] = site
        edits[site.path].append((marker[1], marker[2], "/* sqlite-large-value: allow-for-now %s owner=%s TODO=%s */" % (site.id, site.owner, marker[3]["todo"])))
    entries = []
    for e in manifest["sites"]:
        s = remap.get(e["id"])
        if s is not None:
            e = dict(e, id=s.id, path=s.path, owner=s.owner, line_hint=s.line)
        entries.append(e)
    for oid, s in copies:
        entries.append(dict(old[oid], id=s.id, path=s.path, owner=s.owner, line_hint=s.line))
    manifest["sites"] = sorted(entries, key=lambda e: (e["path"], e["id"]))
    # manual flows follow their function
    flows = []
    for flow in manifest.get("flows", []):
        found = None
        candidates = [flow["path"]] + [p for p, (src, _) in cache.items() if p != flow["path"] and ("fn " + flow["function"]) in src]
        for path in candidates:
            if path not in cache:
                continue
            try:
                cur = slv.flow_site(path, cache[path][0], flow["function"])
            except ValueError:
                continue
            if cur["fingerprint"] == flow["fingerprint"]:
                found = cur
                break
        if found is None:
            flows.append(flow)
            continue
        if found["id"] != flow["id"] or found["path"] != flow["path"]:
            src = cache[found["path"]][0]
            comments = [t for t in cache[found["path"]][1] if t.kind == "comment" and t.end <= found["start"]]
            last = comments[-1] if comments else None
            if last is not None and slv.FLOW_MARKER.fullmatch(last.text) and not src[last.end : found["start"]].strip():
                edits[found["path"]].append((last.start, last.end, "/* sqlite-large-value-flow: allow-for-now %s owner=%s TODO=%s */" % (found["id"], found["owner"], flow["todo"])))
            flow = dict(flow, id=found["id"], path=found["path"], owner=found["owner"])
        flows.append(flow)
    manifest["flows"] = flows
    for path, es in edits.items():
        text = Path(path).read_text()
        for a, b, rep in sorted(es, reverse=True):
            text = text[:a] + rep + text[b:]
        Path(path).write_text(text)
    _reanchor_inventory(slv, root, manifest)
    # the catalogs are derived from the (rewritten) sources
    cache = {}
    sites, _ = slv.scan(root, cache)
    manifest["declarations"] = slv.declarations(root, sites, {p: v[0] for p, v in cache.items()})
    manifest["sqlite_crates"] = slv.sqlite_crates(root)
    mpath.write_text(_manifest_text(manifest))
    problems = slv.findings(root, manifest)
    print("inventory: %d sites moved, %d copied, %d unmatched, %d problems left" % (len(remap), len(copies), len(unmatched), len(problems)))
    for p in problems[:12]:
        print("  " + p)
    return 1 if problems else 0



def _unused_imports(cmd, crate):
    """({lib}, {test}) sets of (file, start, end) for the `unused_imports` warnings one `cargo test --no-run`
    of `crate` reports, split by the unit that reported them. The library is built the way its tests build
    it (with the test-support feature), which is also how the workspace builds it. A unit's messages come
    just before its artifact, whose profile says whether it is the test unit."""
    out = subprocess.run(cmd, capture_output=True, text=True).stdout
    lib, test, pending = set(), set(), set()
    for line in out.split("\n"):
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if crate not in m.get("package_id", "").replace("#", "/"):
            continue
        if m.get("reason") == "compiler-message":
            d = m["message"]
            if (d.get("code") or {}).get("code") == "unused_imports":
                for sp in d["spans"]:
                    if sp["is_primary"]:
                        pending.add((sp["file_name"], sp["byte_start"], sp["byte_end"]))
        elif m.get("reason") == "compiler-artifact" and m["target"]["kind"] == ["lib"]:
            (test if m["profile"]["test"] else lib).update(pending)
            pending = set()
    return lib, test


def _uses_with_context(text, items, in_test):
    """Every `use` item of a file, nested inline modules included, with whether it sits in test-only code."""
    for it in items:
        if it.kind == "use":
            yield it, in_test
        elif it.kind == "mod" and it.body:
            yield from _uses_with_context(text, ri.parse_items(text, it.body[0], it.body[1]), in_test or it.cfg_test)


def _imported_names(src):
    """The names the `use` items of a crate's files import from the crates below (by `st3_*`, `crate`, `super`)."""
    names = set()
    for rel in module_files(src):
        text = open(os.path.join(src, rel)).read()
        for it in ri.parse(text):
            if it.kind != "use":
                continue
            parsed = use_paths(it.text)
            if not parsed:
                continue
            for segs, alias in parsed[2]:
                if segs[-1] != "*" and (segs[0] in ("crate", "super", "self") or segs[0].startswith("st3_")):
                    names.add(segs[-1])
    return names


def cmd_imports(args):
    """Remove the imports nothing uses. `cargo fix` removes an import that only test code uses (the library
    build has no use for it); this removes what both the library and its test build leave unused, gates
    with `#[cfg(test)]` what only the library build does, and removes what the test build leaves unused in
    code the library never compiles (test-only files and `#[cfg(test)]` modules)."""
    for crate in [c for c in args.crates.split(",") if c]:
        lib, test = _unused_imports(["cargo", "test", "-p", crate, "--lib", "--no-run", "--message-format=json"], crate)
        src = "crates/%s/src" % crate
        parsed = {os.path.join(src, rel): (open(os.path.join(src, rel)).read(),) for rel in module_files(src)}
        parsed = {f: (t[0], ri.parse(t[0])) for f, t in parsed.items()}
        test_files = {os.path.join(src, rel) for rel in test_file_set({os.path.relpath(f, src): v for f, v in parsed.items()})}
        remove = lib & test
        gate = lib - test
        by_file = collections.defaultdict(lambda: ([], [], []))
        for f, a, b in remove:
            by_file[f][0].append((a, b))
        for f, a, b in gate:
            by_file[f][1].append((a, b))
        for f, a, b in test - lib:
            by_file[f][2].append((a, b))
        removed = gated_total = 0
        for f, (rem, gat, tonly) in by_file.items():
            raw = Path(f).read_bytes()
            text = raw.decode()
            def cspan(a, b):
                return len(raw[:a].decode()), len(raw[:b].decode())
            rem_c = [cspan(a, b) for a, b in rem]
            gat_c = [cspan(a, b) for a, b in gat]
            tonly_c = [cspan(a, b) for a, b in tonly]
            file_in_test = f in test_files or bool(TEST_FILE.search(f))
            edits = []
            for it, in_test in _uses_with_context(text, ri.parse(text), file_in_test):
                parsed_use = use_paths(it.text)
                if not parsed_use:
                    continue
                attrs, vis, paths = parsed_use
                code = _attr_start(text, it)
                inside = lambda spans: [(a, b) for a, b in spans if it.start <= a and b <= it.end]
                r, g = inside(rem_c), inside(gat_c)
                if in_test:
                    r += inside(tonly_c)
                if not r and not g:
                    continue

                def leaf_text(segs, alias):
                    return alias or segs[-1]

                def flagged(segs, alias, spans):
                    names = {text[a:b].split("::")[-1].split(" as ")[-1].strip() for a, b in spans}
                    whole = {text[a:b].strip() for a, b in spans}
                    if segs[-1] == "*":
                        return "::".join(segs[:-1]) in whole or "::".join(segs) in whole  # rustc flags `super` of `super::*`
                    return leaf_text(segs, alias) in names or "::".join(segs) in whole
                keep, gated = [], []
                for segs, alias in paths:
                    if flagged(segs, alias, r):
                        removed += 1
                    elif flagged(segs, alias, g):
                        gated.append((segs, alias))
                    else:
                        keep.append((segs, alias))
                new = render_use(attrs, vis, keep) if keep else ""
                if gated:
                    new = (new + "\n" if new else "") + render_use("#[cfg(test)]\n" + attrs, vis, gated)
                    gated_total += len(gated)
                edits.append((code, it.end, new))
            for a, b, rep in sorted(edits, reverse=True):
                text = text[:a] + rep + text[b:]
            Path(f).write_text(text)
        print("%s: removed %d, gated %d unused imports" % (crate, removed, gated_total))
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    pf = sub.add_parser("fix", help="cargo check the workspace and fix visibility and trait imports until it builds")
    pf.add_argument("--roots", required=True, help="comma-separated source dirs whose items may be made pub")
    pf.add_argument("--all-targets", action="store_true")
    pf.add_argument("--rounds", type=int, default=12)
    sub.add_parser("inventory", help="carry the SQLite large-value inventory over a code move")
    pi = sub.add_parser("imports", help="remove unused imports without breaking the test build")
    pi.add_argument("--crates", required=True)
    ps = sub.add_parser("sweep", help="remove the dead code a cut leaves in the lower crates")
    ps.add_argument("--roots", required=True)
    ps.add_argument("--rounds", type=int, default=4)
    for name in ("plan", "apply"):
        p = sub.add_parser(name)
        p.add_argument("--src", required=True)
        p.add_argument("--config", required=True)
        p.add_argument("--pins")
        p.add_argument("--above", action="append", default=[], help="PATH=LAYER: a crate already cut, whose items only refer downwards")
        if name == "plan":
            p.add_argument("--json")
            p.add_argument("--show")
        else:
            p.add_argument("--layer", required=True)
            p.add_argument("--crate")
            p.add_argument("--lower")
            p.add_argument("--src-dir", dest="src_dir")
            p.add_argument("--upper")
            p.add_argument("--description", default="A domain of the store")
    args = ap.parse_args(argv)
    if args.cmd == "fix":
        return cmd_fix(args)
    if args.cmd == "sweep":
        return cmd_sweep(args)
    if args.cmd == "inventory":
        return cmd_inventory(args)
    if args.cmd == "imports":
        return cmd_imports(args)
    config = load_config(args.config)
    pins = [l.strip() for l in open(args.pins)] if args.pins and os.path.exists(args.pins) else []
    nodes, files, tfiles = load(args.src)
    existing = {lay: name for lay, name in config.get("crates", {}).items() if os.path.isdir(f"crates/{name}/src")}
    layer_of = {lay: i + 1 for i, lay in enumerate(config["order"])}
    current = layer_of.get(getattr(args, "layer", None), 0)
    for lay, name in existing.items():
        # a crate already cut, above or below this layer, only refers downwards: what it names stays where it is
        if lay in layer_of and layer_of[lay] != current and f"crates/{name}/src={lay}" not in args.above:
            args.above.append(f"crates/{name}/src={lay}")
    named_by_cut = set()
    for spec in args.above:
        path, lay = spec.split("=")
        load(path, nodes=nodes, fixed=config["order"].index(lay) + 1)
        if config["order"].index(lay) + 1 < current:
            named_by_cut |= _imported_names(path)
    # what a crate already cut imports from below by name stays below (its `pub use st3_store::store::{..}` lists)
    for n in nodes:
        if n.fixed is None and not n.test and n.name in named_by_cut and n.kind in FREE_KINDS:
            n.pinned = True
    if args.cmd == "apply":
        if args.above:
            globals()["SHARED_SUFFIX"] = "_" + args.layer + "_tests"
        below = [(layer_of[l], n) for l, n in existing.items() if layer_of[l] < current]
        above = [(layer_of[l], n) for l, n in existing.items() if layer_of[l] > current]
        if not args.lower:
            args.lower = max(below)[1] if below else "st3-store"
        if not args.upper:
            args.upper = min(above)[1] if above else "st3-github"
        if not args.crate:
            args.crate = config["crates"][args.layer]
    build_refs(nodes)
    comp, order = plan(nodes, config, pins, current or None)
    if args.cmd == "plan":
        for name, c, b, pc in summarize(nodes, order):
            print(f"{name:10s} items {c:5d} bytes {b:8d} ({pc:.0f}%)")
        if args.show:
            for n in nodes:
                if (n.eff == 0 and args.show == "core") or (args.show.isdigit() and n.eff == int(args.show)):
                    print(n.key(), n.size)
        if args.json:
            json.dump([{"key": n.key(), "file": n.file, "kind": n.kind, "name": n.name, "eff": n.eff, "label": n.label, "size": n.size, "core": n.core} for n in nodes], open(args.json, "w"), indent=1)
        return 0
    args.src_dir = args.src_dir or args.src
    top = apply_cut(args, nodes, files, tfiles, order)
    wire_crate(args, top)
    return 0


if __name__ == "__main__":
    sys.exit(main())
