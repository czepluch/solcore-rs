//! The `test` command: runs the test contracts of the input file.

use hir::{diag::DiagnosticLevel, input::SourceFile};
use nameres::LibraryId;
use solcore_contract_tests::{
    ContractReport, CreationBytecode, SetUpStep, creation_bytecode, defines_contracts,
    merge_creation_bytecode, run_test_contracts, test_contracts,
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
    let reports = run_test_contracts(&all_creation_bytecode, &test_contracts);
    print!("{}", render_reports(&reports));
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

fn render_reports(reports: &[ContractReport]) -> String {
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
            match &test.result {
                Ok(()) => {
                    output.push_str(&format!(
                        "[PASS] {} (gas: {})\n",
                        test.signature, test.gas_used
                    ));
                    passed_tests += 1;
                }
                Err(message) => {
                    output.push_str(&format!(
                        "[FAIL: {message}] {} (gas: {})\n",
                        test.signature, test.gas_used
                    ));
                    failed_tests += 1;
                }
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
    output
}

/// `1 test`, `2 tests`.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
