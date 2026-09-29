#!/usr/bin/env python3
"""
Migrate the `register_validator` ABI change: insert a `region` argument into
every `register_validator` / `try_register_validator` call and every
`batch_register_validators` entries tuple element.

New signature:
    register_validator(wallet, credentials, affiliation, region, specializations)
"""
import os

ROOT = "contracts"


def find_matching_paren(text, open_idx):
    """text[open_idx] is '('. Returns index of matching ')' respecting nesting/strings."""
    depth = 0
    i = open_idx
    in_str = False
    while i < len(text):
        ch = text[i]
        if in_str:
            if ch == "\\":
                i += 2
                continue
            if ch == '"':
                in_str = False
            i += 1
            continue
        if ch == '"':
            in_str = True
            i += 1
            continue
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    raise ValueError("unbalanced parens")


def split_top_level(s):
    parts = []
    depth = 0
    cur = []
    in_str = False
    for ch in s:
        if in_str:
            cur.append(ch)
            if ch == "\\":
                cur.append("")
                continue
            if ch == '"':
                in_str = False
            continue
        if ch == '"':
            in_str = True
            cur.append(ch)
            continue
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append("".join(cur).strip())
            cur = []
        else:
            cur.append(ch)
    tail = "".join(cur).strip()
    if tail:
        parts.append(tail)
    return parts


def derive_env(text):
    m = None
    # look for a from_str( token
    import re
    for m in re.finditer(r"from_str\(\s*(&?[A-Za-z0-9_$.]+)", text):
        return m.group(1)
    return "env"


def migrate_file(path):
    with open(path, "r", encoding="utf-8") as f:
        content = f.read()
    count = 0

    # Walk and rewrite register_validator / try_register_validator calls.
    out = []
    i = 0
    n = len(content)
    while i < n:
        m = None
        # search for the function name token
        idx = content.find("register_validator", i)
        if idx == -1:
            out.append(content[i:])
            break
        # check it's a call `register_validator(`
        j = idx + len("register_validator")
        # allow optional `try_` prefix already included via find? we search generic
        while j < n and content[j] in " \t\n":
            j += 1
        if j >= n or content[j] != "(":
            out.append(content[i:idx].rstrip() + "register_validator")
            i = idx + len("register_validator")
            continue
        close = find_matching_paren(content, j)
        call_inner = content[j + 1 : close]
        parts = split_top_level(call_inner)
        if len(parts) == 4:
            envtok = derive_env(parts[1] + " " + parts[2])
            region = '&String::from_str({}, "Default Region")'.format(envtok)
            new_inner = ", ".join([parts[0], parts[1], parts[2], region, parts[3]])
            out.append(content[i:j] + "(" + new_inner + ")")
            count += 1
        else:
            out.append(content[i : close + 1])
        i = close + 1

    content = "".join(out)
    return content, count


def main():
    total = 0
    changed = []
    for rootdir, dirs, files in os.walk(ROOT):
        for fn in files:
            if not fn.endswith(".rs"):
                continue
            path = os.path.join(rootdir, fn)
            new, c = migrate_file(path)
            if c:
                with open(path, "w", encoding="utf-8") as f:
                    f.write(new)
                total += c
                changed.append((path, c))
    for p, c in sorted(changed):
        print(f"{c:4d}  {p}")
    print("TOTAL register_validator insertions:", total)


if __name__ == "__main__":
    main()