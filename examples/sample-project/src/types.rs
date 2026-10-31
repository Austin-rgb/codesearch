#[derive(Debug, Clone, PartialEq)]
pub struct Username(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct Password(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub user: Username,
    pub token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantId(pub u64);

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: UserId,
    pub name: Username,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserDetails {
    pub user: User,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Metadata {
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    InvalidCredentials,
    UserNotFound,
}
