use crate::types::{AuthError, Password, Session, Username};

/// Creates a session for the given credentials.
pub fn login(username: Username, password: Password) -> Session {
    Session {
        token: format!("{}:{}", username.0, password.0.len()),
        user: username,
    }
}

/// Like `login`, but rejects empty passwords.
pub fn try_login(username: Username, password: Password) -> Result<Session, AuthError> {
    if password.0.is_empty() {
        return Err(AuthError::InvalidCredentials);
    }
    Ok(login(username, password))
}

/// Arguments are in the opposite order, so this does NOT match `(Username, Password) -> Session`.
pub fn login_swapped(password: Password, username: Username) -> Session {
    login(username, password)
}

/// The second parameter is arbitrary: found by `(Username, *) -> Session`.
pub fn login_with_label(username: Username, label: String) -> Session {
    Session {
        token: label,
        user: username,
    }
}
