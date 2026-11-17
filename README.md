# meridian-core
A unified configuration DSL that compiles to Ansible, Puppet, Chef, DSC, Terraform, and ARM — write once, target any configuration management or infrastructure-as-code backend.

## Status

Early implementation. The accepted specifications live in [`docs/`](docs/index.md) —
start with the [design plan](docs/plans/design-plan.md).

Milestones 1 to 3 of the [build sequence](docs/plans/design-plan.md#12-build-sequencing-milestones)
are in place, along with the first half of milestone 4:

- `internal/ir` — parsing and validation
- `internal/resolve` — hierarchical data resolution
- `internal/graph` — dependency graph and topological sort
- `internal/ast` — the target-agnostic resource tree every emitter consumes
- `pkg/sdk` — the public contract a target implements, and the server side of the plugin protocol
- `internal/plugin` — the host side: launching a plugin binary, the handshake, and the target registry
- `internal/fixtures` — the documents every target's tests compile, shared so their outputs can be read side by side
- `plugins/ansible` — the first target, built against the SDK alone and shipped as its own binary
- `plugins/puppet` — the second target, written against the SDK from its first line

A target is a separate process. The host launches the binary, reads one handshake line
saying where it is listening, checks the protocol version, and then talks to it over gRPC
as though it were a local emitter. Nothing above `internal/plugin` can tell a subprocess
target from a compiled-in one.

The two targets exist to prove the tree is target-agnostic. Both compile the same
documents from `internal/fixtures`, so their golden files differ only where the targets
themselves do. Ansible has no graph, so its sort stage flattens the tree into one task
list and warns about the parallelism that cost; Puppet's catalog is a graph, so the same
edges survive as `require` metaparameters and nothing is warned about. Where a target
cannot express what a document asks for, it refuses rather than emitting output that does
less: Puppet has no apply-time conditional and its refresh only restarts, so a document
carrying `runtimeWhen` or notifying a reload fails to compile with an explanation. Those
refusals are recorded as golden files too, because the wording an operator reads is
output like any other.

There is no CLI and no runner yet.

### Moving to Rust

Everything described above is written in Go. The core is being rewritten in Rust; the
decision and the order of work are in the
[design plan](docs/plans/design-plan.md#121-rust-migration-sequencing). The plugin protocol
does not change, so the Go Ansible and Puppet plugins keep working behind a Rust host, and
the Go core stays in the repository until the Rust core reproduces its golden files.

The Rust core is a Cargo workspace under `crates/`. Ported so far:

- `crates/meridian-ir` — parsing and validation, the counterpart of `internal/ir`
- `crates/meridian-resolve` — hierarchical data resolution, the counterpart of `internal/resolve`
- `crates/meridian-graph` — dependency graph and topological sort, the counterpart of `internal/graph`
- `crates/meridian-ast` — the target-agnostic resource tree, the counterpart of `internal/ast`
- `crates/meridian-value` — the dynamic value type and its canonical JSON form, shared by the core and the SDK
- `crates/meridian-sdk` — what a plugin author builds against: the `Emitter` and `Runner` traits,
  `serve`, and the wire conversion. The counterpart of `pkg/sdk`
- `crates/meridian-plugin-host` — launching, discovering and driving target plugins, the counterpart of `internal/plugin`
- `crates/meridian-orchestrator` — runs a workflow: compiles each document, drives its tool through a
  plugin, hands outputs from one stage to the next, and stops on the first failure
- `crates/meridian-cli` — the `meridian` command
- `crates/meridian-target-fake` — a small, complete plugin whose tool is a directory, used by the tests
  and readable as a worked example of the SDK

That is every layer the Go core has. The Rust host drives the Go Ansible and Puppet plugin
binaries and reproduces every golden file under `plugins/*/testdata` byte for byte; the
test that proves it (`crates/meridian-plugin-host/tests/contract.rs`) compiles the documents
in `fixtures/` and needs a Go toolchain to build the plugins.

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

### The `meridian` command

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

### Building and running the target plugins

```
go build -o meridian-target-ansible ./plugins/ansible/cmd/meridian-target-ansible
go build -o meridian-target-puppet  ./plugins/puppet/cmd/meridian-target-puppet
```

The binary is not meant to be run by hand: its stdout carries the protocol, and it exits
when the host closes its stdin. Drop it in a directory and a host discovers it by name.

### Regenerating the protocol

The generated protocol code under `pkg/sdk/pluginpb` is committed, so building this
repository never requires protoc. Regenerate it only when `proto/meridian/plugin/v1/plugin.proto`
changes, with protoc 29.3 and:

```
go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.36.6
go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.5.1
protoc --proto_path=proto \
  --go_out=. --go_opt=module=github.com/ZanattaMichael/meridian-core \
  --go-grpc_out=. --go-grpc_opt=module=github.com/ZanattaMichael/meridian-core \
  proto/meridian/plugin/v1/plugin.proto
```

### Tests

```
go build ./...
go test ./...          # includes the contract test, which builds the plugin binaries
go test -short ./...   # unit tests only
```
