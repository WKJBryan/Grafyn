mod canonical;
mod store;

pub use canonical::*;
#[allow(unused_imports)] // MCP compiles the shared seam before Task 7 injects it.
pub use store::*;

#[cfg(test)]
pub(crate) mod test_support;
