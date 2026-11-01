//! R002 cases. Every function takes no parameters, so R001 never fires here.

pub fn no_docs() {}

/// Hi.
pub fn too_short() {}

/// Parses a token.
pub fn boundary_ok() {}

/// Reads a token.
pub fn boundary_short() {}

/// Create session.
pub fn create_session() {}

/// Closes the connection and flushes pending writes
pub fn no_period() {}

/// This sentence keeps going and going, describing the history of the project and every design decision in far more detail than any single intent line should ever carry, so readers must scroll sideways to finish it.
pub fn too_long() {}

/// Opens the store at the given location and returns a handle.
///
/// Fails when the location is not writable.
pub fn open_store() {}

// codesearch-lint: allow(R002)
pub fn undocumented_but_allowed() {}

///
/// Starts with a blank line.
pub fn blank_first() {}

// codesearch-lint: allow(R001, R002)
pub fn both_allowed(a: u64, b: u64, c: u64) -> u64 {
    0
}

fn private_helper() {}

pub(super) fn restricted() {}
