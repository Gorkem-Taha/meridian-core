# How Meridian is tested

This page says what tests exist today, what each kind is for, and how to add to
them. The [testing plan](../plans/testing-plan.md) says where testing is
heading; this page is what CI enforces now.

```mermaid
flowchart TD
    U["Unit tests<br/>one crate, one behaviour, table-driven"]
    G["Golden files<br/>a document in, the tool's files out, byte for byte"]
    P["Property tests<br/>what holds for every input"]
    C["Conformance tests<br/>the same document on every target"]
    E["Examples<br/>every file under examples/, through the real command"]
    R["Real-tool workflows<br/>the tool itself, in CI"]
    U --> G --> C --> E --> R
    U --> P
```

| Kind | Lives in | Run it |
|---|---|---|
| Unit tests | `crates/*/tests/`, `plugins/*/tests/` | `cargo test -p <crate>` |
| Golden files | `plugins/<name>/testdata/` | `cargo test -p meridian-target-<name>`; record with `UPDATE_GOLDEN=1` |
| Property tests | `crates/meridian-cli/tests/properties.rs` | `cargo test -p meridian-cli --test properties` |
| Conformance tests | `crates/meridian-cli/tests/conformance.rs` | `cargo test -p meridian-cli --test conformance` |
| Robustness tests | `crates/meridian-cli/tests/robustness.rs` | `cargo test -p meridian-cli --test robustness` |
| Examples | `crates/meridian-cli/tests/examples.rs` | `cargo test -p meridian-cli --test examples` |
| Real tools | a workflow per tool under `.github/workflows/` | in CI; each target's page says how |

Everything except the real-tool workflows runs with `cargo test --workspace`,
needs no tool installed, and takes a few minutes.

## Unit tests and golden files

A plugin's `tests/compile.rs` compiles documents in process and checks two
things. What a valid document compiles to is compared with a golden file,
because the exact bytes are the contract with the tool. What an invalid document
is refused with is asserted field by field: the rule, the resource, the message
and the line. A refusal's wording is part of what a user sees, so it is tested
as closely as the output is.

`tests/run.rs` tests the runner against a stand-in: a shell script that takes
the tool's place, sees its real arguments and working directory, and answers as
the tool would. [Writing a plugin](writing-a-plugin.md) has both in detail.

## Property tests

An example-based test says "this input gives that output". A property test
says what must hold for every input, and lets the machine look for an input that
breaks it. `properties.rs` runs about twenty-five of them, a few thousand
generated inputs each, in three shapes:

| Shape | Examples |
|---|---|
| **Nothing panics** | arbitrary bytes, punctuation soup, a real document with bytes cut out or doubled, and nesting thousands deep are each answered with a value or an error, never a crash |
| **Round trips** | YAML Meridian writes is read back as the same value; canonical JSON decodes to what was encoded; a document is the same document in YAML and in JSON |
| **A model agrees** | a second, naive implementation written in the test decides the same as the real one: the condition language, the topological sort, `when` pruning, label selectors, and each hierarchy merge strategy |

The model tests are the ones behind the claims the documentation makes. "A
condition means the same on every target" rests on the condition language
having one meaning, and
`a_condition_is_decided_as_the_documented_language_says` holds the evaluator to
the grammar and the rules in [conditions](../reference/dsl/conditions.md) for
three thousand generated expressions a run, with their precedence, their
short-circuiting and their errors. The model covers the core of the language:
literals, paths into the data, `defined()`, `!`, `&&`, `||`, `==` and `!=`.
The other operators and the functions are tested in `crates/meridian-expr`.

An example that decides by `when` needs its data before it can be compiled,
so the conformance and robustness tests, which build a tree straight from a
file, pass over it. `examples.rs` compiles it through the command with its
data files.

When a property fails, the output ends with the smallest input that still
fails:

```
minimal failing input: original = List([Float(8.038549324716471e-175)])
```

proptest also writes that input to
`crates/meridian-cli/tests/properties.proptest-regressions`, and tries it first
on every later run. Commit the file with the fix. To look harder for a failure
locally, raise the number of cases:

```
PROPTEST_CASES=20000 cargo test -p meridian-cli --test properties
```

What these tests have found so far, each fixed with a test of its own:

- A float in a JSON document or crossing the plugin boundary could change in
  its last digit. JSON numbers are now read exactly.
- A document nested a few hundred levels deep could overflow the stack of a
  thread with a small one. The limit is now 64 levels, in YAML and in JSON, and
  holds on a 1 MiB stack.

### Writing one

Add a property when you add something that parses, orders, merges or decides.
State the rule as the documentation states it, write the slowest, most obvious
implementation of that rule in the test, and assert the two agree:

```rust
proptest! {
    #![proptest_config(cases(1500))]

    #[test]
    fn a_selector_selects_exactly_when_every_requirement_holds(
        requirements in vec(requirement(), 1..5),
        labels in btree_map(key(), value(), 0..4),
    ) {
        let expected = requirements.iter().all(|r| r.holds(&labels));
        let selector = Selector::parse(&text_of(&requirements)).unwrap();
        prop_assert_eq!(selector.matches(&labels), expected);
    }
}
```

Generate inputs that are nearly right as well as inputs that are random: a
valid document with one edit finds more than noise does.

## Conformance tests

`conformance.rs` compiles the same document with every target that should
agree about it.

**The shared dialect.** About ninety small documents, covering every type,
state and param of the [configuration management types](../reference/resource-types.md),
`dependsOn`, `notifies`, references and `runtimeWhen`, are each compiled by
Ansible, Chef, cloud-init, OpenVox, Puppet and Salt. How each compile came out
is written to the [conformance matrix](../reference/conformance-matrix.md), and
the test fails when that page is out of date. Each case is marked as one the
targets must agree on or one where the tools differ; a target that breaks ranks
on a case marked the same fails the test. Further tests hold all six to:

- Puppet and OpenVox compile every case to the same manifest;
- every case compiles to the same bytes twice;
- every refusal names its target, its resource and a line;
- a settled `${...}` leaves no marker in any output, and the value it named is
  there;
- a dependency is written before what depends on it.

**Every target.** Each example under `examples/<target>/` is put through rules
that hold whatever the segment: it compiles deterministically to relative
paths; a `runtimeWhen` on any of its resources is compiled in or refused, never
dropped; a target that declares no notify refuses one between any two
resources; an unknown type is refused under one rule. A document written for
Terraform compiles to the same configuration for OpenTofu.

To add a case, add a `case(...)` to `CASES` and record the page:

```
UPDATE_GOLDEN=1 cargo test -p meridian-cli --test conformance
```

Read the row it adds. If the targets disagree where you expected them to
agree, that is a finding: either a target is wrong, or the case is `Varies` and
the reference should say why.

## Robustness tests

A plugin is a separate program, and must not trust that the tree it is handed
was checked. `robustness.rs` takes every example's tree and damages it in the
ways a careless or hostile host could: about fifty awkward strings (empty,
newlines, quotes, every tool's own interpolation syntax, path traversal,
control characters) and a dozen awkward values in every name, id, type, state
and param; resources missing, doubled or reversed; edges that point nowhere or
in a circle; outputs that read nothing. Tens of thousands of trees per run,
and one rule for every target: the answer is files at relative paths, or a
refusal that names the target and the rule. Never a panic.

The first run found one: a resource id used twice made the Ansible, Puppet and
OpenVox sort stages panic. They now refuse it as `duplicate-resource`.

## Examples

Every `.yaml` file under `examples/` is validated and compiled by the real
`meridian` command with the real plugin binaries. The workflows under
`examples/workflow/` are held to more:

- each document that can be compiled is written to a directory named after it,
  and each ResourceSet that waits for an output is reported as deferred;
- each deferred document is compiled again with a host filled in, so a mistake
  in a document that waits is still caught;
- a workflow with data files is compiled once per data set
  (`site.data.yaml`, then `site.<name>.data.yaml` layered over it), and
  `environments.yaml` is checked resource by resource against what its
  conditions should keep in development and in production.

An example the documentation shows is always one of these files, so it cannot
rot.

## Coverage

The `coverage` workflow measures line, function and region coverage of the
whole workspace on every pull request, writes the table to the run's summary,
and fails if line coverage falls below its floor. To measure it yourself:

```
cargo install cargo-llvm-cov --locked
rustup component add llvm-tools
cargo llvm-cov --workspace --locked --summary-only
cargo llvm-cov --workspace --locked --html      # target/llvm-cov/html/index.html
```

Each plugin's `main.rs` shows as uncovered. It is three lines that hand the
plugin to the SDK, and it runs in a separate process the measurement does not
follow; the tests that start the plugin's real binary, and the examples test,
are what exercise it.

A number is a way to find code no test reaches, not a goal. A refusal that no
test triggers is the usual finding, and the fix is a test that names the rule
and the message.
