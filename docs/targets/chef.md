# Target: `chef`

<img src="../branding/logos/meridian-chef.svg" width="72" alt="Meridian Chef plugin logo">

Compiles a ResourceSet into a Chef cookbook and converges it with `chef-client`
in local mode, or with `chef-run` for another machine.

## What it does and does not support

| | |
|---|---|
| Resource types | `package`, `file`, `service`, `user`, `group`, `exec` ([reference](../reference/resource-types.md)) |
| `dependsOn` | Yes, as recipe order. A recipe runs top to bottom |
| `notifies` | Native: `notifies`, delayed to the end of the run. Actions `restart`, `reload`, `start`, `stop` on a service, `run` on an exec. The action is kept: a reload stays a reload |
| `runtimeWhen` | Yes: an `only_if` guard |
| Plan | On this machine, yes: `--why-run`. For another machine, no: `chef-run` has no dry run, so the plan is reported as unknown |
| Outputs | No |

## What a document compiles to

A cookbook named `meridian_<document name>`:

```
cookbooks/meridian_web/metadata.rb
cookbooks/meridian_web/recipes/default.rb
```

The recipe has one block a resource, named by its id. A file whose state is
`directory` becomes Chef's `directory` resource. A service's `enabled` becomes
an `:enable` or `:disable` action in front of `:start` or `:stop`. A user's
`groups` become one `group` block each, with `append true`, because Chef's
`user` has a primary group only. A `group` resource is written with
`append true` as well: a document declares the group, not its whole
membership, and without it Chef would remove every member on each converge.

`runtimeWhen` is the Ruby an `only_if` guard holds, so it can read node
attributes:

```yaml
runtimeWhen: "node['platform_family'] == 'debian'"
```

Two resources that manage the same package, service, file, user or group are
refused at compile time.

## Example

[`examples/chef/web-server.yaml`](../../examples/chef/web-server.yaml) compiles to
([the whole recipe](../../plugins/chef/testdata/web-server.cookbooks_meridian_web_recipes_default_rb)):

```ruby
file 'nginx_conf' do
  content "server {\n  listen 80;\n  root /srv/www;\n}\n"
  mode '0644'
  owner 'root'
  path '/etc/nginx/conf.d/site.conf'
  notifies :reload, 'service[nginx_svc]', :delayed
  action :create
end

service 'nginx_svc' do
  service_name 'nginx'
  action [:enable, :start]
end

execute 'open_firewall' do
  command 'ufw allow 80/tcp'
  only_if { node['platform_family'] == 'debian' }
  action :run
end
```

## Running

Chosen by the document's host:

| Host | Tool | Plan | Apply |
|---|---|---|---|
| this machine (`localhost`) | `chef-client` | `chef-client --local-mode --override-runlist 'recipe[meridian_web]' --why-run` | the same without `--why-run` |
| anything else | `chef-run` | not available | `chef-run <host> cookbooks/meridian_web` |

| Variable | Meaning |
|---|---|
| `MERIDIAN_CHEF_CLIENT` | the client to run, when `chef-client` is not on `PATH`. Cinc's `cinc-client` takes the same arguments |
| `MERIDIAN_CHEF_RUN` | the program to run for another machine |
| `MERIDIAN_CHEF_TRANSPORT` | `local` or `remote`, to override the choice made from the host |
| `MERIDIAN_CHEF_ARGS` | arguments added to every run, such as `--chef-license accept` |

Changes are read from chef-client's report. `chef-run` does not say which
resources it changed, so an apply through it reports success without a list.

The runner is tested against a real Chef client (Cinc Client 18, the community
build of Chef Infra Client) in local mode on every pull request that touches
it: a why-run plan, a converge checked on disk, a second plan that is in sync,
a notification, a `runtimeWhen` guard, a failing command, and, as root, a
package, a group, a user and a service. Running against a Chef server, and
`knife` and `chef-run` reaching another machine, have only met stand-ins. See
[Real-tool tests](../contributing/real-tool-tests.md).
