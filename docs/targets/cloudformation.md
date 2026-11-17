# Target: `cloudformation`

<img src="../branding/logos/meridian-cloudformation.svg" width="72" alt="Meridian CloudFormation plugin logo">

Compiles an Infrastructure document into an AWS CloudFormation template and
deploys it as a stack with the `aws` command line.

## What it does and does not support

| | |
|---|---|
| Resource types | CloudFormation's own: any `Vendor::Service::Type` or `Custom::Name`, plus `stack` for the deployment's settings |
| `dependsOn` | Yes, as `DependsOn`, unless a reference already implies the order |
| `notifies` | No. CloudFormation has no way to react to a change in another resource. Refused |
| `runtimeWhen` | No. Refused |
| `state: absent` | No. Remove the resource from the document; CloudFormation deletes what the template no longer declares, subject to its `DeletionPolicy` |
| Plan | Yes: a change set, which is then discarded |
| Outputs | Yes: the template's `Outputs` |

## What a document looks like

A resource's `type` is the CloudFormation resource type and its `params` are
that type's properties, exactly as AWS documents them. A few keys are resource
attributes and not properties, and are written beside `Properties`:
`DeletionPolicy`, `UpdateReplacePolicy`, `CreationPolicy`, `UpdatePolicy`,
`Condition` and `Metadata`.

One resource may have the type `stack`. It is not part of the template. It holds
the stack's `name`, its `region`, and the `capabilities` the deployment
acknowledges.

Logical ids hold only letters and digits, so a resource id such as `web_vm` is
written `WebVm` in the template. Everything Meridian reports uses the id you
wrote.

## References

| You write | It becomes |
|---|---|
| `${web_vm}` | `Ref` |
| `${web_vm.PublicIp}` | `Fn::GetAtt` |
| either inside longer text | `Fn::Sub` |
| `${AWS::Region}`, `${AWS::StackName}` | CloudFormation's own pseudo parameters, through `Fn::Sub` |

## Example

[`examples/cloudformation/web-vm.yaml`](../../examples/cloudformation/web-vm.yaml)
compiles to
[`web-vm.template_yaml`](../../plugins/cloudformation/testdata/web-vm.template_yaml).
A part of it:

```yaml
- id: web_vm
  type: AWS::EC2::Instance
  params:
    ImageId: ami-0c7217cdde317cfec
    InstanceType: t3.small
    SecurityGroupIds: ["${web_sg.GroupId}"]
    DeletionPolicy: Retain
  dependsOn:
    - resource: web_role
```

```yaml
WebVm:
  Type: AWS::EC2::Instance
  DependsOn:
    - WebRole
  DeletionPolicy: Retain
  Properties:
    ImageId: ami-0c7217cdde317cfec
    InstanceType: t3.small
    SecurityGroupIds:
      - Fn::GetAtt:
          - WebSg
          - GroupId
```

## Running

Both plan and apply go through a change set, so what is applied is what was
previewed.

| | |
|---|---|
| Plan | create a change set, wait for it, describe it, then delete it. For a stack that does not exist yet, the empty stack the change set created is removed too |
| Apply | create and describe the change set, execute it, wait for the stack to finish, then read the stack's outputs |
| `MERIDIAN_AWS` | the program to run, when it is not `aws` on `PATH` |
| `MERIDIAN_AWS_ARGS` | arguments added to every call, such as `--profile prod` |

Credentials are the `aws` command's own.

The runner is tested with the real `aws` command against
[LocalStack](https://github.com/localstack/localstack), an emulator of AWS, on
every pull request that touches it. It has not been run against a real AWS
account. [Real-tool tests](../contributing/real-tool-tests.md#cloudformation) says what the emulator proves,
what it cannot, and how to run the same tests against an account.
