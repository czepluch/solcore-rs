//! The `test` command: runs the test contracts of the input file.

use hir::{diag::DiagnosticLevel, input::SourceFile};
use nameres::LibraryId;
use solcore_contract_tests::{
    ContractReport, CreationBytecode, FuzzRuns, SetUpStep, TestKind, TestReport, abi,
    creation_bytecode, defines_contracts, merge_creation_bytecode, random_fuzz_seed,
    run_test_contracts, test_contracts,
};
use solcore_evm_backend::Backend;

use crate::{
    args::Args,
    db::DriverDb,
    diagnostics::{apply_warning_policy, render_diagnostics},
};

/// Compiles the contracts of every loaded main-library file, runs the test
/// contracts of `entry_file`, prints the results, and returns the exit code.
pub(crate) fn run(db: &DriverDb, entry_file: SourceFile, args: &Args) -> i32 {
    let backend = args.backend.unwrap_or(Backend::Sonatina);
    let mut all_creation_bytecode = CreationBytecode::new();
    for file in contract_files(db, entry_file) {
        let (program, mut diagnostics) =
            match compiler::build_checked_hull(db, file, args.specialize_options) {
                Ok(checked) => (Some(checked.program), checked.diagnostics),
                Err(diagnostics) => (None, diagnostics),
            };
        apply_warning_policy(&mut diagnostics, args.warning_policy);
        if !diagnostics.is_empty() {
            eprint!("{}", render_diagnostics(db, &diagnostics, args));
        }
        let Some(program) = program.filter(|_| {
            !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.level == DiagnosticLevel::Error)
        }) else {
            return 1;
        };
        merge_creation_bytecode(
            &mut all_creation_bytecode,
            creation_bytecode(backend, db, &program),
        );
    }

    let test_contracts = test_contracts(db, entry_file);
    if test_contracts.is_empty() {
        eprintln!(
            "no test contracts: no contract has a public function whose name starts with `test`"
        );
        return 1;
    }
    let fuzz_seed = args.fuzz_seed.unwrap_or_else(random_fuzz_seed);
    let reports = run_test_contracts(&all_creation_bytecode, &test_contracts, fuzz_seed);
    print!("{}", render_reports(&reports, fuzz_seed));
    if reports.iter().all(passed) { 0 } else { 1 }
}

/// The entry file, then every other loaded main-library file that defines
/// contracts.
fn contract_files(db: &DriverDb, entry_file: SourceFile) -> Vec<SourceFile> {
    let mut others = db
        .module_files
        .iter()
        .filter(|(key, file)| {
            key.library == LibraryId::Main && **file != entry_file && defines_contracts(db, **file)
        })
        .map(|(key, file)| (key.logical_path.clone(), *file))
        .collect::<Vec<_>>();
    others.sort_by(|(left, _), (right, _)| left.cmp(right));
    std::iter::once(entry_file)
        .chain(others.into_iter().map(|(_, file)| file))
        .collect()
}

fn passed(report: &ContractReport) -> bool {
    report.set_up.is_ok() && report.tests.iter().all(|test| test.result.is_ok())
}

/// Forge's layout: one line per test, a suite result per test contract, a
/// total, and the fuzz seed when a fuzz test ran.
fn render_reports(reports: &[ContractReport], fuzz_seed: u64) -> String {
    let mut output = String::new();
    let mut passed_tests = 0;
    let mut failed_tests = 0;
    for report in reports {
        output.push_str(&format!(
            "Ran {} for {}\n",
            count(report.tests.len(), "test"),
            report.contract
        ));
        if let Err(failure) = &report.set_up {
            let message = &failure.message;
            output.push_str(&match failure.step {
                SetUpStep::Compile => {
                    format!("[FAIL: does not compile: {message}] constructor()\n")
                }
                SetUpStep::Deploy => format!("[FAIL: {message}] constructor()\n"),
                SetUpStep::SetUp => format!("[FAIL: {message}] setUp()\n"),
            });
            failed_tests += 1;
        }
        for test in &report.tests {
            output.push_str(&render_test(test));
            output.push('\n');
            if test.result.is_ok() {
                passed_tests += 1;
            } else {
                failed_tests += 1;
            }
        }
        output.push_str(&format!(
            "Suite result: {}.\n\n",
            if passed(report) { "ok" } else { "FAILED" }
        ));
    }
    output.push_str(&format!(
        "Ran {}: {passed_tests} passed, {failed_tests} failed\n",
        count(reports.len(), "test contract")
    ));
    let ran_fuzz_tests = reports
        .iter()
        .flat_map(|report| &report.tests)
        .any(|test| matches!(test.kind, TestKind::Fuzz(_)));
    if ran_fuzz_tests {
        output.push_str(&format!("Fuzz seed: {fuzz_seed}\n"));
    }
    output
}

/// `[PASS] testName() (gas: 21000)`, or for a fuzz test
/// `[FAIL: message; counterexample: calldata=0x.. args=[1]] testName(uint256) (runs: 3, μ: 21000, ~: 21000)`.
fn render_test(test: &TestReport) -> String {
    let status = match (&test.result, &test.kind) {
        (Ok(()), _) => "PASS".to_owned(),
        (
            Err(message),
            TestKind::Fuzz(FuzzRuns {
                counterexample: Some(counterexample),
                ..
            }),
        ) => format!(
            "FAIL: {message}; counterexample: calldata={} args=[{}]",
            abi::to_hex(&counterexample.calldata),
            counterexample.arguments.join(", ")
        ),
        (Err(message), _) => format!("FAIL: {message}"),
    };
    let measurements = match &test.kind {
        TestKind::Unit { gas_used } => format!(" (gas: {gas_used})"),
        TestKind::Fuzz(FuzzRuns { runs: 0, .. }) => String::new(),
        TestKind::Fuzz(runs) => format!(
            " (runs: {}, μ: {}, ~: {})",
            runs.runs, runs.mean_gas, runs.median_gas
        ),
    };
    format!("[{status}] {}{measurements}", test.signature)
}

/// `1 test`, `2 tests`.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
