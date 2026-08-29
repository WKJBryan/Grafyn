mod attention;
mod canonical;
mod projection;
mod proposals;
mod store;

pub use attention::*;
pub use canonical::*;
#[allow(unused_imports)] // Task 8 consumes projection entry points from command surfaces.
pub use projection::*;
pub use proposals::*;
#[allow(unused_imports)] // MCP compiles the shared seam before Task 7 injects it.
pub use store::*;

#[cfg(test)]
pub(crate) mod test_support;
