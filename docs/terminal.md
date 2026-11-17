# The terminal

`yashyc` treats a terminal as a place a person is watching and a pipe as a place
a program is reading. That one rule decides colour, width and every machine
format.

## Colour

Human diagnostics are coloured only when the stream is a terminal.

| Setting | Effect |
|---|---|
| `--color=always` | force SGR, even into a pipe |
| `--color=never`, `--no-color` | never emit an escape |
| `--color=auto` (default) | colour when stderr is a terminal |
| `--color=sometimes` | usage error, exit `2` |

The environment agrees with the flag: `NO_COLOR` (any value), `CLICOLOR=0` and
`TERM=dumb` all degrade to plain text. `--color=always` overrides the
environment, because an explicit flag is a stronger statement than an inherited
variable.

`--json` never contains an escape, whatever the terminal says. A machine stream
and a human stream are different renderings of one model, not one string with
colour spliced in.

```text
yashyc check --color=never main.ya
NO_COLOR=1 yashyc check main.ya
yashyc check --color=always main.ya | less -R
```

## Width

`COLUMNS` wraps a diagnostic that is wider than the shell. A caret line is never
wrapped: it is aligned to columns of the source excerpt above it, so splitting
it would point at the wrong byte.

```text
COLUMNS=40 yashyc check main.ya
```

## Global flags

`--color` and `--no-color` are global. They may appear before or after the verb,
and everything after `--` belongs to the program being run, so
`yashyc run main.ya -- --color=never` passes the flag through untouched.

## Shell completion

Four scripts are generated from the same command table that renders `yashyc
help`, `yashyc man` and the shell completion the CLI prints. The committed
copies live in [`scripts/completions`](../scripts/completions) and a test
compares them with the renderer byte for byte, so a verb added to the table
cannot be missing from a completion script.

| Shell | Install |
|---|---|
| bash | `source scripts/completions/yashyc.bash` — or permanently: `install -Dm644 scripts/completions/yashyc.bash ~/.local/share/bash-completion/completions/yashyc` |
| zsh | put `scripts/completions/yashyc.zsh` in a directory on `$fpath`, named `_yashyc` |
| fish | `cp scripts/completions/yashyc.fish ~/.config/fish/completions/yashyc.fish` |
| PowerShell | add `. scripts/completions/yashyc.ps1` to `$PROFILE` |

Or generate the current one instead of copying:

```text
yashyc completions bash       > ~/.local/share/bash-completion/completions/yashyc
yashyc completions zsh        > ~/.zfunc/_yashyc
yashyc completions fish       > ~/.config/fish/completions/yashyc.fish
yashyc completions powershell > yashyc-completions.ps1
```

Regenerate the repository copies after changing the command table:

```text
yashyc completions bash       > scripts/completions/yashyc.bash
yashyc completions zsh        > scripts/completions/yashyc.zsh
yashyc completions fish       > scripts/completions/yashyc.fish
yashyc completions powershell > scripts/completions/yashyc.ps1
```

## Paging

`yashyc explain` pages through `PAGER` when stdout is a terminal.

| Setting | Effect |
|---|---|
| `--pager` | page even if the policy would not |
| `--no-pager` | never page; the strongest form of the flag |
| `PAGER` | the program to run; unset means `more` on Windows and `less -R` elsewhere |
| `--json` | never pages |

A pipe never starts a pager, so `yashyc explain E0100 | head -20` prints the
page and stops, and a CI log holds the page rather than a child process's idea
of it. A pager the reader quits early is a normal end of reading, not an error.

## Hyperlinks

A diagnostic opens with `path:line:col`. On a terminal that understands OSC 8
it is also a link to the file at that position, so a click opens the editor
that registered the handler.

| Setting | Effect |
|---|---|
| `--hyperlink=always` | emit the link even into a pipe |
| `--hyperlink=never`, `--no-hyperlink` | never emit the link |
| `--hyperlink=auto` (default) | link when the terminal advertises OSC 8 |

The terminal is asked through positive signals: `WT_SESSION` (Windows
Terminal), `TERM_PROGRAM` (iTerm, WezTerm, VS Code, Hyper, Ghostty, Rio),
`KITTY_WINDOW_ID`, `VTE_VERSION` ≥ 5000, or a `TERM` naming kitty, alacritty,
wezterm or ghostty. Anything else gets plain text, because a stray escape is
worse than a link that does not exist.

The visible text never changes, so the log of a linked run is the same
`path:line:col` a plain pipe prints. `yashyc inspect --human` follows the same
rule for its source position.

## Manual page

`yashyc man` prints the roff page, rendered from the same table:

```text
yashyc man | man -l -
```
