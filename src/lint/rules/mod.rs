//! Rule registry. To add a rule, implement [`Rule`](super::Rule) in a new module and add
//! one line to [`all`].

pub mod r001;
pub mod r002;

use super::Rule;

/// Every rule the linter knows about, in the order their findings are reported.
pub fn all() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(r001::PrimitiveHeavySignature),
        Box::new(r002::MissingIntentLine),
        // Extension point: register new rules here, one line each.
    ]
}
