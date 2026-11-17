# Target: `dsc`

<img src="../branding/logos/meridian-dsc.svg" width="72" alt="Meridian DSC plugin logo">

Compiles a ResourceSet into a DscPipelineRunner configuration and runs it with
`Invoke-DscRunner`.

DscPipelineRunner drives both generations of DSC through one configuration
format: DSC v2 through `Invoke-DscResource`, and DSC v3 through the `dsc`
command line. So this one target covers both, and which engine evaluates the
resources is a setting.

## What it does and does not support

| | |
|---|---|
| Resource types | DSC's own: `Module/Resource`, such as `PSDscResources/Service` or `Microsoft.Windows/Registry`, plus `runner` for how the configuration is run. The abstract types (`package`, `service`, ...) are not taken |
| `dependsOn` | Yes, as the runner's own `dependsOn` |
| `notifies` | Native: the runner's `notify`. The action is `refresh`: when the notifier really changed, the notified resource's `Set` runs even if its `Test` says there is nothing to do |
| `runtimeWhen` | Yes: a `preCondition` |
| `state` | No. A DSC resource carries its desired state in its own properties (`Ensure`, `_exist`), so set it in `params` |
| Plan | Yes: the runner's `Test` mode |
| Outputs | No |

## What a document looks like

A resource's `type` is the DSC resource as the runner names it and its `params`
are the resource's properties, exactly as the resource documents them. Which
module a resource lives in, and whether it is a v2 or a v3 resource, is DSC's
business: `Module/Resource` is the same shape for both, and only the engine can
tell whether a type exists.

One resource may have the type `runner`. It is not a DSC resource:

| Param | Meaning |
|---|---|
| `engine` | `DscV2`, `DscV3`, or `Auto` (the default: v3 when `dsc` is installed) |
| `target` | `Local`, `WinRM` or `SSH`: how the runner reaches the document's host |
| `credential` | the name of a credential the runner resolves itself. The secret never appears in the document |

### The version of DSC

The plugin declares the versions `2` and `3`, and a document's
[`targetVersion`](../reference/dsl/index.md#targetversion) chooses the engine:

| `targetVersion` | Engine |
|---|---|
| not set | the runner's `engine`, or `Auto` |
| `"2"` (or `"2.x"`) | `DscV2` |
| `"3"` (or `"3.x"`) | `DscV3` |

```yaml
spec:
  target: dsc
  targetVersion: "3"
```

Say it once. A document whose `targetVersion` means one engine and whose
`runner` names another (including `Auto`) is refused with the rule
`conflicting-engine`; it is not resolved in favour of either. Any other version
is refused by Meridian before the plugin is asked.

When the document's host is another machine, `target` must be given. The runner
cannot guess between WinRM and SSH, and Meridian will not guess for it.

## References

`${id.Property}` reads a property of another resource as DSC reports it, once
that resource has run. It becomes the runner's `reference()` accessor, and the
resource that reads is ordered after the one it reads.

The runner treats every property as a PowerShell expandable string. A `$` or a
backtick you write in a param is escaped, so it reaches the resource as written.

## Conditions

`runtimeWhen` is a `preCondition`: a PowerShell predicate in the runner's
function language, evaluated just before the resource.

```yaml
runtimeWhen: "equals (nodeName) 'web'"
```

## Example

[`examples/dsc/web-server.yaml`](../../examples/dsc/web-server.yaml) compiles to a
one-node project:
[`Datum.yml`](../../plugins/dsc/testdata/web-server.Datum_yml) and
[`Projects/Present/web.yml`](../../plugins/dsc/testdata/web-server.Projects_Present_web_yml).
One resource of it:

```yaml
- id: site_config
  type: PSDesiredStateConfiguration/File
  params:
    DestinationPath: C:\inetpub\app\web.config
    Contents: <configuration><system.webServer/></configuration>
    Ensure: Present
  dependsOn:
    - resource: site_root
  notifies:
    - resource: w3svc
      action: refresh
```

```yaml
- name: site_config
  type: PSDesiredStateConfiguration/File
  dependsOn:
    - PSDesiredStateConfiguration/File/site_root
  notify:
    - PSDscResources/Service/w3svc
  target:
    action: WinRM
    computerName: web01.example.com
    credentialRef: svcDeploy
  properties:
    Contents: <configuration><system.webServer/></configuration>
    DestinationPath: C:\inetpub\app\web.config
    Ensure: Present
```

`Invoke-DscRunner` takes a Datum project and compiles it before running
anything. Meridian has already done the resolving Datum is for, so the project
it writes has one layer and one node.

## Running

| | |
|---|---|
| Plan | `Invoke-DscRunner -Mode Test`: every resource is tested, none is set. A resource that is not in its desired state is a change |
| Apply | a `Test` to learn what has drifted, then `Invoke-DscRunner -Mode Set` |
| `MERIDIAN_PWSH` | the PowerShell to run, when it is not `pwsh` on `PATH` |
| `MERIDIAN_DSC_MODULE` | the module to import: a name PowerShell can find, or the path of its manifest. `Dsc.PipelineRunner` by default |

In `Test` mode the runner records both drift and a resource that could not be
tested as a failure, so a plan lists either as a change, with the runner's
message.

The runner is tested against a real PowerShell, the DscPipelineRunner module
from the PowerShell Gallery and DSC v3 on Linux, on every pull request that
touches it. On Windows the same compiled projects are run through both DSC v2
(`PSDscResources`) and DSC v3 (`Microsoft.Windows/Registry`) and the machine's
environment and registry are checked afterwards. The plugin's own Rust runner
has not yet run on Windows, and reaching another machine over WinRM or SSH is
not tested for real. See [Real-tool tests](../contributing/real-tool-tests.md#dsc).
