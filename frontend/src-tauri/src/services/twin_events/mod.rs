mod attention;
mod canonical;
mod capture;
mod journal;
mod mutation_coordinator;
mod projection;
mod proposals;
mod secure_fs;
mod store;

pub use attention::*;
pub use canonical::*;
pub use capture::*;
pub use journal::*;
pub use mutation_coordinator::*;
#[allow(unused_imports)] // Task 8 consumes projection entry points from command surfaces.
pub use projection::*;
pub use proposals::*;
pub(crate) use secure_fs::*;
#[allow(unused_imports)] // MCP compiles the shared seam before Task 7 injects it.
pub use store::*;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod secure_fs_contract_tests;
