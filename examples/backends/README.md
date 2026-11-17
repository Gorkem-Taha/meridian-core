# State backends

A state backend is where Meridian keeps its own record: the lock, the
run-state and the history of runs. `local`, the state directory, is built in.
Any other is a program named `meridian-backend-<name>`.

| File | What it is |
|---|---|
| [`meridian-backend-dir`](meridian-backend-dir) | keeps the record in a directory, such as a mount several machines share. It is the reference implementation, and the one the tests prove the protocol against |

```
meridian apply site.yaml \
    --plugin-dir /usr/local/lib/meridian --plugin-dir examples/backends \
    --backend dir --backend-config path=/mnt/team/meridian
```

The contract a backend keeps is in
[the state reference](../../docs/reference/state.md#writing-a-backend). The
one thing it must get right is `create`: it has to be atomic, because the
lock is built on it.
