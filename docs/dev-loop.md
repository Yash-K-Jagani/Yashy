# `yashyc dev` — the development loop

`yashyc dev [path] [--json] [--interval <ms>]` watches a program and restarts
it whenever its sources change.

- `path` may be a `.ya` file or a directory. A directory registers every
  `.ya` file under it, so a change to a sibling module, a template or a
  migration source triggers a rebuild too.
- Generation 1 is the initial build. Each later generation reports the files
  that changed, the build time and whether a replacement process is running.
- A failed build reports `diagnostics` and leaves the last working process
  alone; a fix restarts it.
- The loop uses the CLI's own `build` command and the existing watcher, so it
  shares the compiler pipeline with a one-shot build.

## JSON form

`--json` prints one object per generation:

```json
{"event":"generation","generation":1,"changed":["main.ya"],"status":"restarted","build_ms":412,"running":true}
```

`status` is `restarted` on a successful generation and `diagnostics` when the
build failed. This is the form an editor or an agent should consume.

## Restart behaviour

The old process is killed and waited for before the new one starts, so at most
one copy is alive at a time. A hung old process is bounded by the kill, not by
its own shutdown path. The listening socket is **not** inherited: portable
socket inheritance is not available on both target operating systems, so a
restart can drop an in-flight connection. A production service should use its
own drain contract (`packages/http/src/lifecycle.ya`) in front of the restart.
