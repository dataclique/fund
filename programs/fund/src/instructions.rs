pub mod create_fund;
pub mod deposit;

// The globs carry Anchor's generated client-accounts modules to the crate
// root, where the #[program] macro requires them. Every instruction module
// also exports a `handler`, so the glob collision is expected — handlers are
// only ever called module-qualified (`<name>::handler`).
#[allow(ambiguous_glob_reexports)]
pub use create_fund::*;
#[allow(ambiguous_glob_reexports)]
pub use deposit::*;
