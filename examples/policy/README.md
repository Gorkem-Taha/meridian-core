# An example policy

[`meridian.rego`](meridian.rego) is a policy for Open Policy Agent that
`meridian plan` and `meridian apply` can check every stage against:

```sh
meridian plan  site.yaml --policy examples/policy
meridian apply site.yaml --policy examples/policy
```

It needs `opa` on the `PATH` (or named in `MERIDIAN_OPA`) and the
`meridian-policy-opa` program beside `meridian`.

| Rule | Says |
|---|---|
| `deny` | nothing in a stage labelled `env: prod` is deleted or replaced |
| `deny` | a preview the tool could not complete is not applied to `env: prod` |
| `deny` | no `file` resource has mode `0666` or `0777` |
| `warn` | a stage has no `env` label |

How policies are asked, what `input` holds and how to write an engine for
another policy language are in
[the policy reference](../../docs/reference/policy.md).
