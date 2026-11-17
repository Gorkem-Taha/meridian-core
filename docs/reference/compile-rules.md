# Compile rules reference

Compile rules are policies over the layers of a hierarchy, checked before
anything is compiled. This page is the reference: the files, every rule, every
field and every message. For what the rules are for and how they were
designed, read [the concept page](../concepts/compile-rules.md).

Every example on this page is in
[`examples/compile-rules/`](../../examples/compile-rules/README.md), and a
test runs it through the real command.

## Contents

- [The files](#the-files)
- [The hierarchy file](#the-hierarchy-file)
- [The rules file](#the-rules-file)
- [Groups](#groups)
- [Key patterns](#key-patterns)
- [Shipped rule sets](#shipped-rule-sets)
- [Rules](#rules): [`no-override`](#no-override), [`final`](#final),
  [`restrict`](#restrict), [`required`](#required),
  [`forbidden-value`](#forbidden-value), [`merge-strategy`](#merge-strategy)
- [How a rule reads a merge](#how-a-rule-reads-a-merge)
- [The command line as a layer](#the-command-line-as-a-layer)
- [Running the check](#running-the-check)
- [Refusals](#refusals)
- [Mistakes in the rules file](#mistakes-in-the-rules-file)
- [Protecting the rules file](#protecting-the-rules-file)
- [From DscPipelineRunner](#from-dscpipelinerunner)
- [Using the rules from Rust](#using-the-rules-from-rust)

## The files

```
config/
├── hierarchy.yaml        the layers, and how each key merges
├── rules.yaml            the compile rules: its own file, never a layer
├── common.yaml           organization
├── environment/
│   └── prod.yaml         environment
├── role/
│   └── web.yaml          team
└── node/
    └── web01.yaml        team
```

`rules.yaml` is found because it is beside `hierarchy.yaml`. It is not listed
in the hierarchy and must not be: a hierarchy with a layer path that resolves
to the rules file is refused.

```mermaid
flowchart LR
    subgraph hierarchy["hierarchy.yaml: least specific first"]
        direction LR
        A["common.yaml"] --> B["environment/prod.yaml"] --> C["role/web.yaml"] --> D["node/web01.yaml"]
    end
    D --> E["--data files"]
    R["rules.yaml"] -. "checks every layer,<br/>is not one" .-> hierarchy
    R -.-> E
    style R fill:#e8f0fe,stroke:#1a56db,color:#000
```

## The hierarchy file

The hierarchy file is what the rules are checked against, so its fields are
listed here.

```yaml
hierarchy:
  - common.yaml
  - environment/%{env}.yaml
  - role/%{role}.yaml
  - node/%{node}.yaml

default:
  strategy: deep-merge

keys:
  packages:
    strategy: unique-array-merge

strictness: lenient
```

| Field | Value |
|---|---|
| `hierarchy` | the layer files, **least specific first**, relative to the hierarchy file. A later layer is more specific and wins a merge. `%{name}` is replaced by the variable `name`, given with `--set name=value` |
| `default.strategy` | how a top-level key merges when `keys` does not say: `first-found` (the default), `deep-merge` or `unique-array-merge` |
| `keys.<key>.strategy` | the strategy for one top-level key |
| `keys.<key>.dedupKey` | for `unique-array-merge`: the field that identifies an item that is a mapping |
| `strictness` | `lenient` (the default): a layer file that does not exist is skipped. `strict`: it is an error |

| Strategy | What the more specific layer's value does |
|---|---|
| `first-found` | replaces the less specific layer's value for that top-level key, whole |
| `deep-merge` | mappings merge key by key at every depth; anything else is replaced |
| `unique-array-merge` | lists are joined and repeated items removed |

## The rules file

```yaml
rulesVersion: 1

layers:
  - name: organization
    paths: [common.yaml]
  - name: environment
    paths: ["environment/*"]
  - name: team
    paths: ["role/**", "node/**"]

use:
  - protect-variables
  - protect-secrets

rules:
  - id: region-is-final
    rule: final
    keys: [variables.Region]
```

| Field | Required | Value |
|---|---|---|
| `rulesVersion` | yes | `1` |
| `layers` | no | the [groups](#groups): who owns which layer |
| `use` | no | names of [shipped rule sets](#shipped-rule-sets) to switch on |
| `rules` | no | your own [rules](#rules) |

Nothing is in force that the file does not ask for. A rules file with only
`rulesVersion: 1` checks nothing.

Every rule has these fields, and then its own:

| Field | Required | Value |
|---|---|---|
| `id` | yes | a name for the rule, unique in the file. It opens every refusal the rule produces. Ids that start `meridian/` belong to the shipped rules |
| `rule` | yes | the kind of rule: `no-override`, `final`, `restrict`, `required`, `forbidden-value` or `merge-strategy` |
| `keys` | yes | the [key patterns](#key-patterns) the rule is about |
| `message` | no | your own words, added to every refusal the rule produces: who to ask, where the policy is written down |

The file is read strictly. An unknown field, a field a rule's kind does not
take, a group nobody declared: each is an error and not something to ignore,
because a rule that silently does nothing is worse than no rule. See
[mistakes in the rules file](#mistakes-in-the-rules-file).

## Groups

```yaml
layers:
  - name: organization
    paths: [common.yaml]
  - name: team
    paths: ["role/**", "node/**"]
```

A group is a named set of layers. Rules use group names to say whose values
are protected and who is constrained.

| Field | Value |
|---|---|
| `name` | the group's name, unique. Names in brackets are Meridian's own |
| `paths` | patterns matched against a layer's path as the hierarchy expands it, such as `node/web01.yaml` |

In a path pattern, `*` and `?` match within one path element, and `**` as a
whole element matches any number of elements, including none. Matching is
exact about case.

| Pattern | Matches | Does not match |
|---|---|---|
| `common.yaml` | `common.yaml` | `x/common.yaml` |
| `node/*` | `node/web01.yaml` | `node/eu/web01.yaml` |
| `node/**` | `node/web01.yaml`, `node/eu/web01.yaml` | `role/web.yaml` |
| `**/*.yaml` | every `.yaml` layer at any depth | `common.json` |

**The first group whose pattern matches owns the layer.** Put a narrower group
before a broader one.

**A layer no group claims may set nothing.** When the file declares groups,
every layer the hierarchy loads must belong to one. Otherwise a new path added
to the hierarchy would be a layer no rule that names a group applies to:

```
[meridian/unclaimed-layer] layer "extra.yaml" is not claimed by any group under `layers`; a layer nobody owns may set nothing
```

Two group names are Meridian's own and can be used in a rule without being
declared: `(command line)`, for [values given on the command
line](#the-command-line-as-a-layer), and `(unclaimed)`.

Groups are optional. A rules file without `layers` can still use every rule
that does not name a group, and its refusals name layers by path alone.

## Key patterns

A key pattern selects values inside a layer by their path, written with dots.

| Pattern | Selects |
|---|---|
| `variables` | the key `variables` **and everything beneath it** |
| `variables.**` | the same |
| `variables.Region` | that one key, and everything beneath it if it is a mapping |
| `*.password` | `password` directly beneath any top-level key |
| `**.password` | `password` at any depth, including the top |
| `secrets.*.path` | `path` of every entry under `secrets` |
| `**` | everything |

- `*` is any one key. `**` is any run of keys, including none.
- A pattern selects the key it names and everything beneath it. A rule that
  protects a mapping protects what is in it.
- Keys are compared exactly, including case: `variables.Region` does not
  select `variables.region`. They are two different keys in the data too.
- A key that contains a dot, a star or a quote is written in double quotes:
  `variables."app.kubernetes.io/name"`. In YAML that means quoting the whole
  pattern with single quotes: `'variables."a.b"'`.
- A list is one value. A pattern selects the list by its key, and a rule that
  looks at values ([`forbidden-value`](#forbidden-value)) looks at each item.

Refusals write keys the same way, so a key from a refusal can be pasted into
a rule.

## Shipped rule sets

Switched on by name under `use:`. Each is a [`no-override`](#no-override) rule
over every layer.

| Name | Rule id in a refusal | Protects |
|---|---|---|
| `protect-variables` | `meridian/protect-variables` | everything under `variables` |
| `protect-secrets` | `meridian/protect-secrets` | everything under `secrets` and under `credentials` |
| `protect-all` | `meridian/protect-all` | every key |

```yaml
use: [protect-variables, protect-secrets]
```

With these two, a layer may add a variable or a secret nobody has set, and may
repeat one unchanged, and may not change or remove one a less specific layer
set. To protect only what one group set, or to constrain only some groups,
write the rule yourself with [`setBy` and `appliesTo`](#no-override).

## Rules

### `no-override`

A more specific layer may not change a value a less specific layer set.

```yaml
- id: org-values-hold
  rule: no-override
  keys: [variables, secrets]
  setBy: [organization]
  appliesTo: [team]
```

| Field | Value |
|---|---|
| `setBy` | the groups whose values are protected. Left out: every layer's |
| `appliesTo` | the groups that are constrained. Left out: every layer |

What counts, for a selected value a less specific layer set:

| The more specific layer | Result |
|---|---|
| sets a different value | refused |
| sets a value of another type (`5432` and `"5432"`) | refused |
| replaces a mapping above it with a scalar, `null` or a list | refused, once for each value beneath |
| sets a sibling under a key that merges by replacing it whole, leaving the value out | refused: the value is dropped |
| replaces a list with another list, under `deep-merge` or `first-found` | refused |
| sets the same value again | allowed |
| adds a key nobody set | allowed |
| adds items to a list that merges with `unique-array-merge` | allowed: nothing is lost |

```
[org-values-hold] layer "node/web01.yaml" (team) changes "variables.Region", which layer "common.yaml" (organization) set
[org-values-hold] layer "node/web01.yaml" (team) sets "variables", replacing "variables.Region", which layer "common.yaml" (organization) set
[org-values-hold] layer "node/web01.yaml" (team) sets "variables", which is merged by replacing it whole (first-found), and so drops "variables.Region", which layer "common.yaml" (organization) set
```

When several less specific layers set the value, the refusal names the least
specific one: that is whose value the rule protects.

### `final`

Once a layer sets the key, no more specific layer may set it at all, not even
to the same value. Takes `setBy` and `appliesTo` as `no-override` does.

```yaml
- id: region-is-final
  rule: final
  keys: [variables.Region]
```

```
[region-is-final] layer "node/web01.yaml" (team) sets "variables.Region", which is final once set, and layer "common.yaml" (organization) set it
```

`final` does not say which layer sets the key first. Whichever does, that is
the last word. Use it where a second copy would be a copy that can go stale,
and use `no-override` where repeating a value is harmless. A final list cannot
be added to either, whatever its merge strategy.

### `restrict`

Only the named groups may set the key.

```yaml
- id: secrets-belong-to-the-organization
  rule: restrict
  keys: [secrets, credentials]
  allow: [organization]
```

| Field | Value |
|---|---|
| `allow` | required: the groups that may set the keys. An empty list means no layer may |

```
[secrets-belong-to-the-organization] layer "node/web01.yaml" (team) sets "secrets.mine.from", which only organization may set
[no-debug] layer "common.yaml" (organization) sets "debug", which no layer may set
```

A layer that may not set a restricted key also may not replace it from above.
With `keys: [secrets.db.password]`, a team layer that writes `secrets: null`
never names the key, and is refused all the same when an allowed layer set
it:

```
[s] layer "node/x.yaml" (team) sets "secrets", replacing "secrets.db.password.from", which layer "common.yaml" (organization) set and only organization may set
```

`restrict` is about who may write a key. It does not stop an allowed group's
more specific layer from changing an allowed group's less specific value; that
is [`no-override`](#no-override).

### `required`

The key must be set. `keys` are exact here: no wildcards.

```yaml
# In the merged result: from whichever layer.
- id: cost-centre-is-set
  rule: required
  keys: [variables.CostCentre]

# In every layer of a group.
- id: team-layers-name-an-owner
  rule: required
  keys: [variables.Owner]
  appliesTo: [team]
```

| Field | Value |
|---|---|
| `appliesTo` | left out: the key must be set in the merged result. Given: every layer of these groups must set it itself |

A key that is `null` is not set.

```
[cost-centre-is-set] no layer sets "variables.CostCentre", which must be set
[team-layers-name-an-owner] layer "node/web01.yaml" (team) does not set "variables.Owner", which every layer of team must set
```

The first form looks at the merged result, so a value the merge throws away
does not satisfy it. A layer file that does not exist is not a layer, so the
second form has nothing to say about it; use both when the value must exist
one way or another.

### `forbidden-value`

The key may not hold one of these values.

```yaml
- id: no-floating-tags
  rule: forbidden-value
  keys: ["**.image"]
  values: [latest]
  matches: ["*:latest"]
  ignoreCase: true
```

| Field | Value |
|---|---|
| `values` | scalars compared exactly, type included: `"0"` does not forbid `0` |
| `matches` | patterns over the value as text, where `*` is any run of characters and `?` any one. A number or a boolean is matched as it is written; `null` matches nothing |
| `ignoreCase` | `true`: compare text without regard to case. Default `false` |
| `appliesTo` | the groups that are checked. Left out: every layer |

At least one of `values` and `matches` is needed. Items of a list are checked
one by one, and the refusal says which:

```
[no-floating-tags] layer "node/web01.yaml" (team) sets "app.sidecars[0].image" to a value the rule forbids (matches[0])
```

The refusal names the entry of the rule that matched, by its position, and
not the value that was found.

### `merge-strategy`

The key may only be merged with one of these strategies. `keys` are exact
top-level keys, because that is what a strategy belongs to.

```yaml
- id: variables-merge-deep
  rule: merge-strategy
  keys: [variables]
  allow: [deep-merge]
```

```
[variables-merge-deep] "variables" is merged with first-found, and may only be merged with deep-merge
```

This is a rule about the hierarchy file and not about a layer, so it names no
layer. It is how a policy can insist on the merge direction it was written
for: under `first-found`, a layer that sets one variable replaces all of
them.

## How a rule reads a merge

`no-override` and `final` ask what a more specific layer's value does to a
less specific layer's, and the answer depends on the key's strategy.

```mermaid
flowchart TD
    S["a key both layers set"] --> K{"how does the key merge?"}
    K -- "first-found<br/>(replace)" --> RP["compare the two values leaf by leaf"]
    RP --> RP1["same value: restated"]
    RP --> RP2["different value: changed"]
    RP --> RP3["in the less specific layer only: <b>dropped</b>"]
    K -- "deep-merge" --> DP["both mappings: look inside, key by key"]
    DP --> DP1["same value: restated"]
    DP --> DP2["different value, or a mapping replaced<br/>by something else: changed"]
    DP --> DP3["in the less specific layer only: kept"]
    K -- "unique-array-merge<br/>(append)" --> AP["both lists: joined, nothing lost"]
    RP2 --> V{{"no-override: refused"}}
    RP3 --> V
    DP2 --> V
    RP1 --> F{{"final: refused"}}
    DP1 --> F
    AP --> F
    style V fill:#fde8e8,stroke:#c0392b,color:#000
    style F fill:#fde8e8,stroke:#c0392b,color:#000
```

`final` refuses everything `no-override` refuses as well.

## The command line as a layer

A `--data` file is merged over the hierarchy and wins, so it is checked as
the most specific layer. Each file is a layer of its own, named
`--data <file>`, in the group `(command line)`, whatever the file is called.
A command-line value replaces the top-level key it names whole, so it is
judged the way `first-found` is.

```
[meridian/protect-variables] layer "--data override.yaml" ((command line)) changes "variables.OrganizationName", which layer "common.yaml" (organization) set
```

To let the command line set a restricted key, name the group:
`allow: [organization, "(command line)"]`.

## Running the check

```
meridian rules check --hierarchy FILE [--rules FILE] [--require-rules]
                     [--set NAME=VALUE]... [--data FILE]...
```

| Option | Meaning |
|---|---|
| `--hierarchy FILE` | the hierarchy file. Layers are found relative to it |
| `--rules FILE` | the rules file. Default: `rules.yaml` beside the hierarchy file, when there is one |
| `--require-rules` | fail when there is no rules file |
| `--set NAME=VALUE` | a variable the layer paths read as `%{NAME}`; may be repeated |
| `--data FILE` | a file of values merged over the hierarchy and checked as its most specific layer; may be repeated |

It loads the layers one node would get, checks every rule, and compiles
nothing. Run it once for each node, or each combination of variables, that
your pipeline compiles:

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

The list shows which group owns each layer, which is the first thing to look
at when a rule does not apply where you expected.

| Exit status | Meaning |
|---|---|
| `0` | every rule holds, or there is no rules file and none was required |
| `1` | a rule is broken, or the hierarchy, a layer or the rules file is wrong |
| `2` | a mistake in the command line |

`compile`, `plan` and `apply` do not read a hierarchy yet, so they do not run
the rules by themselves; run `rules check` before them in a pipeline. See
[how this fits the rest of the language](../concepts/compile-rules.md#how-this-fits-the-rest-of-the-language).

## Refusals

```
$ meridian rules check --hierarchy config/hierarchy.yaml --set env=prod --set role=web --set node=rogue
error: compile rules: 9 violations; nothing was compiled
  - [meridian/protect-variables] layer "node/rogue.yaml" (team) changes "variables.OrganizationName", which layer "common.yaml" (organization) set
  - [meridian/protect-secrets] layer "node/rogue.yaml" (team) sets "credentials", replacing "credentials.deploy.type", which layer "common.yaml" (organization) set
  - [meridian/protect-secrets] layer "node/rogue.yaml" (team) sets "credentials", replacing "credentials.deploy.user", which layer "common.yaml" (organization) set
  - [meridian/protect-secrets] layer "node/rogue.yaml" (team) changes "secrets.database.path", which layer "common.yaml" (organization) set
  - [secrets-belong-to-the-organization] layer "node/rogue.yaml" (team) sets "credentials", which only organization may set. Ask the platform team to add it to common.yaml
  - [secrets-belong-to-the-organization] layer "node/rogue.yaml" (team) sets "secrets.database.path", which only organization may set. Ask the platform team to add it to common.yaml
  - [region-is-final] layer "node/rogue.yaml" (team) sets "variables.Region", which is final once set, and layer "common.yaml" (organization) set it
  - [team-layers-name-an-owner] layer "node/rogue.yaml" (team) does not set "variables.Owner", which every layer of team must set
  - [no-floating-tags] layer "node/rogue.yaml" (team) sets "web.image" to a value the rule forbids (matches[0])
```

Each line is `[rule id]`, the layer at fault and its group, the key, and what
the rule holds against it, followed by the rule's own `message` when it has
one.

- **Every violation is listed**, not only the first.
- **The order is fixed**: rules in the order the file lists them (shipped
  sets first), then layers from least to most specific, then keys. The same
  files always print the same text.
- **No value is ever printed.** Not the value that was set, not the one it
  replaced.
- **Nothing is handed over.** A hierarchy that breaks a rule produces no
  merged data at all.

## Mistakes in the rules file

These are errors in the file itself, reported before any layer is looked at.

| Message | Cause |
|---|---|
| `rulesVersion: required (write `rulesVersion: 1`)` | the file has no version |
| `rulesVersion: this Meridian reads version 1, found 2` | a file for a newer Meridian |
| `rules file: unknown field "ruls"` | a misspelt top-level field |
| `use[0]: there is no shipped rule set "..."` | a misspelt name under `use` |
| `layers[1].name: the group "a" is declared twice` | two groups with one name |
| `layers[0].paths: the group "a" claims no layers` | a group without `paths` |
| `rules[0].id: required`, `rules[1].id: the id "a" is used twice` | every rule needs its own id |
| `rules[0].rule: unknown rule "locked"` | not one of the six kinds |
| `rules[0]: a final rule does not take the field "allow"` | a field that belongs to another kind of rule |
| `rules[0].appliesTo: there is no group "team" under `layers`` | a rule names a group the file does not declare |
| `rules[0].keys: the key pattern "a..b" has an empty segment` | a malformed [key pattern](#key-patterns) |
| `rules[0].keys: a required rule names exact keys, and a.* has a wildcard` | `required` and `merge-strategy` take exact keys |
| `rule "m": allow: unknown merge strategy "deep"` | a `merge-strategy` rule names a strategy the hierarchy does not have |
| `the rules file ... is also the hierarchy layer "rules.yaml"` | the hierarchy lists the rules file as a layer |
| `compile rules are required for this run, and there is no rules.yaml ...` | `--require-rules` was given and there is no rules file |

## Protecting the rules file

`rules.yaml` is in the configuration repository, so whoever can change it
decides what every layer may do.

- Give `rules.yaml` and `hierarchy.yaml` to your most trusted owners, with
  CODEOWNERS and branch protection.
- Pass `--require-rules` in the pipeline, so that deleting the file is an
  error and not a way to switch the rules off.
- If that is not enough, keep the rules somewhere contributors cannot write
  and pass `--rules FILE`. It is used instead of the file in the repository.
- Give every folder of layers to a group. A layer outside every group is
  refused, so a new folder cannot appear outside the policy.

## From DscPipelineRunner

A DscPipelineRunner layer file has `parameters`, `variables` and `resources`
at the top, layers are listed most specific first in `Datum.yml`, and
`variables` merge deeply. The same shape in Meridian, with the organization's
variables made to hold:

| DscPipelineRunner `Datum.yml` | Meridian |
|---|---|
| `ResolutionPrecedence`, most specific first | `hierarchy`, **least** specific first: reverse the list |
| `Projects\$($Node.Project)` | `Projects/%{Project}.yaml`, with `--set Project=...` |
| `lookup_options: variables: merge_hash_array: deep` | `keys: variables: strategy: deep-merge` |
| `lookup_options: resources: merge_hash_array: UniqueKeyValTuples`, `tuple_keys: [name]` | `keys: resources: strategy: unique-array-merge`, `dedupKey: name`. The first layer to declare a name keeps its entry; entries are not merged |
| `Policy.yml` beside `Datum.yml` | `rules.yaml` beside `hierarchy.yaml` |
| `Layers: - Name: Platform`, `Paths: [ 'Platform/**' ]` | `layers: - name: platform`, `paths: ["Platform/**"]`. Paths include the file extension and are exact about case |
| `-RequirePolicy`, `-PolicyPath` | `--require-rules`, `--rules` |

```yaml
# hierarchy.yaml
hierarchy:
  - OrganizationPolicies/Organization.yaml
  - ProjectPolicies/Project.yaml
  - Projects/%{Project}.yaml
keys:
  variables: {strategy: deep-merge}
```

```yaml
# rules.yaml
rulesVersion: 1
layers:
  - name: platform
    paths: ["OrganizationPolicies/**", "ProjectPolicies/**"]
  - name: teams
    paths: ["Projects/**"]
rules:
  # A project file may add variables of its own. It may not change one the
  # platform set, and it may not declare a secret at all.
  - id: platform-variables-hold
    rule: no-override
    keys: [variables]
    setBy: [platform]
    appliesTo: [teams]
  - id: secrets-are-the-platform-s
    rule: restrict
    keys: [secrets]
    allow: [platform]
  - id: variables-merge-deep
    rule: merge-strategy
    keys: [variables]
    allow: [deep-merge]
```

With `OrganizationName: Contoso` in `Organization.yaml`, a
`Projects/Magenta.yaml` that sets `ProjectDescription` passes, and one that
sets `OrganizationName: Fabrikam` is refused:

```
[platform-variables-hold] layer "Projects/Magenta.yaml" (teams) changes "variables.OrganizationName", which layer "OrganizationPolicies/Organization.yaml" (platform) set
```

DscPipelineRunner's capability policy (which targets, engines and scripts a
layer may use) is about resources and not about data, and is not part of
compile rules.

## Using the rules from Rust

The crate is `meridian-rules`. A compiler makes one call:

```rust
let loaded = meridian_rules::enforce(&rules, &hierarchy, &scope, &loader, &command_line)?;
// loaded.resolved.data is the merged data. It exists only if every rule held.
```

| Item | Use |
|---|---|
| `Rules::parse(name, bytes)` | read a rules file |
| `enforce(rules, hierarchy, scope, loader, command_line)` | load, merge, check; `Ok(Loaded)` or `Err(Refusal)` |
| `check_files(files, scope, command_line)` | the same from paths on disk, finding `rules.yaml` and refusing a rules file that is a layer |
| `evaluate(rules, stack)` | check rules over any `Stack`: layers, a `MergePlan` and the merged result |
| `MergePlan` | how a stack merges: `kind(path)` gives `Replace`, `Deep` or `Append`. Implemented for `meridian_resolve::Hierarchy`; implement it to check another way of layering with the same rules |
| `Violation` | `rule`, `check`, `layer`, `group`, `key` and `text`, for a caller that reports in its own format |

`evaluate` is not tied to data files. Anything that can be presented as an
ordered stack of layers of keyed values can be checked by the same rules.
