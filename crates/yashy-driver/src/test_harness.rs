//! Test harness v2: a project's whole test suite is compiled into one harness
//! binary and linked once. The harness runs every test in a single process,
//! printing a marker around each call so stdout is attributed to the test
//! that wrote it. If the harness exits non-zero or crashes, every test is
//! re-run in its own fresh process so a failure cannot disturb its siblings.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

use yashy_codegen::{check_main_exists, emit_program_with_entry, CodegenOptions};
use yashy_common::{Diagnostic, Json, SourceMap};
use yashy_verify::{ContractMode, VerifyOptions};

use crate::pipeline::{
    check_and_verify_timed, compile_hir_timed, link_directives, platform_library_flags, run_tool,
    BuildOptions, CpuTarget, OptLevel, TimingRow,
};
use crate::toolchain::Toolchain;
use yashy_runtime::{
    EXECUTOR_C_SOURCE, EXECUTOR_SOURCE_NAME, GRAPHEME_DATA_HEADER, GRAPHEME_DATA_HEADER_PATH,
    HASHMAP_C_SOURCE, HASHMAP_SOURCE_NAME, NET_C_SOURCE, NET_SOURCE_NAME, PLATFORM_C_SOURCE,
    PLATFORM_SOURCE_NAME, RUNTIME_C_SOURCE, RUNTIME_HEADER, RUNTIME_HEADER_NAME,
    RUNTIME_SOURCE_NAME,
};

/// Markers the harness prints around each test invocation.
pub const TEST_START: &str = "__YASHY_TEST_START__";
pub const TEST_END: &str = "__YASHY_TEST_END__";
const SUITE_END: &str = "__YASHY_SUITE_END__";

/// One discovered test. `index` is its position in the discovered list.
pub struct TestEntry {
    pub index: usize,
    /// The display path (`tests/foo.ya`), used as the human-facing name and in
    /// the harness markers.
    pub name: String,
    /// The file stem (`foo`), used as the `name` in schema-v1 JSON output.
    pub stem: String,
    pub source: PathBuf,
    /// The unique symbol its `main` was renamed to for the harness.
    pub symbol: String,
}

/// The linked harness and the tests it runs.
pub struct TestBuild {
    pub harness: PathBuf,
    pub tests: Vec<TestEntry>,
}

/// The verdict for one test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed,
    CompileError,
}

impl TestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestStatus::Passed => "passed",
            TestStatus::Failed => "failed",
            TestStatus::CompileError => "compile_error",
        }
    }
}

/// The result of running one test.
pub struct TestResult {
    pub name: String,
    pub file: String,
    pub status: TestStatus,
    /// The exit status the test returned, when it ran at all.
    pub exit: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: i64,
    pub diagnostics: Vec<Json>,
    /// Human-readable messages, used by the CLI's non-JSON rendering.
    pub messages: Vec<String>,
}

/// The result of running a whole suite.
pub struct TestReport {
    pub passed: usize,
    pub failed: usize,
    pub compile_errors: usize,
    pub results: Vec<TestResult>,
    pub timings: Vec<TimingRow>,
    /// A failure of the runner itself (for example, an unusable temp
    /// directory), not of any test. The CLI reports it as an internal error.
    pub internal_error: Option<String>,
}

const VERIFY_OPTIONS: VerifyOptions = VerifyOptions {
    contracts: ContractMode::Enforce,
};

/// Compile every test to an object, link one harness, and run the suite.
///
/// Tests that fail to compile or verify become per-test `compile_error`
/// results and are not part of the harness. Any global failure (a missing
/// toolchain, a failed link) falls back to running each test on its own.
pub fn run_suite(
    tests: &[PathBuf],
    toolchain: &Toolchain,
    target: &'static str,
    jobs: usize,
    sandbox_env: &[(&str, &str)],
) -> TestReport {
    let tmp = temp_harness_dir();
    if let Err(error) = fs::create_dir_all(&tmp) {
        // Without the directory no IR can be written. This is a failure of the
        // runner itself, not of any test; the CLI reports it as exit 3 with an
        // empty `tests` array, as `test --json` always has.
        return TestReport {
            passed: 0,
            failed: 0,
            compile_errors: 0,
            results: Vec::new(),
            timings: Vec::new(),
            internal_error: Some(format!("cannot create {}: {error}", tmp.display())),
        };
    }
    let mut timings: Vec<TimingRow> = Vec::new();
    let mut results: Vec<Option<TestResult>> = (0..tests.len()).map(|_| None).collect();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut compile_errors = 0usize;

    let build = match build_harness(&tmp, tests, toolchain, target, jobs, &mut timings) {
        Ok(build) => build,
        Err(global) => {
            // A global failure is not any one test's fault. Name it so a
            // failing CI run says what broke instead of only "tests failed".
            for diagnostic in &global {
                eprintln!("test harness: {}", diagnostic.message);
            }
            for (index, path) in tests.iter().enumerate() {
                let result = run_single(
                    path,
                    &stem_of(path),
                    &display_test_path(path),
                    &tmp,
                    toolchain,
                    target,
                    jobs,
                    sandbox_env,
                    &mut timings,
                );
                tally(&result, &mut passed, &mut failed, &mut compile_errors);
                results[index] = Some(result);
            }
            let _ = fs::remove_dir_all(&tmp);
            return TestReport {
                passed,
                failed,
                compile_errors,
                results: results.into_iter().flatten().collect(),
                timings,
                internal_error: None,
            };
        }
    };

    // Record the tests that never made it into the harness.
    for (index, diagnostics) in &build.compile_errors {
        let path = &tests[*index];
        results[*index] = Some(TestResult {
            name: stem_of(path),
            file: display_test_path(path),
            status: TestStatus::CompileError,
            exit: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 0,
            diagnostics: diagnostics.clone(),
            messages: messages_from_json(diagnostics),
        });
        compile_errors += 1;
    }

    match run_harness(&build.binary, &build.tests, sandbox_env) {
        Ok(HarnessOutcome::AllPassed(per_test)) => {
            for (position, entry) in build.tests.iter().enumerate() {
                let run = &per_test[position];
                let result = TestResult {
                    name: entry.stem.clone(),
                    file: entry.name.clone(),
                    status: TestStatus::Passed,
                    exit: Some(run.exit),
                    stdout: run.stdout.clone(),
                    stderr: run.stderr.clone(),
                    duration_ms: 0,
                    diagnostics: Vec::new(),
                    messages: Vec::new(),
                };
                results[entry.index] = Some(result);
                passed += 1;
            }
        }
        Err(error) => {
            eprintln!(
                "test harness: cannot start {}: {error}",
                build.binary.display()
            );
            for entry in &build.tests {
                let result = run_single(
                    &entry.source,
                    &entry.stem,
                    &entry.name,
                    &tmp,
                    toolchain,
                    target,
                    jobs,
                    sandbox_env,
                    &mut timings,
                );
                tally(&result, &mut passed, &mut failed, &mut compile_errors);
                results[entry.index] = Some(result);
            }
        }
        Ok(HarnessOutcome::Fallback) => {
            // The harness exited non-zero or a marker was missing: re-run every
            // test in its own process so each failure is attributed precisely.
            for entry in &build.tests {
                let result = run_single(
                    &entry.source,
                    &entry.stem,
                    &entry.name,
                    &tmp,
                    toolchain,
                    target,
                    jobs,
                    sandbox_env,
                    &mut timings,
                );
                tally(&result, &mut passed, &mut failed, &mut compile_errors);
                results[entry.index] = Some(result);
            }
        }
    }

    let _ = fs::remove_dir_all(&tmp);
    TestReport {
        passed,
        failed,
        compile_errors,
        results: results.into_iter().flatten().collect(),
        timings,
        internal_error: None,
    }
}

fn tally(result: &TestResult, passed: &mut usize, failed: &mut usize, compile_errors: &mut usize) {
    match result.status {
        TestStatus::Passed => *passed += 1,
        TestStatus::Failed => *failed += 1,
        TestStatus::CompileError => *compile_errors += 1,
    }
}

/// Build the harness: the linked binary, the tests it contains, and the
/// tests that failed to compile (with rendered diagnostics).
pub struct BuiltHarness {
    pub binary: PathBuf,
    pub tests: Vec<TestEntry>,
    pub compile_errors: Vec<(usize, Vec<Json>)>,
}

pub fn build_harness(
    tmp_dir: &Path,
    tests: &[PathBuf],
    toolchain: &Toolchain,
    target: &'static str,
    jobs: usize,
    timings: &mut Vec<TimingRow>,
) -> Result<BuiltHarness, Vec<Diagnostic>> {
    let mut entries: Vec<(TestEntry, PathBuf)> = Vec::new();
    let mut compile_errors: Vec<(usize, Vec<Json>)> = Vec::new();
    let mut libs: Vec<String> = Vec::new();
    // IR is written here and compiled to objects in one bounded worker pool
    // below. A hundred tiny modules must not cost a hundred clang processes
    // started one after another.
    let mut pending: Vec<(usize, String, String, PathBuf, String, PathBuf, PathBuf)> = Vec::new();

    let opt = OptLevel::None;
    let cpu = CpuTarget::default();

    for (index, path) in tests.iter().enumerate() {
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                compile_errors.push((
                    index,
                    vec![json_error(format!(
                        "cannot read {}: {error}",
                        path.display()
                    ))],
                ));
                continue;
            }
        };
        let name = display_test_path(path);
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(name.as_str())
            .to_string();
        let symbol = format!("__yashy_test_{index}");

        let mut sources = SourceMap::new();
        // Diagnostics name the display path (`tests/foo.ya`) on every OS, so
        // the checker gets that spelling rather than the native path.
        let entry_str = name.clone();
        let mut phases: Vec<TimingRow> = Vec::new();
        let outcome = check_and_verify_timed(
            &mut sources,
            Path::new(&name),
            source,
            false,
            target,
            &VERIFY_OPTIONS,
            jobs,
            &mut phases,
        );
        accumulate(timings, &phases);
        let (hir, report) = match outcome {
            Ok(result) => result,
            Err(diagnostics) => {
                compile_errors.push((index, crate::diagnostics_json(&sources, &diagnostics)));
                continue;
            }
        };
        if report.error_count() > 0 {
            compile_errors.push((
                index,
                crate::diagnostics_json(&sources, &report.diagnostics),
            ));
            continue;
        }
        let facts = report.facts;

        for flag in link_directives(&hir) {
            if !libs.contains(&flag) {
                libs.push(flag);
            }
        }

        // The test is checked as an ordinary program with its `main` intact.
        // Only the emitted symbol is renamed, so codegen still gives it the
        // entry-point signature and runtime initialisation.
        if let Err(diagnostic) = check_main_exists(&hir.items) {
            compile_errors.push((index, vec![crate::diagnostic_json(&sources, &diagnostic)]));
            continue;
        }

        let ir = emit_program_with_entry(
            &hir,
            &entry_str,
            CodegenOptions {
                target,
                ..Default::default()
            },
            &facts,
            &symbol,
        );
        let ir_path = tmp_dir.join(format!("test_{index}.ll"));
        if let Err(error) = fs::write(&ir_path, ir.as_bytes()) {
            compile_errors.push((
                index,
                vec![json_error(format!(
                    "cannot write {}: {error}",
                    ir_path.display()
                ))],
            ));
            continue;
        }

        let object = tmp_dir.join(format!("test_{index}.o"));
        pending.push((
            index,
            name.clone(),
            stem.clone(),
            path.clone(),
            symbol.clone(),
            ir_path,
            object,
        ));
    }

    // Compile every test's IR in parallel, bounded by `--jobs` (0 = all cores).
    // Deterministic order is restored by indexing the results, never by which
    // worker finished first.
    let workers = if jobs == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    } else {
        jobs
    }
    .max(1)
    .min(pending.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let opaque = crate::pipeline::opaque_pointers_args(toolchain);
    let compiled: std::sync::Mutex<Vec<Option<(TestEntry, PathBuf)>>> =
        std::sync::Mutex::new((0..pending.len()).map(|_| None).collect());
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..workers {
            let next = &next;
            let pending = &pending;
            let opaque = &opaque;
            let compiled = &compiled;
            handles.push(scope.spawn(move || {
                let mut failures = Vec::new();
                loop {
                    let slot = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((index, name, stem, source, symbol, ir_path, object)) =
                        pending.get(slot)
                    else {
                        break;
                    };
                    let mut args = vec![
                        "-x".to_string(),
                        "ir".to_string(),
                        ir_path.to_string_lossy().into_owned(),
                        "-c".to_string(),
                        "-o".to_string(),
                        object.to_string_lossy().into_owned(),
                        format!("--target={target}"),
                        opt.flag().to_string(),
                    ];
                    args.extend(opaque.iter().map(|flag| flag.to_string()));
                    let result = run_tool(toolchain, &args, name, "compiling a test", None);
                    let entry = match result {
                        Ok(()) => Some((
                            TestEntry {
                                index: *index,
                                name: name.clone(),
                                stem: stem.clone(),
                                source: source.clone(),
                                symbol: symbol.clone(),
                            },
                            object.clone(),
                        )),
                        Err(diagnostic) => {
                            failures.push((
                                *index,
                                crate::diagnostics_json(&SourceMap::new(), &diagnostic),
                            ));
                            None
                        }
                    };
                    compiled
                        .lock()
                        .expect("the compile-result lock was poisoned")[slot] = entry;
                }
                failures
            }));
        }
        for handle in handles {
            if let Ok(failures) = handle.join() {
                compile_errors.extend(failures);
            }
        }
    });
    for slot in compiled
        .into_inner()
        .expect("the compile-result lock was poisoned")
    {
        if let Some(entry) = slot {
            entries.push(entry);
        }
    }

    let binary = link_harness(
        tmp_dir, &entries, toolchain, target, &opt, &cpu, &libs, timings,
    )?;
    Ok(BuiltHarness {
        binary,
        tests: entries.into_iter().map(|(entry, _)| entry).collect(),
        compile_errors,
    })
}

/// What running the harness once produced.
enum HarnessOutcome {
    /// Every test returned zero; per-test stdout was attributed.
    AllPassed(Vec<HarnessRun>),
    /// The harness exited non-zero or crashed, so per-test runs are needed.
    Fallback,
}

struct HarnessRun {
    exit: i32,
    stdout: String,
    stderr: String,
}

/// Link the harness `main` and every test object into one binary.
fn link_harness(
    tmp_dir: &Path,
    entries: &[(TestEntry, PathBuf)],
    toolchain: &Toolchain,
    target: &'static str,
    opt: &OptLevel,
    cpu: &CpuTarget,
    libs: &[String],
    timings: &mut Vec<TimingRow>,
) -> Result<PathBuf, Vec<Diagnostic>> {
    let harness_src = tmp_dir.join("harness.c");
    let mut src =
        String::from("#include <stdio.h>\nextern void yashy_args_init(int argc, char** argv);\n");
    for (entry, _) in entries {
        src.push_str(&format!(
            "extern int {}(int argc, char** argv);\n",
            entry.symbol
        ));
    }
    src.push_str(
        "int main(int argc, char** argv) {\n    yashy_args_init(argc, argv);\n    int any_failed = 0;\n",
    );
    for (index, (entry, _)) in entries.iter().enumerate() {
        let name = c_string(&entry.name);
        src.push_str(&format!(
            "    printf(\"{s} %s\\n\", \"{name}\"); fflush(stdout);\n\
             \x20   int rc_{index} = {symbol}(0, NULL);\n\
             \x20   printf(\"{e} %s %d\\n\", \"{name}\", rc_{index}); fflush(stdout);\n\
             \x20   if (rc_{index} != 0) any_failed = 1;\n",
            s = TEST_START,
            e = TEST_END,
            name = name,
            symbol = entry.symbol,
        ));
    }
    src.push_str(&format!(
        "    printf(\"{SUITE_END} %d\\n\", any_failed); fflush(stdout);\n"
    ));
    src.push_str("    return any_failed;\n}\n");
    fs::write(&harness_src, src.as_bytes())
        .map_err(|error| vec![diagnostic(format!("cannot write the harness: {error}"))])?;

    let harness_object = tmp_dir.join("harness.o");
    let args = vec![
        "-c".to_string(),
        harness_src.to_string_lossy().into_owned(),
        "-o".to_string(),
        harness_object.to_string_lossy().into_owned(),
        format!("--target={target}"),
        opt.flag().to_string(),
    ];
    if let Err(diagnostic) = run_tool(toolchain, &args, "harness", "compiling the harness", None) {
        return Err(diagnostic);
    }

    let runtime_dir = tmp_dir.join("runtime");
    write_runtime_sources(&runtime_dir)
        .map_err(|error| vec![diagnostic(format!("cannot write the runtime: {error}"))])?;

    // Reuse the compiler's cached runtime objects when they exist: it is what
    // the normal build does, and it keeps a suite from recompiling the C
    // runtime once per harness link.
    let mut runtime_inputs: Vec<PathBuf> = Vec::new();
    if let Some(cache_dir) = crate::pipeline::runtime_cache_dir() {
        let key = crate::pipeline::runtime_cache_key(&toolchain.llc, target, *opt);
        let sources: [(&str, &str); 5] = [
            (RUNTIME_SOURCE_NAME, "runtime"),
            (HASHMAP_SOURCE_NAME, "hashmap"),
            (EXECUTOR_SOURCE_NAME, "executor"),
            (NET_SOURCE_NAME, "net"),
            (PLATFORM_SOURCE_NAME, "platform"),
        ];
        let mut complete = true;
        for (name, stem) in sources {
            match crate::pipeline::cached_runtime_object(
                &cache_dir,
                &runtime_dir.join(name),
                stem,
                &runtime_dir,
                key,
                target,
                *opt,
                toolchain,
                &[],
                None,
            ) {
                Some(object) => runtime_inputs.push(object),
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if !complete {
            runtime_inputs.clear();
        }
    }

    let mut inputs: Vec<String> = vec![harness_object.to_string_lossy().into_owned()];
    for (_, object) in entries {
        inputs.push(object.to_string_lossy().into_owned());
    }
    if runtime_inputs.is_empty() {
        for name in [
            RUNTIME_SOURCE_NAME,
            HASHMAP_SOURCE_NAME,
            EXECUTOR_SOURCE_NAME,
            NET_SOURCE_NAME,
            PLATFORM_SOURCE_NAME,
        ] {
            inputs.push(runtime_dir.join(name).to_string_lossy().into_owned());
        }
    } else {
        for object in runtime_inputs {
            inputs.push(object.to_string_lossy().into_owned());
        }
    }

    let mut all_libs: Vec<String> = libs.to_vec();
    for flag in platform_library_flags(target) {
        if !all_libs.contains(&flag.to_string()) {
            all_libs.push(flag.to_string());
        }
    }

    let out = tmp_dir.join(format!("harness{}", std::env::consts::EXE_SUFFIX));
    let mut args = vec![opt.flag().to_string(), format!("--target={target}")];
    // Same compatibility flag as the test objects and the normal link.
    args.extend(
        crate::pipeline::opaque_pointers_args(toolchain)
            .into_iter()
            .map(str::to_string),
    );
    for flag in cpu.flags_for(target) {
        args.push(flag.to_string());
    }
    args.extend(inputs.iter().cloned());
    args.push(format!("-I{}", runtime_dir.display()));
    args.push("-o".to_string());
    args.push(out.to_string_lossy().into_owned());
    args.extend(all_libs.into_iter());

    crate::pipeline::timed(timings, "link", || {
        run_tool(toolchain, &args, "harness", "linking the harness", None)
    })?;
    Ok(out)
}

/// Run the harness once and attribute its stdout to each test.
fn run_harness(
    harness: &Path,
    tests: &[TestEntry],
    envs: &[(&str, &str)],
) -> std::io::Result<HarnessOutcome> {
    let output = run_binary(harness, &[], envs)?;
    if !output.status.success() {
        return Ok(HarnessOutcome::Fallback);
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let mut runs: Vec<HarnessRun> = Vec::new();
    let mut cursor = 0usize;

    for entry in tests {
        let start = format!("{} {}\n", TEST_START, entry.name);
        let Some(rel) = stdout[cursor..].find(&start) else {
            return Ok(HarnessOutcome::Fallback);
        };
        let content_start = cursor + rel + start.len();
        let end = format!("{} {} ", TEST_END, entry.name);
        let Some(end_rel) = stdout[content_start..].find(&end) else {
            return Ok(HarnessOutcome::Fallback);
        };
        let end_pos = content_start + end_rel;
        let test_stdout = stdout[content_start..end_pos].to_string();
        let rc_start = end_pos + end.len();
        let rc_end = stdout[rc_start..]
            .find('\n')
            .map(|offset| rc_start + offset)
            .unwrap_or(stdout.len());
        let exit = stdout[rc_start..rc_end].trim().parse::<i32>().unwrap_or(1);
        cursor = (rc_end + 1).min(stdout.len());
        runs.push(HarnessRun {
            exit,
            stdout: test_stdout,
            stderr: String::new(),
        });
    }

    if let Some(first) = runs.first_mut() {
        first.stderr = stderr;
    }
    Ok(HarnessOutcome::AllPassed(runs))
}

/// Run one test as its own freshly built binary, exactly as the 1.0 runner
/// did, so a crashing or exiting test cannot disturb its siblings.
#[allow(clippy::too_many_arguments)]
pub fn run_single(
    path: &Path,
    name: &str,
    file: &str,
    tmp_dir: &Path,
    toolchain: &Toolchain,
    target: &'static str,
    jobs: usize,
    envs: &[(&str, &str)],
    timings: &mut Vec<TimingRow>,
) -> TestResult {
    let started = Instant::now();
    let mut result = TestResult {
        name: name.to_string(),
        file: file.to_string(),
        status: TestStatus::CompileError,
        exit: None,
        stdout: String::new(),
        stderr: String::new(),
        duration_ms: 0,
        diagnostics: Vec::new(),
        messages: Vec::new(),
    };

    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            result.diagnostics.push(json_error(error.to_string()));
            result.duration_ms = elapsed_ms(started);
            return result;
        }
    };

    let mut sources = SourceMap::new();
    let outcome = check_and_verify_timed(
        &mut sources,
        Path::new(file),
        source,
        false,
        target,
        &VERIFY_OPTIONS,
        jobs,
        timings,
    );
    let (hir, report) = match outcome {
        Ok(value) => value,
        Err(diagnostics) => {
            result.diagnostics = crate::diagnostics_json(&sources, &diagnostics);
            result.duration_ms = elapsed_ms(started);
            return result;
        }
    };
    if report.error_count() > 0 {
        result.diagnostics = crate::diagnostics_json(&sources, &report.diagnostics);
        result.duration_ms = elapsed_ms(started);
        return result;
    }
    let facts = report.facts;
    let warnings = crate::diagnostics_json(&sources, &report.diagnostics);
    result.messages = messages_from_json(&warnings);

    let exe = tmp_dir.join(format!(
        "single_{}{}",
        std::process::id(),
        std::env::consts::EXE_SUFFIX
    ));
    if let Err(diagnostic) = compile_hir_timed(
        &sources,
        &hir,
        file,
        &exe,
        toolchain,
        BuildOptions {
            opt: OptLevel::None,
            target,
            ..BuildOptions::default()
        },
        &facts,
        timings,
    ) {
        result.diagnostics = crate::diagnostics_json(&sources, &diagnostic);
        result.duration_ms = elapsed_ms(started);
        return result;
    }

    match run_binary(&exe, &[], envs) {
        Ok(output) => {
            result.exit = output.status.code();
            result.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            result.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            result.status = if output.status.success() {
                TestStatus::Passed
            } else {
                TestStatus::Failed
            };
            result.diagnostics = warnings;
        }
        Err(error) => {
            result.status = TestStatus::Failed;
            result.stderr = format!("cannot run the test: {error}");
        }
    }
    result.duration_ms = elapsed_ms(started);
    result
}

fn run_binary(exe: &Path, args: &[String], envs: &[(&str, &str)]) -> std::io::Result<Output> {
    let mut command = Command::new(exe);
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output()
}

fn temp_harness_dir() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("yashy-harness-{}-{stamp}", std::process::id()))
}

fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}

fn display_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn stem_of(path: &Path) -> String {
    match path.file_stem().and_then(|stem| stem.to_str()) {
        Some(stem) => stem.to_string(),
        None => path.to_string_lossy().into_owned(),
    }
}

fn c_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn diagnostic(message: String) -> Diagnostic {
    Diagnostic::new(
        yashy_common::Severity::Error,
        yashy_common::FileId::from_index(0),
        yashy_common::Span::SYNTHETIC,
        message,
    )
}

fn messages_from_json(diagnostics: &[Json]) -> Vec<String> {
    diagnostics
        .iter()
        .filter_map(|diagnostic| match diagnostic {
            Json::Object(fields) => {
                let message =
                    fields
                        .iter()
                        .find(|(key, _)| key == "message")
                        .and_then(|(_, value)| match value {
                            Json::String(message) => Some(message.clone()),
                            _ => None,
                        })?;
                let code = fields
                    .iter()
                    .find(|(key, _)| key == "code")
                    .and_then(|(_, value)| match value {
                        Json::String(code) => Some(code.clone()),
                        _ => None,
                    });
                Some(match code {
                    Some(code) => format!("{code}: {message}"),
                    None => message,
                })
            }
            _ => None,
        })
        .collect()
}

/// Add one test's phase rows to the suite's totals, keeping first-seen order.
/// A suite of N tests must report one `timing: load` line, not N.
fn accumulate(total: &mut Vec<TimingRow>, timings: &[TimingRow]) {
    for timing in timings {
        if let TimingRow::Phase { phase, ms } = timing {
            if let Some(TimingRow::Phase { ms: seen, .. }) = total
                .iter_mut()
                .find(|seen| matches!(seen, TimingRow::Phase { phase: p, .. } if p == phase))
            {
                *seen = seen.saturating_add(*ms);
                continue;
            }
        }
        total.push(timing.clone());
    }
}

fn json_error(message: String) -> Json {
    let mut object = Json::object();
    object.set("severity", Json::string("error"));
    object.set("message", Json::string(message));
    object
}

/// Write the C runtime sources once so every test symbol links against a
/// single copy of `yashy_print_str`, `yashy_alloc`, and friends.
fn write_runtime_sources(dir: &Path) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    fs::create_dir_all(dir.join("src"))?;
    fs::write(dir.join(RUNTIME_SOURCE_NAME), RUNTIME_C_SOURCE.as_bytes())?;
    fs::write(dir.join(HASHMAP_SOURCE_NAME), HASHMAP_C_SOURCE.as_bytes())?;
    fs::write(dir.join(EXECUTOR_SOURCE_NAME), EXECUTOR_C_SOURCE.as_bytes())?;
    fs::write(dir.join(NET_SOURCE_NAME), NET_C_SOURCE.as_bytes())?;
    fs::write(dir.join(PLATFORM_SOURCE_NAME), PLATFORM_C_SOURCE.as_bytes())?;
    fs::write(dir.join(RUNTIME_HEADER_NAME), RUNTIME_HEADER.as_bytes())?;
    fs::write(
        dir.join(GRAPHEME_DATA_HEADER_PATH),
        GRAPHEME_DATA_HEADER.as_bytes(),
    )?;
    Ok(dir.to_path_buf())
}
