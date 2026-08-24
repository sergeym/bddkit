// The plugin layer is built bottom-up over several commits: the ABI types, the
// loader, and the registry all land before `main` wires any of it in. Scoped to
// this module so it cannot mask dead code anywhere else, and removed once the
// runner dispatches plugin steps.
#![allow(dead_code)]

pub mod abi;
pub mod library;
pub mod lock;
