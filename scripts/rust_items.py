"""A small, tolerant parser for the item structure of a Rust source file.

It finds the top-level items of a file (and the fns, consts and types inside `impl` blocks) with
their exact byte spans, including the doc comments and attributes in front of each. It does not
parse expressions: bodies are skipped by matching braces while stepping over comments and string,
char and raw-string literals (module_layers._skip_literal). The spans are what a code-motion tool
needs: cutting an item out of a file and pasting it into another keeps its text byte for byte.

The invariant `parse(text)` keeps is that the items' spans are disjoint, in order, and that the
text between them is only whitespace, plain comments and `use`/`mod x;` lines the caller treats
as glue (see `Item.kind`).
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import module_layers as ml  # noqa: E402

KEYWORDS = ("fn", "struct", "enum", "union", "trait", "type", "static", "const", "mod", "impl", "use", "extern")
_VIS = re.compile(r"pub(?:\s*\([^)]*\))?\s+")
# the same, then any block comments (the SQLite large-value inventory puts its markers there)
_VIS_COMMENTED = re.compile(r"pub(?:\s*\([^)]*\))?(?:\s+(?:/\*.*?\*/\s*)*|\s*(?:/\*.*?\*/\s*)+)", re.S)
_MODS = re.compile(r"(?:(?:default|async|unsafe|safe)\s+|const\s+(?=(?:async|unsafe|extern|fn)\b))*(?:extern\s+(?:\"[^\"]*\"\s+)?)?")


class Item:
    def __init__(self, kind, name, start, end, text, header=None):
        self.kind = kind  # fn struct enum union trait type static const mod impl use macro_rules macro_call other
        self.name = name
        self.start = start
        self.end = end
        self.text = text
        self.header = header  # impl header, for kind == "impl"
        self.children = []  # items of an impl or trait body
        self.impl_type = None  # base type name of an impl
        self.impl_trait = None  # trait name of a trait impl
        self.parent = None
        self.cfg_test = False
        self.is_pub = False
        self.body = None  # for an inline `mod x { .. }`: the span between the braces
        self.file = None

    def __repr__(self):
        return f"Item({self.kind} {self.name!r} {self.start}..{self.end})"


def _skip_ws_and_comments(s, i, n):
    """Advance over whitespace and comments; return the new index."""
    while i < n:
        c = s[i]
        if c.isspace():
            i += 1
        elif s.startswith("//", i):
            j = s.find("\n", i)
            i = n if j < 0 else j
        elif s.startswith("/*", i):
            i = _skip_block_comment(s, i)
        else:
            break
    return i


def _skip_block_comment(s, i):
    depth, i = 1, i + 2
    n = len(s)
    while i < n and depth:
        if s.startswith("/*", i):
            depth, i = depth + 1, i + 2
        elif s.startswith("*/", i):
            depth, i = depth - 1, i + 2
        else:
            i += 1
    return i


def _match_delim(s, i):
    """s[i] is an opening ( [ or {; return the index just past its matching closer."""
    n = len(s)
    stack = []
    while i < n:
        k = ml._skip_literal(s, i)
        if k != i:
            i = k
            continue
        if s.startswith("//", i):
            j = s.find("\n", i)
            i = n if j < 0 else j
            continue
        if s.startswith("/*", i):
            i = _skip_block_comment(s, i)
            continue
        c = s[i]
        if c in "([{":
            stack.append(c)
        elif c in ")]}":
            stack.pop()
            if not stack:
                return i + 1
        i += 1
    raise ValueError("unbalanced delimiters")


def _find_body_or_semi(s, i):
    """From the end of an item's keyword/name, find the first `{` or `;` outside parens/brackets/angles'
    parens. Returns (index, char)."""
    n = len(s)
    depth = 0
    while i < n:
        k = ml._skip_literal(s, i)
        if k != i:
            i = k
            continue
        if s.startswith("//", i):
            j = s.find("\n", i)
            i = n if j < 0 else j
            continue
        if s.startswith("/*", i):
            i = _skip_block_comment(s, i)
            continue
        c = s[i]
        if c in "([":
            depth += 1
        elif c in ")]":
            depth -= 1
        elif depth == 0 and c in "{;":
            return i, c
        i += 1
    raise ValueError("no item end")


def _leading_meta(s, i, n):
    """Index of the start of the doc comments, plain comments and attributes at i (contiguous lines),
    returning (meta_start, first_code_index, attrs_text)."""
    start = i
    attrs = []
    while True:
        j = i
        while j < n and s[j] in " \t":
            j += 1
        if s.startswith("///", j) or (s.startswith("//", j) and not s.startswith("////", j)) or s.startswith("//!", j):
            k = s.find("\n", j)
            i = n if k < 0 else k + 1
            continue
        if s.startswith("/*", j):
            # a plain or doc block comment in front of an item belongs to it
            i = _skip_block_comment(s, j)
            continue
        if s.startswith("#![", j):
            # an inner attribute belongs to the module, not to the next item: what came before it does not either
            e = _match_delim(s, s.index("[", j))
            start = e
            attrs = []
            i = e
            while i < n and s[i] in " \t":
                i += 1
            if i < n and s[i] == "\n":
                i += 1
            continue
        if s.startswith("#[", j):
            k = s.index("[", j)
            e = _match_delim(s, k)
            attrs.append(s[j:e])
            i = e
            while i < n and s[i] in " \t":
                i += 1
            if i < n and s[i] == "\n":
                i += 1
            continue
        if j < n and s[j] == "\n" and False:
            continue
        return start, j, attrs


def parse_items(s, a=0, b=None, in_impl=False):
    """Items between a and b of s, in order. Gaps (blank lines, plain stray comments) are not items."""
    n = len(s) if b is None else b
    items = []
    i = a
    while True:
        i = _skip_ws_and_comments_but_keep_leading(s, i, n)
        if i >= n:
            break
        meta_start, code, attrs = _leading_meta(s, i, n)
        if code >= n:
            break
        if s[code] == "\n":
            i = code + 1
            continue
        rest_start = code
        m = _VIS_COMMENTED.match(s, rest_start)
        is_pub = bool(m)
        p = m.end() if m else rest_start
        m2 = _MODS.match(s, p)
        q = m2.end() if m2 else p
        kw = re.match(r"(fn|struct|enum|union|trait|type|static|const|mod|impl|use|macro_rules|macro)\b", s[q : q + 16])
        item = None
        if s.startswith("extern crate", p):
            e = s.index(";", p) + 1
            item = Item("other", None, meta_start, e, s[meta_start:e])
        elif kw is None and re.match(r"[A-Za-z_]\w*(?:::[A-Za-z_]\w*)*\s*!", s[p : p + 80]):
            # macro invocation at item level
            mm = re.match(r"[A-Za-z_]\w*(?:::[A-Za-z_]\w*)*\s*!\s*", s[p:])
            d = p + mm.end()
            # optional macro name (macro_rules! x) handled below; here `name!{..}` or `name!(..);`
            e = _match_delim(s, d)
            while e < n and s[e] in " \t":
                e += 1
            if e < n and s[e] == ";":
                e += 1
            item = Item("macro_call", None, meta_start, e, s[meta_start:e])
        elif kw is None:
            # unknown token at item position: skip to the next newline to avoid an infinite loop
            e = s.find("\n", p)
            e = n if e < 0 else e + 1
            item = Item("other", None, meta_start, e, s[meta_start:e])
        else:
            k = kw.group(1)
            after = q + kw.end()
            if k == "macro_rules":
                mm = re.match(r"\s*!\s*([A-Za-z_]\w*)\s*", s[after:])
                d = after + mm.end()
                e = _match_delim(s, d)
                while e < n and s[e] in " \t":
                    e += 1
                if e < n and s[e] == ";":
                    e += 1
                item = Item("macro_rules", mm.group(1), meta_start, e, s[meta_start:e])
            elif k == "use":
                e = _find_body_or_semi(s, after)
                # a `use a::{b, c};` has braces: find the real terminating `;`
                if e[1] == "{":
                    e2 = _match_delim(s, e[0])
                    e3 = s.index(";", e2)
                    end = e3 + 1
                else:
                    end = e[0] + 1
                item = Item("use", None, meta_start, end, s[meta_start:end])
            elif k == "impl":
                bi, ch = _find_body_or_semi(s, after)
                header = s[after:bi].strip()
                if ch == ";":
                    raise ValueError("impl without body")
                be = _match_delim(s, bi)
                item = Item("impl", None, meta_start, be, s[meta_start:be], header=header)
                item.children = parse_items(s, bi + 1, be - 1, in_impl=True)
                for c in item.children:
                    c.parent = item
                _set_impl_names(item)
            elif k == "mod":
                mm = re.match(r"\s*([A-Za-z_]\w*)", s[after:])
                name = mm.group(1)
                bi, ch = _find_body_or_semi(s, after + mm.end())
                if ch == ";":
                    e = bi + 1
                else:
                    e = _match_delim(s, bi)
                item = Item("mod", name, meta_start, e, s[meta_start:e])
                item.body = (bi + 1, e - 1) if ch == "{" else None
            elif k == "trait":
                mm = re.match(r"\s*([A-Za-z_]\w*)", s[after:])
                name = mm.group(1)
                bi, ch = _find_body_or_semi(s, after + mm.end())
                e = _match_delim(s, bi) if ch == "{" else bi + 1
                item = Item("trait", name, meta_start, e, s[meta_start:e])
            elif k in ("const", "static") and re.match(r"\s*(?:mut\s+)?_\s*[:=]", s[after:]):
                e = _find_end_semi(s, after)
                item = Item("const", "_", meta_start, e, s[meta_start:e])
            else:
                mm = re.match(r"\s*(?:mut\s+)?([A-Za-z_]\w*)", s[after:])
                if k == "const" and s[after:].lstrip().startswith(("fn ", "async ", "unsafe ", "extern ")):
                    # `const fn` was consumed by _MODS; reach here only for odd spellings
                    pass
                name = mm.group(1)
                if k in ("fn",):
                    bi, ch = _find_body_or_semi(s, after + mm.end())
                    e = _match_delim(s, bi) if ch == "{" else bi + 1
                elif k in ("struct", "union"):
                    bi, ch = _find_body_or_semi(s, after + mm.end())
                    if ch == "{":
                        e = _match_delim(s, bi)
                    else:
                        # tuple/unit struct: may have a where clause before ';'
                        e = bi + 1
                elif k == "enum":
                    bi, ch = _find_body_or_semi(s, after + mm.end())
                    e = _match_delim(s, bi)
                else:  # type, static, const
                    e = _find_end_semi(s, after + mm.end())
                item = Item(k, name, meta_start, e, s[meta_start:e])
            item.is_pub = is_pub
        item.attrs = attrs
        item.cfg_test = any(re.match(r"#\[cfg\(\s*(?:test\s*\)|any\(\s*test\b|all\(\s*test\b)", a.strip()) for a in attrs)
        item.is_pub = is_pub if item.kind not in ("macro_call", "other") else False
        items.append(item)
        i = item.end
    return items


def _skip_ws_and_comments_but_keep_leading(s, i, n):
    """Skip blank lines and inner doc/attributes; comments directly above an item are its leading meta."""
    while i < n:
        if s[i] in " \t\r\n":
            i += 1
        elif s.startswith("//!", i):
            j = s.find("\n", i)
            i = n if j < 0 else j
        elif s.startswith("#![", i):
            i = _match_delim(s, s.index("[", i))
        else:
            break
    return i


def _find_end_semi(s, i):
    n = len(s)
    depth = 0
    while i < n:
        k = ml._skip_literal(s, i)
        if k != i:
            i = k
            continue
        if s.startswith("//", i):
            j = s.find("\n", i)
            i = n if j < 0 else j
            continue
        if s.startswith("/*", i):
            i = _skip_block_comment(s, i)
            continue
        c = s[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == ";" and depth == 0:
            return i + 1
        i += 1
    raise ValueError("no semicolon")


def _strip_generics(h):
    """Remove leading `<...>` generics of an impl header."""
    h = h.strip()
    if h.startswith("<"):
        depth = 0
        for k, c in enumerate(h):
            if c == "<":
                depth += 1
            elif c == ">":
                depth -= 1
                if depth == 0:
                    return h[k + 1 :].strip()
    return h


def _base_type(t):
    t = t.strip()
    t = re.split(r"\s+where\b", t)[0].strip()
    t = t.lstrip("&").strip()
    t = re.sub(r"^(?:mut|dyn)\s+", "", t)
    m = re.match(r"([A-Za-z_]\w*(?:::[A-Za-z_]\w*)*)", t)
    return m.group(1).split("::")[-1] if m else t


def _set_impl_names(item):
    h = _strip_generics(item.header)
    # split on a top-level ` for `
    depth = 0
    cut = None
    for m in re.finditer(r"<|>|\bfor\b", h):
        t = m.group(0)
        if t == "<":
            depth += 1
        elif t == ">":
            depth -= 1
        elif depth == 0:
            cut = m
            break
    if cut is not None:
        item.impl_trait = _base_type(h[: cut.start()])
        item.impl_type = _base_type(h[cut.end() :])
    else:
        item.impl_type = _base_type(h)
    item.name = f"impl {item.impl_trait + ' for ' if item.impl_trait else ''}{item.impl_type}"


def parse(text):
    return parse_items(text)


def check_roundtrip(text):
    """The text outside the items is whitespace, comments and nothing else: return the stray text."""
    items = parse(text)
    stray = []
    pos = 0
    for it in items:
        stray.append(text[pos : it.start])
        pos = it.end
    stray.append(text[pos:])
    leftover = [x for x in stray if x.strip() and not _only_comments(x)]
    return items, leftover


def _only_comments(x):
    y = ml.strip_comments(x)
    return not y.strip()
