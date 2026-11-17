# Packer plugin

<img src="../branding/logos/meridian-packer.svg" width="96" alt="Meridian Packer plugin logo">

The Packer plugin compiles a Meridian `Infrastructure` document into a
[Packer](https://developer.hashicorp.com/packer) build template and runs it with `packer`.
Use it to build machine images and container images: an AMI, an Azure or Google Cloud
image, a VMware template, a Docker image.

| | |
| --- | --- |
| Target name | `packer` |
| Category | `infrastructure-as-code` (see [Why this category](#why-this-category)) |
| Operations | compile, plan, apply, outputs |
| Document kind | `Infrastructure` |
| Compiles to | `build.pkr.json`, plus `meridian-build.json` for Meridian's own use |
| Needs | `packer` 1.7 or later on the machine running Meridian |
| Binary | `meridian-target-packer` |

## Contents

- [A first build](#a-first-build)
- [How a document maps to a build](#how-a-document-maps-to-a-build)
- [Resource types](#resource-types)
- [References](#references)
- [Order](#order)
- [Outputs](#outputs)
- [Running a build](#running-a-build)
- [What the plugin refuses](#what-the-plugin-refuses)
- [Examples](#examples)
- [Why this category](#why-this-category)
- [Testing](#testing)

## A first build

This document needs no cloud account and no Packer plugin. The null builder builds
nothing, and every step runs on the machine running Packer.

```yaml
apiVersion: meridian/v1
kind: Infrastructure
metadata:
  name: hello
spec:
  target: packer
  resources:
    - id: greeting
      type: variable
      params:
        type: string
        default: hello

    - id: box
      type: source.null
      params:
        communicator: none

    - id: say
      type: provisioner.shell-local
      params:
        inline:
          - echo "${greeting} from ${source.name}"

  outputs:
    - name: builder
      value: ${box.builder_type}
```

```console
$ meridian plan hello.yaml
hello (Infrastructure, target packer)
  + create box: null image

$ meridian apply hello.yaml
  hello | ==> hello.null.box: Running local shell script: ...
  hello |     hello.null.box: hello from box
hello (Infrastructure, target packer)
  + create box: null image
  applied 1 change
  output builder = "null"
```

`meridian compile hello.yaml --out build/` writes the template without running anything,
so you can read exactly what Packer is given.

## How a document maps to a build

**One document is one Packer build.** The build is named after the document
(`metadata.name`) and builds every source the document declares. For a second build, write
a second document.

The plugin does not hide Packer's vocabulary behind a smaller one. Packer's builders,
provisioners and post-processors come from its own plugins and there are hundreds of them,
so after the prefix that says what part of a build a resource is, a resource's `type` is
Packer's type and its `params` are that type's arguments, exactly as its documentation
lists them.

What Meridian adds:

- **Order from the graph.** Provisioners run in the order `dependsOn` gives them, and
  post-processors that depend on one another become a sequence.
- **References by id.** `${region}` refers to the variable with the id `region`. The
  plugin writes Packer's own address for it and refuses a reference to something the
  document does not declare.
- **Outputs.** Packer has none. The plugin records what each source produced and reports
  the values the document asks for, so a later stage can use the id of the image.
- **A check before anything runs.** A document that Packer could not run the way it is
  written is refused at compile time, with the line of the document that is wrong.

The template is written in Packer's JSON syntax, not HCL. In HCL a nested block and a
map-valued argument are written differently, and only a Packer plugin's schema says which
one a param is. In the JSON syntax Packer decides that itself, so a document compiles
correctly without Meridian knowing any plugin's schema.

## Resource types

| `type` | Becomes | Notes |
| --- | --- | --- |
| `source.<builder>` | a `source` block, built by the build | `source.amazon-ebs`, `source.docker`, `source.azure-arm`, `source.null` and any other builder. At least one is required. |
| `provisioner.<type>` | a `provisioner` of the build | `provisioner.shell`, `provisioner.file`, `provisioner.ansible` and so on. |
| `post-processor.<type>` | a `post-processor` of the build | `post-processor.docker-tag`, `post-processor.compress` and so on. |
| `error-cleanup-provisioner.<type>` | the build's `error-cleanup-provisioner` | At most one. It runs only when the build fails. |
| `data.<type>` | a `data` block | `data.amazon-ami`, `data.http` and so on. |
| `variable` | a `variable` block | Params are the block's arguments: `type`, `default`, `description`, `sensitive`. |
| `local` | an entry of `locals` | Takes one param, `value`. |
| `plugin` | an entry of `required_plugins` | Takes `source` and `version`. The id is the plugin's name. `packer init` installs it. |

A resource's id is the block's name, so it must start with a letter or underscore and hold
only letters, digits, underscores and hyphens.

Arguments Packer accepts on every provisioner and post-processor, such as `only`, `except`,
`override`, `pause_before`, `max_retries`, `timeout` and `keep_input_artifact`, are params
like any other. In `only` and `except` a source can be named by its id alone: the plugin
writes `amazon-ebs.ubuntu` for `ubuntu`.

`state` must be `present` or left out, since a build only creates images. `notifies` and
`runtimeWhen` are refused: see [What the plugin refuses](#what-the-plugin-refuses).

## References

`${...}` in a param is a reference or an expression.

| You write | Packer is given | When |
| --- | --- | --- |
| `${region}` | `${var.region}` | `region` is a `variable` of the document |
| `${stamp}` | `${local.stamp}` | `stamp` is a `local` |
| `${base.id}` | `${data.amazon-ami.base.id}` | `base` is a `data.amazon-ami` |
| `${source.name}`, `${build.ID}`, `${path.root}`, `${packer.version}` | unchanged | Packer's own names: `var`, `local`, `data`, `source`, `build`, `path`, `packer` |
| `${formatdate("YYYYMMDD", timestamp())}` | unchanged | anything that is not a plain path is Packer's expression language |
| `$${HOME}` | `$${HOME}` (a literal `${HOME}`) | the escape for a literal `${`, in every Meridian document |

Two things follow from this:

- A plain path that names nothing, such as `${regoin}`, is refused at compile time. Without
  the check, the mistake would surface as a Packer error about a file you did not write.
- A shell script that uses `${HOME}` must write `$${HOME}`, because `${` starts a
  reference. `$HOME` and `$(date)` need no escape. Packer's older `{{ .Vars }}` templates
  are plain text to Meridian and pass through untouched.

A source, a provisioner, a post-processor and a plugin requirement have nothing to read
inside a param. Inside a provisioner, Packer's own `${source.name}` and `${build.ID}`
describe the source being built.

## Order

A Packer build has a fixed shape: install plugins, read variables, look up data, start
every source, run the provisioners, post-process the result. `dependsOn` works inside that
shape.

**Provisioners** run in dependency order. Where no dependency decides, they run in the
order they are written.

```yaml
- id: configure
  type: provisioner.shell
  params: {inline: ["sudo systemctl enable nginx"]}
  dependsOn:
    - resource: install      # declared below, runs first

- id: install
  type: provisioner.shell
  params: {inline: ["sudo apt-get install -y nginx"]}
```

**Post-processors** that depend on one another form a sequence, in which each takes the
artifact of the one before it. A post-processor with no such dependency starts a sequence
of its own and works on what the build itself produced.

```yaml
- id: tag
  type: post-processor.docker-tag
  params: {repository: registry.example.com/team/app, tags: ["1.0.0"]}

- id: push
  type: post-processor.docker-push
  params: {login: false}
  dependsOn:
    - resource: tag          # push takes the tagged image

- id: checksum               # a sequence of its own
  type: post-processor.checksum
  params: {checksum_types: ["sha256"]}
```

compiles to

```json
"post-processors": [
  { "post-processor": [ { "docker-tag": { "...": "..." } }, { "docker-push": { "login": false } } ] },
  { "post-processor": [ { "checksum": { "checksum_types": ["sha256"] } } ] },
  { "post-processor": [ { "manifest": { "output": "meridian-manifest.json", "strip_path": true } } ] }
]
```

The last sequence is the plugin's own: see [Outputs](#outputs).

A dependency that only restates the shape, such as a provisioner depending on a source, is
accepted and changes nothing. One that contradicts it is refused, because writing the
template anyway would run the steps in an order the document ruled out.

## Outputs

Every build the plugin compiles ends with Packer's built-in `manifest` post-processor,
which records what each source produced in `meridian-manifest.json` in the work directory.
A document's outputs read from it, as `${<source id>.<attribute>}`:

| Attribute | Value |
| --- | --- |
| `id` | The image's id. For a single-region image from an Amazon builder the region is removed: `ami-0abc1234`. Otherwise it is the artifact id unchanged. |
| `artifact_id` | The artifact's id exactly as Packer reports it, such as `us-east-1:ami-0abc1234`. |
| `builder_type` | The builder that made it, such as `amazon-ebs`. |
| `build_time` | When the build finished, in seconds since 1970. |
| `files` | The files of the artifact, each with its name and size, for a builder that produces files. |
| `packer_run_uuid` | The id of the `packer` run that made it. |

```yaml
outputs:
  - name: image_id
    value: ${ubuntu.id}
  - name: image_label
    value: web built from ${ubuntu.artifact_id}
```

An output that is nothing but one reference keeps the type of what it reads, so
`${ubuntu.build_time}` is a number. Anything else is text.

Outputs always describe the last build. The manifest keeps earlier runs too, and the
plugin ignores them. `meridian` reads the outputs back without building again whenever a
later stage needs them.

An output can only read what a build produced, so a reference to a variable, a local or a
data source in an output is refused.

## Running a build

| Command | What the plugin runs | What changes |
| --- | --- | --- |
| `meridian compile` | nothing | nothing |
| `meridian plan` | `packer init .`, `packer validate .` | nothing is built; plugins the template requires are installed |
| `meridian apply` | `packer init .`, `packer validate .`, `packer build -color=false .` | every source is built |

Packer has no plan of its own. It does not need one to be honest: a build never changes an
existing image, it always makes a new one. So a plan lists one creation per source, after
Packer itself has checked the template. A plan is therefore never "in sync", and applying
twice builds two images.

Packer runs in the document's work directory, which Meridian keeps under its state
directory (`.meridian/` in the directory `meridian` is run from, or `--state-dir`). A
relative path in a param, such as the `source` of a `provisioner.file`, is relative to
that work directory and not to the document, so use an absolute path.

**Variables.** Values handed over from an earlier stage reach Packer as `PKR_VAR_<name>`,
so a `variable` with the same id receives them. Packer ignores a value it has no variable
for. Variables can also be set the usual way, with `PKR_VAR_<name>` in the environment
Meridian runs in.

**Credentials.** The plugin passes its environment through to Packer. Sign in the way the
builder expects, for example `AWS_PROFILE` or `ARM_CLIENT_ID`. Do not write a secret into a
document: declare a `variable` with `sensitive: true` and set it from the environment.

**A different `packer`.** Set `MERIDIAN_PACKER` to the program to run when it is not
`packer` on `PATH`.

When Packer fails, Meridian reports which step failed, the command that was run, the exit
status and the end of what Packer printed.

## What the plugin refuses

Meridian never compiles a document into something that quietly does less than it says.
Each refusal names the resource, the rule and the line of the document.

| Rule | When |
| --- | --- |
| `unsupported-resource-type` | The type is not one of the [resource types](#resource-types), such as `package` or `aws_instance`. |
| `missing-source` | The document declares no `source.<builder>`. |
| `invalid-reference` | A `${...}` names something the document does not declare, reads a source or a step inside a param, is never closed, or is an output that does not read a source's attribute. |
| `unsupported-dependency` | A dependency contradicts the shape of a build: a provisioner depending on a post-processor, a source depending on a source, a post-processor depending on two others, two post-processors depending on the same one, a data source or local depending on another without referring to it, or anything depending on the error-cleanup provisioner. |
| `unsupported-action` | The document uses `notifies`. A build runs each step once on a machine it then discards, so nothing in it can react to a change. |
| `unsupported-runtime-condition` | The document uses `runtimeWhen`. To run a step for some sources only, use the step's `only` or `except` param. |
| `unsupported-state` | `state` is anything but `present`. A build never removes an image. |
| `missing-param`, `invalid-param` | A `plugin` without `source` and `version`, a `local` without `value`, or either with a param it does not take. |
| `duplicate-error-cleanup` | A second `error-cleanup-provisioner`. |
| `invalid-name` | A resource id that cannot be a Packer block name. |

## Examples

Complete documents are in [`examples/packer/`](../../examples/packer/), and what each
compiles to is in [`plugins/packer/testdata/`](../../plugins/packer/testdata/).

| Example | Shows |
| --- | --- |
| [`hello.yaml`](../../examples/packer/hello.yaml) | A build that runs anywhere: a variable, a local, ordered provisioners, a post-processor sequence, outputs. |
| [`web-image.yaml`](../../examples/packer/web-image.yaml) | An AWS machine image: a required plugin, a data source that finds the base image, four ordered provisioners, the image id as an output. |
| [`app-container.yaml`](../../examples/packer/app-container.yaml) | A container image: tag then push as one sequence, a checksum as another, a sensitive variable, an error-cleanup provisioner. |

### A machine image, start to finish

The heart of [`web-image.yaml`](../../examples/packer/web-image.yaml):

```yaml
- id: base
  type: data.amazon-ami
  params:
    region: ${region}
    most_recent: true
    owners: ["099720109477"]
    filters:
      name: ubuntu/images/hvm-ssd/ubuntu-jammy-22.04-amd64-server-*

- id: ubuntu
  type: source.amazon-ebs
  params:
    region: ${region}
    source_ami: ${base.id}
    instance_type: t3.small
    ssh_username: ubuntu
    ami_name: web-${stamp}

- id: install_nginx
  type: provisioner.shell
  params:
    inline:
      - sudo apt-get update
      - sudo apt-get install -y nginx
  dependsOn:
    - resource: wait_for_boot
```

becomes

```json
"data": {
  "amazon-ami": {
    "base": { "region": "${var.region}", "most_recent": true, "...": "..." }
  }
},
"source": {
  "amazon-ebs": {
    "ubuntu": {
      "ami_name": "web-${local.stamp}",
      "region": "${var.region}",
      "source_ami": "${data.amazon-ami.base.id}",
      "...": "..."
    }
  }
}
```

and after `meridian apply`:

```console
  output image_artifact = "us-east-1:ami-0abc1234"
  output image_id = "ami-0abc1234"
```

### One step for some sources only

```yaml
- id: ubuntu
  type: source.amazon-ebs
  params: {region: us-east-1, source_ami: ami-0abc1234, instance_type: t3.small, ssh_username: ubuntu, ami_name: web}

- id: app
  type: source.docker
  params: {image: debian:bookworm-slim, commit: true}

- id: cloud_init
  type: provisioner.shell
  params:
    inline: ["cloud-init status --wait"]
    only: [ubuntu]           # written to Packer as amazon-ebs.ubuntu
```

## Why this category

Every Meridian plugin declares a category, and the plugin protocol has four:
infrastructure as code, configuration management, containerization and container
orchestration. Packer builds images, which is none of them exactly.

It is declared as **infrastructure as code** because that is the closest fit:

- An image is created from nothing by a declarative definition, the way a Terraform
  resource is. Configuration management, by contrast, configures a machine that already
  exists and keeps existing.
- What a build produces is an id that the stage creating machines consumes, so it belongs
  in the same part of a workflow as the tools that provision infrastructure, ahead of them.
- It uses the `Infrastructure` document kind, which is the kind that has outputs.
- Packer builds containers too, but also AMIs and VM templates, so containerization would
  be wrong for most of what it does.

If the protocol gains an image-building category, this plugin should move to it. That is a
one-line change in the plugin.

## Testing

```console
$ cargo test -p meridian-target-packer
```

runs the unit tests, the compile tests with their golden files, the runner tests against a
stand-in for `packer`, and the tests that drive the real plugin binary across the process
boundary. None of them needs Packer.

```console
$ cargo test -p meridian-target-packer --test packer -- --ignored
```

runs the tests that need the real `packer`: its syntax check over every example, and a
whole build, a rejected template and a failing build with the null builder. The `packer`
workflow runs them on every pull request that touches the plugin. After changing what the
plugin emits, record the golden files again with `UPDATE_GOLDEN=1`.
