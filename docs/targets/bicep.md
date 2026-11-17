# Target: `bicep`

<img src="../branding/logos/meridian-bicep.svg" width="72" alt="Meridian Bicep plugin logo">

Compiles an Infrastructure document into an Azure Bicep file and deploys it to a
resource group with the `az` command line.

## What it does and does not support

| | |
|---|---|
| Resource types | Azure's own, with the API version: `Namespace/type@api-version`, plus `deployment` for where the file is deployed |
| `dependsOn` | Yes, as Bicep's `dependsOn`, unless a reference already implies the order |
| `notifies` | No. An Azure deployment has no way to react to a change in another resource. Refused |
| `runtimeWhen` | No. Refused |
| `state: absent` | No. A resource group deployment never deletes a resource; remove it from the document and delete it in Azure |
| Plan | Yes: `what-if`. Where Azure cannot analyse a resource type, the plan is reported as unknown |
| Outputs | Yes, always as strings |

## What a document looks like

A resource's `type` is the Azure resource type with its API version, and its
`params` are the resource's body exactly as Azure documents it: `name`,
`location`, `sku`, `properties` and the rest. A type without its API version is
refused.

One resource **must** have the type `deployment`. It is not part of the Bicep
file. It says where the file goes: `resourceGroup`, and optionally `name` and
`subscription`.

A resource's id is its symbolic name in the Bicep file, so it may hold letters,
digits and underscores.

## References and expressions

| You write | It becomes |
|---|---|
| `${web_ip}` | the resource's symbolic name |
| `${web_ip.properties.ipAddress}` | a property read from it |
| either inside longer text | Bicep string interpolation |
| `${resourceGroup().location}`, `${uniqueString(...)}` | Bicep's own expression language, handed over as written |

Anything between `${` and `}` that is not a path into a resource of the document
is Bicep's. A function call that mentions a resource, such as
`${toUpper(store.name)}`, still orders the two.

## Example

[`examples/bicep/web-vm.yaml`](../../examples/bicep/web-vm.yaml) compiles to
[`web-vm.main_bicep`](../../plugins/bicep/testdata/web-vm.main_bicep). A part of it:

```yaml
- id: target
  type: deployment
  params:
    resourceGroup: web-prod

- id: web_ip
  type: Microsoft.Network/publicIPAddresses@2023-09-01
  params:
    name: web-ip
    location: ${resourceGroup().location}
    sku:
      name: Standard
    properties:
      publicIPAllocationMethod: Static
```

```bicep
resource web_ip 'Microsoft.Network/publicIPAddresses@2023-09-01' = {
  name: 'web-ip'
  location: resourceGroup().location
  properties: {
    publicIPAllocationMethod: 'Static'
  }
  sku: {
    name: 'Standard'
  }
}
```

## Running

| | |
|---|---|
| Plan | `az deployment group what-if --resource-group <rg> --name <name> --template-file main.bicep` |
| Apply | the same preview, so the changes can be reported, then `az deployment group create` |
| Outputs later | `az deployment group show` |
| `MERIDIAN_AZ` | the program to run, when it is not `az` on `PATH` |
| `MERIDIAN_AZ_ARGS` | arguments added to every call |

Credentials are the `az` command's own (`az login`). An output of a secure type
is reported as sensitive and never printed.

This plugin's runner has been tested against a stand-in for the `az` command. It
has not yet been run against a real Azure subscription.
