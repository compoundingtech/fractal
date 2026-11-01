# Accountable fleet roots

An accountable fleet root can declare the human responsible for its trace:

```kdl
version 2
agent "example/root" {
    accountable "person/avery"
    harness "codex" {}
}
```

Publish it as `person/avery`. Only the named person may author this declaration;
an agent, another person, or a missing author is refused. The field belongs on a
top-level root, never a mission-owned seat. Delegated declaration is not supported
in this slice and needs a separate contract decision. Descendants do not copy or
override the field. Trace resolution follows their declaration authors to the root;
a cycle or a chain without a declared root gives no principal. Account bindings
and mission requesters are not trace-principal inputs.

This is a trace attribution fact. It grants no authority and does not affect whose
person receives asks, own-ask checks, or directive notes. The canonical KDL child
and the optional `intent.desired.accountable` field must match. Roots also carry
a matching `fields.accountable` envelope so older admission recognizes the new
field as unknown before decoding the desired body. Declaration reads
expose the optional field in generated clients.

Upgrade **every member** before publishing a root with this field. Older members
retain the new claim field as unknown, without enforcing it or selecting the root.
Existing declarations need no migration and declare no trace principal. Removing
the field requires the normal declaration update; rollback requires removing it
on upgraded members first.

These declaration restrictions apply to both local and replicated admission. The field changes
neither the database layout nor checkpoint rules.
