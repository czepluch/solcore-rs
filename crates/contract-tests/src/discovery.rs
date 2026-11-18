//! Finds test contracts, and files that define contracts.

use hir::{ast::item::Item, input::SourceFile};

/// A contract with public functions whose names start with `test`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestContract {
    pub name: String,
    pub has_set_up: bool,
    /// The signatures of the test functions, such as `testTransfer()`.
    pub test_functions: Vec<String>,
}

/// The test contracts defined in `file`.
pub fn test_contracts(db: &dyn hir_ty::Db, file: SourceFile) -> Vec<TestContract> {
    let module = parser::parse_file_to_hir(db, file).module(db);
    module
        .items(db)
        .iter()
        .filter_map(|item| {
            let Item::ContractDef(contract) = item else {
                return None;
            };
            if contract.has_runtime_main(db) {
                return None;
            }
            let signatures = hir_ty::contract_dispatch_surface_for_module(db, module, *contract)
                .methods
                .iter()
                .map(|method| method.signature.clone())
                .collect::<Vec<_>>();
            let test_functions = signatures
                .iter()
                .filter(|signature| signature.starts_with("test"))
                .cloned()
                .collect::<Vec<_>>();
            (!test_functions.is_empty()).then(|| TestContract {
                name: contract.name_elem(db).atom().text(db).to_owned(),
                has_set_up: signatures.iter().any(|signature| signature == "setUp()"),
                test_functions,
            })
        })
        .collect()
}

/// Whether `file` defines at least one contract.
pub fn defines_contracts(db: &dyn hir_ty::Db, file: SourceFile) -> bool {
    parser::parse_file_to_hir(db, file)
        .module(db)
        .items(db)
        .iter()
        .any(|item| matches!(item, Item::ContractDef(_)))
}
