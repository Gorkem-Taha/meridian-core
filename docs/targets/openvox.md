# The OpenVox target

<img src="../branding/logos/meridian-openvox.svg" width="72" alt="Meridian OpenVox plugin logo">

`target: openvox` compiles a ResourceSet into a manifest and applies it with
[OpenVox](https://voxpupuli.org/openvox/), the community-maintained fork of Puppet.

| | |
|---|---|
| Category | configuration management |
| Compiles to | `manifests/init.pp`, `site.pp` |
| Plans with | `puppet apply --noop`, or `bolt apply --noop` for another host |
| Applies with | `puppet apply`, or `bolt apply` for another host |
| Outputs | none |
| Plugin binary | `meridian-target-openvox` |

## What this target does and does not support

Read this before the examples. Meridian never emits a manifest that quietly does less
than the document asked for, so everything in the second list is a compile-time refusal
with an explanation, not a surprise on the node.

It supports:

- The resource types `package`, `service`, `file`, `user`, `group` and `exec`.
- `dependsOn`, as a `require` on the dependent resource. The whole dependency graph
  survives into the manifest; nothing is flattened into a sequence.
- `notifies` with the actions `restart` and `refresh` on a service, and `run` on an exec.
  It is the language's own `notify`, which also orders the two resources.
- Planning, as the tool's own no-op run, and applying.

It does not support:

- `runtimeWhen`. A catalog is compiled before it reaches the node, so there is no
  apply-time conditional to compile it into. Use a compile-time `when` instead.
- `notifies` with `reload`, `start` or `stop` on a service. A refresh restarts a
  service and takes no argument, and compiling a reload into a restart would do more than
  you asked for.
- Notifying a `package`, `file`, `user` or `group`. Those types do nothing on a refresh.
- `state: restarted` on a service. It is not a state a node can be left in; notify the
  service instead.
- Two resources of one type that manage the same thing, such as two `package` resources
  named `nginx`. The tool only finds that out on the node, so Meridian refuses it first.
- A document name that cannot be a class name. Use lower-case letters, digits and
  underscores.
- Outputs. OpenVox has nothing to report to a later stage.

One behaviour to know about: an `exec` runs on every apply unless it has `creates`.
Notifying it with `run` makes it run once more when the notifier changes; it does not
make it run only then.

## How it relates to the Puppet target

OpenVox reads the language Puppet reads and ships the same `puppet` command, so the two
targets share one compiler. What `openvox` compiles is byte for byte what `puppet`
compiles, and a test in this repository holds them to that. Switching a document between
the two is a one-word change to `spec.target`.

The difference is in which product runs the manifest:

| | `puppet` | `openvox` |
|---|---|---|
| Tool it expects | Puppet | OpenVox |
| Finds the tool | on `PATH` | on `PATH`, then in `/opt/puppetlabs/bin` where the packages install it |
| Remote transport | Bolt | OpenBolt |
| Settings | `MERIDIAN_PUPPET…` | `MERIDIAN_OPENVOX…` |
| Checks which product it is running | no | yes, see [Making sure it is OpenVox](#making-sure-it-is-openvox) |

Because the settings are separate, one machine can drive both.

## Install

You need OpenVox on the machine that applies the manifest.

- To apply on the machine Meridian runs on, install OpenVox there. The `openvox-agent`
  package from the OpenVox project is the usual way. `gem install openvox` also works and
  needs no root, which is how this repository's CI does it.
- To apply to another host, install OpenBolt on the machine Meridian runs on, and have an
  OpenVox agent package on the node. The catalog is compiled locally and applied by the
  node's agent over SSH or WinRM.

Then put `meridian-target-openvox` beside the `meridian` binary, or in a directory named
by `--plugin-dir` or `MERIDIAN_PLUGIN_DIR`, and check that Meridian sees it:

```
$ meridian plugins
openvox
  category:  configuration-management
  can:       compile, plan, apply
  resources: exec, file, group, package, service, user
```

## Try it on your own machine

[`examples/openvox/local.yaml`](../examples/openvox/local.yaml) manages two files and a
command under `/tmp`, so it needs no root:

```yaml
apiVersion: meridian/v1
kind: ResourceSet
metadata:
  name: hello
spec:
  target: openvox
  targetHost:
    from: inline
    host: localhost
  resources:
    - id: hello_dir
      type: file
      state: directory
      params:
        path: /tmp/meridian-hello
    - id: hello_conf
      type: file
      params:
        path: /tmp/meridian-hello/hello.conf
        content: "greeting = hello\n"
        mode: "0644"
      dependsOn:
        - resource: hello_dir
    - id: hello_stamp
      type: exec
      params:
        command: /bin/sh -c 'date > /tmp/meridian-hello/first-applied'
        creates: /tmp/meridian-hello/first-applied
      dependsOn:
        - resource: hello_conf
```

Ask what applying it would change. Nothing is touched:

```
$ meridian plan docs/examples/openvox/local.yaml
  hello | Notice: /Stage[main]/Meridian_hello/File[hello_dir]/ensure: created (noop)
  hello | Notice: /Stage[main]/Meridian_hello/File[hello_conf]/ensure: defined content as '{sha256}6783…' (noop)
  hello | Notice: /Stage[main]/Meridian_hello/Exec[hello_stamp]/returns: executed successfully (noop)
hello (ResourceSet, target openvox on localhost)
  + create hello_dir: ensure: created (noop)
  + create hello_conf: ensure: defined content as '{sha256}6783…' (noop)
  ~ update hello_stamp: returns: executed successfully (noop)
  3 changes to make
```

The lines with a bar are OpenVox's own output, relayed as it runs. The summary below
them is Meridian's, with each change traced back to the `id` you wrote.

Apply it, then plan again:

```
$ meridian apply docs/examples/openvox/local.yaml
hello (ResourceSet, target openvox on localhost)
  + create hello_dir: ensure: created
  + create hello_conf: ensure: defined content as '{sha256}6783…'
  ~ update hello_stamp: returns: executed successfully
  applied 3 changes

$ meridian plan docs/examples/openvox/local.yaml
hello (ResourceSet, target openvox on localhost)
  in sync, nothing to change
```

## A fuller example

[`examples/openvox/web.yaml`](../examples/openvox/web.yaml) sets up a small web
application on another host: a system user and group, a directory they own, nginx, a
site configuration that restarts nginx when it changes, and a command that runs once.

The part worth reading is how ordering and notification are written:

```yaml
    - id: site_conf
      type: file
      params:
        path: /etc/nginx/conf.d/app.conf
        mode: "0644"
        content: |
          server {
            listen 80;
            root /srv/app/public;
          }
      dependsOn:
        - resource: nginx_pkg
      notifies:
        - resource: nginx_svc
          action: restart
    - id: nginx_svc
      type: service
      state: running
      params:
        name: nginx
        enabled: true
      dependsOn:
        - resource: nginx_pkg
```

`meridian compile docs/examples/openvox/web.yaml --out build` writes
`build/web/manifests/init.pp`, where those two resources become:

```puppet
  file { 'site_conf':
    ensure  => file,
    content => "server {\n  listen 80;\n  root /srv/app/public;\n}\n",
    mode    => '0644',
    path    => '/etc/nginx/conf.d/app.conf',
    require => Package['nginx_pkg'],
    notify  => Service['nginx_svc'],
  }

  service { 'nginx_svc':
    ensure  => running,
    enable  => true,
    name    => 'nginx',
    require => Package['nginx_pkg'],
  }
```

and `build/web/site.pp`, which assigns the class to the host:

```puppet
node 'web01.example.com' {
  include meridian_web
}
```

Every resource's title is the `id` you gave it, which is how a line of the tool's output
is traced back to your document. The thing being managed is always spelled out in its
own attribute (`name`, `path` or `command`).

The compiled files are ordinary manifests. You can read them, review them in a pull
request, or hand `manifests/init.pp` to an OpenVox server as a module's class without
Meridian being involved in the run at all.

## Resource reference

Anything not listed is refused at compile time, naming the resource and what is accepted.

### package

| | |
|---|---|
| States | `present` (default), `latest`, `absent` |
| Params | `name` (required) |

### service

| | |
|---|---|
| States | `running` (default), `started` (the same), `stopped` |
| Params | `name` (required), `enabled` |
| Notify actions | `restart`, `refresh` |

### file

| | |
|---|---|
| States | `present` (default) and `file`, `directory`, `absent` |
| Params | `path` (required), `content`, `owner`, `group`, `mode` |

Quote `mode` (`"0644"`) so YAML does not read it as a number.

### user

| | |
|---|---|
| States | `present` (default), `absent` |
| Params | `name` (required), `uid`, `groups`, `home`, `shell`, `system` |

### group

| | |
|---|---|
| States | `present` (default), `absent` |
| Params | `name` (required), `gid`, `system` |

### exec

| | |
|---|---|
| States | none |
| Params | `command` (required), `creates`, `chdir`, `shell` |
| Notify actions | `run` |

Give the command's full path, or set `shell: true` to run it through a shell. Use
`creates` to make it run only until the file it names exists.

## How a run works

Meridian compiles the document, writes the files into its state directory, and runs the
tool there. Which tool depends on the document's host:

- `localhost`, `127.0.0.1` or `::1`: `puppet apply`, on the machine Meridian runs on.
- Anything else: `bolt apply --targets <host>`, which compiles the catalog locally and
  applies it on the node.

A plan is the same run with `--noop`. OpenVox works out every change and makes none.

Meridian reads the result from the tool itself. Locally, an exit status of 0 or 2 is
success (2 means changes were made) and 4 or 6 means a resource failed. When an apply
fails part way, Meridian reports the failure with the tool's own explanation, lists what
had already been changed, and exits non-zero. Resources that depended on the failed one
are skipped by OpenVox and are not reported as changed.

### Settings

All of them are environment variables read by the plugin, and all are optional.

| Variable | Meaning | Default |
|---|---|---|
| `MERIDIAN_OPENVOX` | The OpenVox `puppet` program | `puppet` on `PATH`, then `/opt/puppetlabs/bin/puppet` |
| `MERIDIAN_OPENBOLT` | The OpenBolt `bolt` program | `bolt` on `PATH`, then `/opt/puppetlabs/bin/bolt`, then `/opt/puppetlabs/bolt/bin/bolt` |
| `MERIDIAN_OPENVOX_TRANSPORT` | `local` or `bolt`, to override the choice made from the host | chosen from the host |
| `MERIDIAN_OPENVOX_ARGS` | Extra arguments for every run, split on spaces | none |
| `MERIDIAN_OPENVOX_VERIFY` | `warn`, `strict` or `off`, see below | `warn` |

For example, to apply as root on the node and against a particular environment:

```
MERIDIAN_OPENVOX_ARGS="--run-as root" meridian apply site.yaml              # over OpenBolt
MERIDIAN_OPENVOX_ARGS="--environment staging" meridian apply site.yaml      # locally
```

`MERIDIAN_OPENVOX_TRANSPORT=local` is for running Meridian on the node itself when the
document names that node by its real host name.

### Making sure it is OpenVox

Puppet and OpenVox both install a command called `puppet`, so "is `puppet` installed"
does not tell you which one you have. Before a local run the plugin asks the tool for its
`implementation` fact, which OpenVox reports as `openvox`.

| `MERIDIAN_OPENVOX_VERIFY` | If the tool is not OpenVox |
|---|---|
| `warn` (default) | Says so in the run's output, and runs it anyway. The manifest means the same to both. |
| `strict` | Refuses before anything is written or applied. Use it where running the other product is a licensing or policy problem. |
| `off` | Does not ask. Saves under a second per session. |

The check covers local runs. Over OpenBolt the catalog is applied by whatever agent is
installed on the node, and the plugin does not connect to the node just to ask.

## Troubleshooting

**`could not run puppet: it is not installed, or not on the PATH`**
OpenVox is not installed where Meridian runs, or is somewhere the plugin does not look.
Set `MERIDIAN_OPENVOX` to the full path of its `puppet` command.

**`… is not OpenVox: it has no implementation fact`**
The `puppet` that was found is Puppet, or an OpenVox release from before the fact
existed. Point `MERIDIAN_OPENVOX` at OpenVox's command, or see the table above.

**`puppet apply reported a failed resource`**
OpenVox ran and a resource failed. The tool's own error follows the message. Fix the
cause on the node or in the document and apply again; what already succeeded will not be
redone.

**`bolt apply failed`**
Usually the node could not be reached, or has no agent installed. The output that follows
is OpenBolt's own.

**A plan always shows an `exec` as a change**
It has no `creates`, so it runs on every apply. That is what the document says; add
`creates` if the command should run once.

## How this target is tested

- Golden files: every shared fixture and both documented examples are compiled and
  compared with recorded output in `plugins/openvox/testdata`, and with what the Puppet
  target emits.
- The runner is tested against stand-ins for `puppet` and `bolt`, so it runs anywhere.
- The plugin binary is started as a real process and driven over the plugin protocol.
- In CI the same plugin is run against a real OpenVox: every compiled manifest is
  parsed by it, and manifests are planned, applied, changed, drifted and removed on disk.

To run the last group yourself:

```
gem install openvox
MERIDIAN_TEST_OPENVOX=1 cargo test -p meridian-target-openvox --test real
```
