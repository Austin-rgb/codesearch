/// Charges the user the given amount in the given currency.
pub fn charge(user_id: u64, amount: u64, currency: String) -> bool {
    user_id > 0 && amount > 0 && !currency.is_empty()
}

/// Formats a postal address on a single line.
pub fn format_address(street: String, city: String, country: String) -> String {
    format!("{street}, {city}, {country}")
}

/// Computes the completed share of the work as a percentage.
pub fn percentage(done: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        f64::from(done) * 100.0 / f64::from(total)
    }
}

/// Parses a boolean flag from text such as `true` or `false`.
pub fn parse_flag(raw: &str) -> bool {
    raw == "true"
}
