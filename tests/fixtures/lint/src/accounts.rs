//! Fixture for the lint integration tests. It is read by codesearch, never compiled.

/// Transfers money between two accounts.
pub fn transfer(from: u64, to: u64, amount: u64) -> bool {
    todo!()
}

/// Looks up the account that owns the given identifier.
pub fn find_account(account: AccountId) -> Option<Account> {
    todo!()
}

/// Checks whether a number is even.
pub fn is_even(n: u32) -> bool {
    todo!()
}

/// Describes a value using a label and a width.
pub fn describe(label: String, width: u8) -> String {
    todo!()
}

/// Sums up the bytes of a buffer.
pub fn checksum(data: Vec<u8>) -> u32 {
    todo!()
}

/// Finds the email address registered for an id.
pub fn lookup_email(id: u64) -> Option<String> {
    todo!()
}

// codesearch-lint: allow(R001)
/// Swaps two labels without complaint.
pub fn swap_labels(left: String, right: String) -> (String, String) {
    todo!()
}

/// Not public API, so it is never flagged.
pub(crate) fn internal(a: u64, b: u64) -> u64 {
    todo!()
}

fn helper(a: u64, b: u64) -> u64 {
    todo!()
}

pub struct Ledger;

impl Ledger {
    /// Records a debit against the given account.
    pub fn debit(&self, account: AccountId, amount: u64) -> bool {
        todo!()
    }
}

impl std::fmt::Display for Ledger {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        todo!()
    }
}
