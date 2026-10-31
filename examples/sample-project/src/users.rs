use crate::types::{AuthError, Metadata, TenantId, User, UserDetails, UserId, Username};

pub fn get_user(id: UserId) -> Option<UserDetails> {
    if id.0 == 0 {
        None
    } else {
        Some(details_for(id))
    }
}

/// Same lookup, but with a `Result` return type: does NOT match `UserId -> Option<UserDetails>`.
pub fn find_user(id: UserId) -> Result<UserDetails, AuthError> {
    get_user(id).ok_or(AuthError::UserNotFound)
}

fn details_for(id: UserId) -> UserDetails {
    UserDetails {
        user: User {
            id,
            name: Username(format!("user{}", id.0)),
        },
        email: format!("user{}@example.com", id.0),
    }
}

pub fn user_and_tenant(user: UserId, tenant: TenantId) -> (User, Metadata) {
    let details = details_for(user);
    (
        details.user,
        Metadata {
            tags: vec![format!("tenant:{}", tenant.0)],
        },
    )
}

pub struct UserRepo;

impl UserRepo {
    pub fn new() -> Self {
        UserRepo
    }

    pub fn lookup(&self, id: UserId) -> Option<UserDetails> {
        get_user(id)
    }

    pub fn tenant_user(&self, user: UserId, tenant: TenantId) -> (User, Metadata) {
        user_and_tenant(user, tenant)
    }
}
