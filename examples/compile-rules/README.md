# Compile rules: a worked example

`config/` is a small layered configuration with a rules file beside its
hierarchy. It is the example the
[compile rules reference](../../docs/reference/compile-rules.md) uses, and a
test runs both commands below.

```
config/
├── hierarchy.yaml          the layers, least specific first
├── rules.yaml              the compile rules: never a layer
├── common.yaml             organization
├── environment/prod.yaml   environment
├── role/web.yaml           team
└── node/
    ├── web01.yaml          team: keeps every rule
    └── rogue.yaml          team: breaks them
```

A node that keeps the rules:

```
$ meridian rules check --hierarchy config/hierarchy.yaml --set env=prod --set role=web --set node=web01
rules: config/rules.yaml (9 rules)
layers, least specific first:
  common.yaml            organization
  environment/prod.yaml  environment
  role/web.yaml          team
  node/web01.yaml        team
no violations
```

A node that does not. It changes an organization variable, redirects an
organization secret, wipes the organization's credentials, repeats a final
key, names no owner and uses a floating image tag. Every violation is listed
and no value is shown; the full text is in
[`rogue.expected.txt`](rogue.expected.txt):

```
$ meridian rules check --hierarchy config/hierarchy.yaml --set env=prod --set role=web --set node=rogue
error: compile rules: 10 violations; nothing was compiled
  - [meridian/capabilities] layer "node/rogue.yaml" (team) sets "credentials", which team is not allowed to set: it may set only variables, packages, web
  ...
```
