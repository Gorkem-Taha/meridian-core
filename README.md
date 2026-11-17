# meridian-core

Meridian is one declarative language for infrastructure and configuration. You describe
what you want once; Meridian compiles it into the native files of the tool you choose and
then drives that tool to plan and apply it.

It is a transpiler and an orchestrator, not a replacement engine. Ansible still runs the
playbook and Puppet still applies the catalog. Where a tool cannot express what a document
asks for, Meridian refuses at compile time and says why. It never emits output that
quietly does less.

## Status

Early, and moving quickly. The accepted specifications are in [`docs/`](docs/index.md);
start with the [design plan](docs/plans/design-plan.md).

| Plugin | Category | Compiles to | Runs with |
|---|---|---|---|
| [`ansible`](docs/targets/ansible.md) | configuration management | a playbook and inventory | `ansible-playbook` |
| [`puppet`](docs/targets/puppet.md) | configuration management | a class and node statement | `puppet apply`, or `bolt apply` for another host |
| [`chef`](docs/targets/chef.md) | configuration management | a cookbook | `chef-client`, or `chef-run` for another host |
| [`dsc`](docs/targets/dsc.md) | configuration management | a DscPipelineRunner configuration | `Invoke-DscRunner` (DSC v2 and v3) |
| [`cloud-init`](docs/targets/cloud-init.md) | configuration management | first-boot user-data and a NoCloud seed | `cloud-init schema` to check it; the machine applies it on its first boot |
| [`terraform`](docs/targets/terraform.md) | infrastructure as code | `main.tf.json` | `terraform`, or `tofu` |
| [`opentofu`](docs/targets/opentofu.md) | infrastructure as code | `main.tofu.json`, with state and plan encryption | `tofu` |
| [`cloudformation`](docs/targets/cloudformation.md) | infrastructure as code | a template | `aws cloudformation` change sets |
| [`bicep`](docs/targets/bicep.md) | infrastructure as code | `main.bicep` | `az deployment group` |
| [`docker`](docs/targets/docker.md) | containerization | a Compose file | `docker compose` |
| [`podman`](docs/targets/podman.md) | containerization | Quadlet units | `systemctl` and `podman`, over `ssh` for another host |
| [`kubernetes`](docs/targets/kubernetes.md) | container orchestration | manifests | `kubectl apply` |
| [`helm`](docs/targets/helm.md) | container orchestration | a Helm chart, with an optional Kustomize overlay | `helm upgrade --install`, with `kustomize build` for an overlay |

What each can and cannot express is in the generated
[capability matrix](docs/reference/capability-matrix.md). To get started, read the
[quickstart](docs/quickstart/install.md); there is a worked example for every plugin under
[`examples/`](examples).

## The `meridian` command

```
cargo build --workspace
meridian validate site.yaml                 # check documents, load no plugin
meridian plugins                            # what is installed, and what each can do
meridian compile site.yaml --out build      # write each target's own files; change nothing
meridian plan site.yaml                     # ask each tool what applying would change
meridian apply site.yaml                    # make it so
```

Plugins are found beside the `meridian` binary, or in the directories named by `--plugin-dir`
or `MERIDIAN_PLUGIN_DIR`. Each tool's working files and Meridian's own run-state live under
`--state-dir` (`.meridian` by default).

A workflow's Infrastructure documents run first, then its ResourceSets. A ResourceSet whose
`targetHost` comes from an Infrastructure output is compiled for, and applied to, whatever
that output turns out to be; until that infrastructure has been applied it is reported as
deferred. Values a tool marks sensitive are masked wherever Meridian prints.

## Layout

The core is a Cargo workspace under `crates/`:

- `meridian-ir` — parsing and validation of the documents
- `meridian-yaml` — YAML scalar rules and a deterministic writer, shared by the parser and the plugins that emit YAML
- `meridian-resolve` — hierarchical data resolution
- `meridian-graph` — dependency graph and topological sort
- `meridian-ast` — the target-agnostic resource tree every plugin consumes
- `meridian-value` — the dynamic value type and its canonical JSON form, shared by the core and the SDK
- `meridian-sdk` — what a plugin author builds against: the `Emitter` and `Runner` traits, `serve`,
  the wire conversion, and helpers for running a command-line tool
- `meridian-plugin-host` — launching, discovering and driving plugins
- `meridian-orchestrator` — runs a workflow: compiles each document, drives its tool through a
  plugin, hands outputs from one stage to the next, and stops on the first failure
- `meridian-cli` — the `meridian` command
- `meridian-target-fake` — a small, complete plugin whose tool is a directory, used by the tests
  and readable as a worked example of the SDK
- `meridian-plugin-testkit` — what the first-party plugins' tests share

The plugins are under `plugins/`, one crate each, built as a binary named
`meridian-target-<name>`. A plugin's library depends on `meridian-sdk` and nothing else of
Meridian's, which is the same position a third-party plugin is in.

A plugin is a separate process. The host launches the binary, reads one handshake line
saying where it is listening, checks the protocol version, and then talks to it over gRPC.
The protocol is defined in [`proto/`](proto/meridian/plugin/v1/plugin.proto), so a plugin
can be written in any language that can serve it. A plugin binary is not meant to be run
by hand: its stdout carries the protocol, and it exits when the host closes its stdin.

Every plugin compiles in four stages, in this order: transform (map each resource to the
tool's own vocabulary, or refuse it), sort (apply the tool's ordering semantics), validate
(the tool's own constraints) and emit (pure serialisation). Running the tool is a separate
half of the plugin, so compiling stays free of side effects.

## Tests

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 tools/check_links.py
```

Each plugin's tests compile the shared documents in `fixtures/` and its own examples, and
compare the output byte for byte with the golden files in `plugins/<name>/testdata`.
Refusals are golden files too, because the wording an operator reads is output like any
other. Set `UPDATE_GOLDEN=1` to record them after a deliberate change. The runners are
tested against stand-ins for the tools, so the suite needs nothing installed but Rust.
