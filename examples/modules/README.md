# Modules

A project that takes part of its configuration from a module.

```
modules/
  hardening/              the module: a directory with a meridian.yaml
    meridian.yaml
    documents/ssh.yaml    a ResourceSet anyone can apply
    data/defaults.yaml    the values its conditions read
  site/                   the project that uses it
    meridian.yaml         requires the module
    meridian.lock         written by `meridian module install`
    web.yaml              the project's own document
    data.yaml             the project's own values, which win over the module's
```

Here the module is required by `path`, so it is read where it is and the example
needs no network. A module from git or from a registry is used the same way; only
its entry in `site/meridian.yaml` differs. [Modules](../../docs/reference/modules.md)
has every form.

Run it from `site/`:

```
cd examples/modules/site
meridian module install
meridian compile web.yaml @acme/hardening:documents/ssh.yaml \
  --data @acme/hardening:data/defaults.yaml --data data.yaml
```

`@acme/hardening:documents/ssh.yaml` is the file `documents/ssh.yaml` in the module
`acme/hardening`. The module's data says root may not log in over SSH; `data.yaml`
does not change that, so the rule is compiled in.
