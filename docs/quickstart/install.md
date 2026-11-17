# Install

Meridian is built from source for now. There are no released binaries yet.

## Build

You need a stable Rust toolchain.

```
git clone https://github.com/ZanattaMichael/meridian-core
cd meridian-core
cargo build --workspace --release
```

That produces the `meridian` command and one binary per plugin, all in
`target/release`:

```
meridian
meridian-target-ansible
meridian-target-cloud-init
meridian-target-terraform
meridian-target-opentofu
meridian-target-pulumi
meridian-target-helm
meridian-target-packer
...
```

`meridian` finds plugins in the directory it sits in, so the build output works
as it stands. To keep plugins somewhere else, name the directories with
`--plugin-dir` (it can be given more than once) or `MERIDIAN_PLUGIN_DIR`
(separated by `:`).

## Check what is installed

```
meridian plugins
```

lists every plugin Meridian can find, with its category, what it can do
(compile, plan, apply, outputs) and the resource types it takes.

## The tools themselves

Meridian compiles a document into a tool's own files, and then runs that tool.
Compiling needs nothing but Meridian. Planning and applying need the tool
installed on the machine Meridian runs on, exactly as if you were running it by
hand:

| Target | Needs |
|---|---|
| `ansible` | `ansible-playbook` |
| `puppet` | `puppet`, or `bolt` to reach another host |
| `chef` | `chef-client`, or `chef-run` to reach another host |
| `salt` | `salt-call`, or `salt-ssh` to reach another host |
| `dsc` | PowerShell (`pwsh`) with the Dsc.PipelineRunner module |
| `cloud-init` | `cloud-init`, to check the user-data (Linux only; the check can be turned off). `ssh` as well, if an apply is to wait for the machine's first boot |
| `terraform` | `terraform` (or `tofu`) |
| `opentofu` | `tofu`, version 1.8 or later |
| `cloudformation` | the `aws` command line |
| `bicep` | the `az` command line |
| `packer` | `packer` 1.7 or later |
| `pulumi` | `pulumi` |
| `docker` | `docker` with the Compose plugin |
| `podman` | `podman` 4.4 or later on a Linux host with systemd |
| `kubernetes` | `kubectl` |
| `helm` | `helm`; `kustomize` when a document has an overlay; `kubectl` when an output reads from the cluster |

Two targets can run `tofu`. Pick `opentofu` when OpenTofu is the tool you use: it writes
`main.tofu.json`, which only OpenTofu reads, and it gives you OpenTofu's own features,
such as state and plan encryption, variables, local values and ephemeral resources. Pick
`terraform` when the same document must also run with `terraform`; it writes
`main.tf.json` and can be pointed at `tofu` with `MERIDIAN_TERRAFORM`. The
[OpenTofu page](../targets/opentofu.md#opentofu-or-terraform) compares the two.

Each tool keeps using its own credentials and configuration. Meridian does not
store or pass any: if `aws sts get-caller-identity` or `kubectl get nodes` works
in your shell, the plugin will work too. Each [target's page](../targets/ansible.md)
lists the environment variables that point a plugin at a different binary or add
arguments to every call.

Next: [your first ResourceSet](first-resourceset.md).
