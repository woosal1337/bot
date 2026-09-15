use std::fmt::{self, Display, Formatter};

use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{kind} ID cannot be empty")]
pub struct IdError {
    kind: &'static str,
}

macro_rules! string_id {
    ($name:ident, $kind:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(IdError { kind: $kind });
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

string_id!(AccountProfileId, "account profile");
string_id!(EventId, "event");
string_id!(SessionId, "session");
string_id!(TurnId, "turn");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_identifiers() {
        assert_eq!(SessionId::new("  "), Err(IdError { kind: "session" }));
    }

    #[test]
    fn preserves_identifier_values() {
        let id = EventId::new("event-7").expect("valid ID");
        assert_eq!(id.as_str(), "event-7");
        assert_eq!(id.to_string(), "event-7");
    }
}
