//! One declaration of the CLI's verbs, flags and examples.
//!
//! Help, shell completions and the man page are renderings of this table. A
//! verb added here is in every surface; a verb added only to `dispatch` is a
//! bug the terminal tests catch.

/// One command the compiler exposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub name: &'static str,
    pub summary: &'static str,
    pub usage: &'static str,
    pub flags: &'static [&'static str],
    /// Whether the verb takes a path argument.
    pub takes_path: bool,
}

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "build",
        summary: "compile a program to a binary",
        usage: "yashyc build <file.ya> [-o <out>] [-O0|-O1|-O2|-O3] [--timings] [--jobs <n>]",
        flags: &[
            "-o",
            "-O0",
            "-O1",
            "-O2",
            "-O3",
            "--release",
            "--timings",
            "--debug-info",
            "--print-capabilities",
            "--features",
            "--no-default-features",
            "--jobs",
            "--target",
            "--emit-ir",
            "--opt-report",
            "--no-verify",
            "--reproducible",
        ],
        takes_path: true,
    },
    CommandSpec {
        name: "run",
        summary: "build and run a program",
        usage: "yashyc run <file.ya> [--debug] [--sandbox] [-- <args>...]",
        flags: &[
            "--debug",
            "--sandbox",
            "--timings",
            "--debug-info",
            "--jobs",
            "--target",
            "--reproducible",
        ],
        takes_path: true,
    },
    CommandSpec {
        name: "check",
        summary: "check a program and print diagnostics",
        usage: "yashyc check <file.ya> [--json] [--timings] [--jobs <n>] [--features <list>]",
        flags: &[
            "--json",
            "--timings",
            "--debug-info",
            "--jobs",
            "--target",
            "--deny",
            "--allow",
            "--features",
            "--no-default-features",
            "--print-cfg",
        ],
        takes_path: true,
    },
    CommandSpec {
        name: "verify",
        summary: "check and verify a program's proofs",
        usage: "yashyc verify <file.ya> [--json] [--inventory] [--features <list>]",
        flags: &[
            "--json",
            "--timings",
            "--jobs",
            "--inventory",
            "--features",
            "--no-default-features",
        ],
        takes_path: true,
    },
    CommandSpec {
        name: "test",
        summary: "compile and run the programs under tests/",
        usage: "yashyc test [path] [--workspace|-p <member>] [--filter <substring>] [--list] [--json] [--timings]",
        flags: &[
            "--workspace",
            "-p",
            "--filter",
            "--list",
            "--json",
            "--timings",
            "--debug-info",
            "--jobs",
            "--sandbox",
            "--target",
        ],
        takes_path: false,
    },
    CommandSpec {
        name: "serve",
        summary: "keep a warm compiler daemon",
        usage: "yashyc serve [path]",
        flags: &[],
        takes_path: false,
    },
    CommandSpec {
        name: "watch",
        summary: "re-check sources on change",
        usage: "yashyc watch [path] [--json] [--run] [--test]",
        flags: &["--json", "--run", "--test", "--interval"],
        takes_path: false,
    },
    CommandSpec {
        name: "cache",
        summary: "inspect the content-addressed cache",
        usage: "yashyc cache verify <file.ya>",
        flags: &[],
        takes_path: false,
    },
    CommandSpec {
        name: "explain",
        summary: "print the long-form page for an error code",
        usage: "yashyc explain [--all] [--json] <code>",
        flags: &["--all", "--json"],
        takes_path: false,
    },
    CommandSpec {
        name: "init",
        summary: "create a new project",
        usage: "yashyc init [path]",
        flags: &[],
        takes_path: false,
    },
    CommandSpec {
        name: "new",
        summary: "create a project from a template",
        usage: "yashyc new <name> [--template <name>]",
        flags: &["--template"],
        takes_path: false,
    },
    CommandSpec {
        name: "debug",
        summary: "start a native debugger with a generated script",
        usage: "yashyc debug <file.ya> [--break <fn>]... [-- <args>...]",
        flags: &["--break", "--binary"],
        takes_path: true,
    },
    CommandSpec {
        name: "repl",
        summary: "check and run expressions with retained declarations",
        usage: "yashyc repl",
        flags: &[],
        takes_path: false,
    },
    CommandSpec {
        name: "fmt",
        summary: "format Yashy sources",
        usage: "yashyc fmt [-w] <files...>",
        flags: &["-w", "--check"],
        takes_path: true,
    },
    CommandSpec {
        name: "completions",
        summary: "print a shell completion script",
        usage: "yashyc completions <bash|zsh|fish|powershell>",
        flags: &[],
        takes_path: false,
    },
    CommandSpec {
        name: "help",
        summary: "print this help, or help for one verb",
        usage: "yashyc help [verb]",
        flags: &[],
        takes_path: false,
    },
    CommandSpec {
        name: "doctor",
        summary: "report the toolchain's health",
        usage: "yashyc doctor [--json]",
        flags: &["--json"],
        takes_path: false,
    },
    CommandSpec {
        name: "lsp",
        summary: "run the language server over stdio",
        usage: "yashyc lsp [--stdio]",
        flags: &["--stdio"],
        takes_path: false,
    },
    CommandSpec {
        name: "mcp",
        summary: "run the compiler's MCP server",
        usage: "yashyc mcp",
        flags: &[],
        takes_path: false,
    },
    CommandSpec {
        name: "doc",
        summary: "render the documentation tables",
        usage: "yashyc doc [--json] [--serve]",
        flags: &["--json", "--serve", "--port"],
        takes_path: false,
    },
    CommandSpec {
        name: "profile",
        summary: "report where a build spent its time",
        usage: "yashyc profile [--report <json>|--alloc-report <file.ya>]",
        flags: &["--json", "--sample", "--report", "--alloc-report"],
        takes_path: false,
    },
    CommandSpec {
        name: "tokens",
        summary: "dump a file's tokens or regenerate the editor grammars",
        usage: "yashyc tokens [--json] <file.ya> | tokens --grammar <dir> | tokens --bpe <path>...",
        flags: &["--json", "--grammar", "--bpe"],
        takes_path: true,
    },
    CommandSpec {
        name: "trace",
        summary: "print the rule matrix or a runtime event trace",
        usage: "yashyc trace [--json] [--fail-orphans] [root] | trace --events <file.ya> [-- --args]",
        flags: &["--json", "--fail-orphans", "--events"],
        takes_path: false,
    },
    CommandSpec {
        name: "lint",
        summary: "report style and correctness advice",
        usage: "yashyc lint [--list|--docs] [--json] [--group <g>] [--deny <code>] <file.ya>",
        flags: &["--list", "--docs", "--json", "--group", "--deny"],
        takes_path: true,
    },
    CommandSpec {
        name: "fix",
        summary: "apply machine-applicable diagnostic edits",
        usage: "yashyc fix [--dry-run] <file.ya>",
        flags: &["--dry-run"],
        takes_path: true,
    },
    CommandSpec {
        name: "inspect",
        summary: "report a symbol's facts from the index",
        usage: "yashyc inspect [--human] <file.ya>",
        flags: &["--json", "--human", "--addr", "--binary", "--layout"],
        takes_path: true,
    },
    CommandSpec {
        name: "tasks",
        summary: "run the project's defined tasks",
        usage: "yashyc tasks <file.ya> [-- <args>...]",
        flags: &["--debug", "--sandbox", "--timings"],
        takes_path: true,
    },
    CommandSpec {
        name: "task",
        summary: "run one named compiler-verb task",
        usage: "yashyc task [--list] <name>",
        flags: &["--list"],
        takes_path: false,
    },
    CommandSpec {
        name: "expand",
        summary: "show a program after macro expansion",
        usage: "yashyc expand [--json] <file.ya>",
        flags: &["--json"],
        takes_path: true,
    },
    CommandSpec {
        name: "index",
        summary: "build the project's symbol index",
        usage: "yashyc index [--json] [path]",
        flags: &["--json"],
        takes_path: false,
    },
    CommandSpec {
        name: "outline",
        summary: "print a file's declaration outline",
        usage: "yashyc outline <file.ya>",
        flags: &[],
        takes_path: true,
    },
    CommandSpec {
        name: "graph",
        summary: "print the module graph",
        usage: "yashyc graph [--json|--dot] [path]",
        flags: &["--json", "--dot"],
        takes_path: false,
    },
    CommandSpec {
        name: "search",
        summary: "search the workspace or a registry",
        usage: "yashyc search [--registry <dir>] <query>",
        flags: &["--registry"],
        takes_path: false,
    },
    CommandSpec {
        name: "context",
        summary: "assemble a task-sized context bundle",
        usage: "yashyc context [--json] [--task <name>] [--budget <n>] [path]",
        flags: &["--json", "--task", "--budget"],
        takes_path: false,
    },
    CommandSpec {
        name: "lock",
        summary: "verify the project lock file",
        usage: "yashyc lock --check [path]",
        flags: &["--check"],
        takes_path: false,
    },
    CommandSpec {
        name: "fetch",
        summary: "fetch the packages named by the lock",
        usage: "yashyc fetch [path]",
        flags: &[],
        takes_path: false,
    },
    CommandSpec {
        name: "update",
        summary: "resolve and write a new lock file",
        usage: "yashyc update [path]",
        flags: &[],
        takes_path: false,
    },
    CommandSpec {
        name: "add",
        summary: "add a dependency to the manifest",
        usage: "yashyc add <name> [--path <dir>|--git <url>] [--rev <commit>]",
        flags: &["--path", "--git", "--rev", "--registry", "--allow"],
        takes_path: false,
    },
    CommandSpec {
        name: "audit",
        summary: "audit dependencies for advisories",
        usage: "yashyc audit [--json] [--capabilities] [path]",
        flags: &["--json", "--capabilities"],
        takes_path: false,
    },
    CommandSpec {
        name: "sbom",
        summary: "print an SPDX SBOM for a package",
        usage: "yashyc sbom [dir] [--format spdx-json]",
        flags: &["--format"],
        takes_path: false,
    },
    CommandSpec {
        name: "verify-package",
        summary: "verify a package tarball's signature and SBOM",
        usage: "yashyc verify-package <tarball> [--registry <dir>]",
        flags: &["--registry"],
        takes_path: true,
    },
    CommandSpec {
        name: "self-update",
        summary: "verify and replace the running compiler binary",
        usage: "yashyc self-update --archive <file> --signature <file> [--dry-run]",
        flags: &["--archive", "--signature", "--dry-run"],
        takes_path: false,
    },
    CommandSpec {
        name: "outdated",
        summary: "compare locked versions with a registry index",
        usage: "yashyc outdated [--registry <dir>] [--json] [dir]",
        flags: &["--registry", "--json"],
        takes_path: false,
    },
    CommandSpec {
        name: "vendor",
        summary: "copy registry archives into a vendor directory",
        usage: "yashyc vendor <dir> --registry <dir>",
        flags: &["--registry"],
        takes_path: false,
    },
    CommandSpec {
        name: "tree",
        summary: "print the resolved dependency graph",
        usage: "yashyc tree [--depth <n>] [--json] [dir]",
        flags: &["--depth", "--json"],
        takes_path: false,
    },
    CommandSpec {
        name: "why",
        summary: "show every path to a dependency",
        usage: "yashyc why <package> [--json] [dir]",
        flags: &["--json"],
        takes_path: false,
    },
    CommandSpec {
        name: "info",
        summary: "print one locked package's facts",
        usage: "yashyc info <package> [--json] [dir]",
        flags: &["--json"],
        takes_path: false,
    },
    CommandSpec {
        name: "yank",
        summary: "mark a released version as yanked with a reason",
        usage: "yashyc yank <name>@<version> --registry <dir> --reason <text>",
        flags: &["--registry", "--reason"],
        takes_path: false,
    },
    CommandSpec {
        name: "remove",
        summary: "remove a dependency and keep a backup",
        usage: "yashyc remove <package> [dir]",
        flags: &[],
        takes_path: false,
    },
    CommandSpec {
        name: "attest",
        summary: "write a signed SLSA provenance statement",
        usage: "yashyc attest <artifact> --key-id <id> [--material <file>]...",
        flags: &["--key-id", "--material"],
        takes_path: true,
    },
    CommandSpec {
        name: "verify-attestation",
        summary: "verify a signed provenance statement",
        usage: "yashyc verify-attestation <artifact> --attestation <file> --signature <file>",
        flags: &["--attestation", "--signature", "--rebuild"],
        takes_path: true,
    },
    CommandSpec {
        name: "publish",
        summary: "publish the project to a registry",
        usage: "yashyc publish [path] --registry <dir> [--key-id <id>]",
        flags: &["--registry", "--key-id"],
        takes_path: false,
    },
    CommandSpec {
        name: "playground",
        summary: "serve the browser playground",
        usage: "yashyc playground [--host <addr>] [--port <n>]",
        flags: &["--host", "--port"],
        takes_path: false,
    },
    CommandSpec {
        name: "man",
        summary: "print the manual page",
        usage: "yashyc man",
        flags: &[],
        takes_path: false,
    },
    CommandSpec {
        name: "version",
        summary: "print the compiler version",
        usage: "yashyc version",
        flags: &[],
        takes_path: false,
    },
];

pub fn find(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|command| command.name == name)
}

/// The nearest verb to a misspelling, by edit distance.
pub fn closest(name: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .map(|command| (distance(name, command.name), command.name))
        .min_by_key(|(distance, _)| *distance)
        .filter(|(distance, _)| *distance <= 2)
        .map(|(_, name)| name)
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(previous + usize::from(ca != *cb));
            previous = old;
        }
    }
    row[b.len()]
}

/// Print one line per verb for `yashyc help`.
pub fn render_help() -> String {
    let width = COMMANDS
        .iter()
        .map(|command| command.name.len())
        .max()
        .unwrap_or(0);
    let mut out = String::from("usage: yashyc <command> [options]\n\ncommands:\n");
    for command in COMMANDS {
        out.push_str(&format!(
            "  {:<width$}  {}\n",
            command.name,
            command.summary,
            width = width
        ));
    }
    out
}

/// Print a single verb's usage and flags.
pub fn render_verb(name: &str) -> Option<String> {
    let command = find(name)?;
    let mut out = format!("{}\n\nflags:\n", command.usage);
    for flag in command.flags {
        out.push_str(&format!("  {flag}\n"));
    }
    Some(out)
}

/// Render the man page from the command table.
pub fn render_man() -> String {
    let mut out = String::from(".TH YASHC 1 \"2026\" \"Yashy\" \"Yashy Manual\"\n");
    out.push_str(".SH NAME\n");
    out.push_str("yashyc \\- the Yashy compiler\n");
    out.push_str(".SH SYNOPSIS\n");
    out.push_str(".B yashyc\n");
    out.push_str("<command> [options]\n");
    out.push_str(".SH DESCRIPTION\n");
    out.push_str("Yashy compiles a statically typed, ownership-explicit language to native binaries through LLVM.\n");
    out.push_str(".SH COMMANDS\n");
    for command in COMMANDS {
        out.push_str(&format!(".TP\n.B {}\n{}\n", command.name, command.summary));
    }
    out.push_str(".SH SEE ALSO\n");
    out.push_str("yashyc help, docs/tutorial/00-intro.md\n");
    out
}

/// Generate a completion script for one shell.
pub fn render_completions(shell: &str) -> Result<String, String> {
    let names: Vec<&str> = COMMANDS.iter().map(|command| command.name).collect();
    let all_flags: Vec<&str> = COMMANDS
        .iter()
        .flat_map(|command| command.flags.iter().copied())
        .collect();
    let unique_flags = {
        let mut flags = Vec::new();
        for flag in all_flags {
            if !flags.contains(&flag) {
                flags.push(flag);
            }
        }
        // Global output flags are accepted by every verb, so every completion
        // script offers them even though no single verb declares them.
        for flag in [
            "--color",
            "--no-color",
            "--pager",
            "--no-pager",
            "--hyperlink",
            "--no-hyperlink",
        ] {
            if !flags.contains(&flag) {
                flags.push(flag);
            }
        }
        flags
    };
    match shell {
        "bash" => {
            let mut out =
                String::from("_yashyc() {\n  local cur=${COMP_WORDS[COMP_CWORD]}\n  local verbs='");
            out.push_str(&names.join(" "));
            out.push_str("'\n  local flags='");
            out.push_str(&unique_flags.join(" "));
            out.push_str("'\n  if (( COMP_CWORD == 1 )); then\n    COMPREPLY=( $(compgen -W \"$verbs\" -- \"$cur\") )\n  else\n    COMPREPLY=( $(compgen -W \"$flags\" -- \"$cur\") )\n  fi\n}\ncomplete -F _yashyc yashyc\n");
            Ok(out)
        }
        "zsh" => {
            let mut out =
                String::from("#compdef yashyc\n_yashyc() {\n  local -a verbs flags\n  verbs=(");
            out.push_str(
                &names
                    .iter()
                    .map(|name| format!("'{name}'"))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            out.push_str(")\n  flags=(");
            out.push_str(
                &unique_flags
                    .iter()
                    .map(|flag| format!("'{flag}'"))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            out.push_str(")\n  if (( CURRENT == 2 )); then\n    _describe 'verb' verbs\n  else\n    _describe 'flag' flags\n  fi\n}\ncompdef _yashyc yashyc\n");
            Ok(out)
        }
        "fish" => {
            let mut out = String::new();
            for name in &names {
                out.push_str(&format!(
                    "complete -c yashyc -n '__fish_use_subcommand' -a '{name}'\n"
                ));
            }
            for flag in &unique_flags {
                out.push_str(&format!(
                    "complete -c yashyc -l '{}' -d 'compiler flag'\n",
                    flag.trim_start_matches('-')
                ));
            }
            Ok(out)
        }
        "powershell" => {
            let mut out = String::from("Register-ArgumentCompleter -Native -CommandName 'yashyc' -ScriptBlock {\n  param($wordToComplete)\n  @(");
            out.push_str(
                &names
                    .iter()
                    .map(|name| format!("'{name}'"))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push_str(") | Where-Object { $_ -like \"$wordToComplete*\" }\n}\n");
            Ok(out)
        }
        other => Err(format!(
            "unknown shell `{other}`; expected bash, zsh, fish or powershell"
        )),
    }
}
