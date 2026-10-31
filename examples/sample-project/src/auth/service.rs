use crate::auth::login::login;
use crate::types::{Password, Session, Username};

pub fn authenticate(username: Username, password: Password) -> Session {
    login(username, password)
}

pub struct AuthService {
    realm: String,
}

impl AuthService {
    pub fn new(realm: &str) -> Self {
        AuthService {
            realm: realm.to_string(),
        }
    }

    /// Takes `&self`, so it needs `--ignore-self` to match `(Username, Password) -> Session`.
    pub fn session_for(&self, username: Username, password: Password) -> Session {
        let mut session = login(username, password);
        session.token.push_str(&self.realm);
        session
    }

    pub fn verify(&self, username: &Username, password: &Password) -> bool {
        !username.0.is_empty() && !password.0.is_empty()
    }
}
