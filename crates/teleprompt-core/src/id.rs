//! The names a script gives its parts: a line, an action block, a shot of
//! one, and the timeline item they make. Each is its own type, so one cannot be passed for another, and
//! each is never empty: the parser gives every line and block one.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self(id.to_string())
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                Self(id)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<String> for $name {
            fn eq(&self, other: &String) -> bool {
                &self.0 == other
            }
        }
    };
}

id!(
    /// A line of narration: `{#welcome}`, or derived from its chapter.
    LineId
);
id!(
    /// An action block: `id=` on its fence, or derived from the line before.
    BlockId
);
id!(
    /// One shot of a block: `welcome-a#0`, the block and its index.
    ShotId
);

id!(
    /// A timeline item: its line's id when it has one, else its shot's.
    ItemId
);

impl From<LineId> for ItemId {
    fn from(id: LineId) -> Self {
        Self(id.0)
    }
}

impl From<ShotId> for ItemId {
    fn from(id: ShotId) -> Self {
        Self(id.0)
    }
}

impl ShotId {
    /// Shot `index` of `block`.
    pub fn of(block: &BlockId, index: usize) -> Self {
        Self(format!("{block}#{index}"))
    }

    /// The block it is a shot of: all of it, for an id with no `#`.
    pub fn block(&self) -> &str {
        self.0.split_once('#').map_or(&self.0, |(b, _)| b)
    }

    /// Its place in its block, if the id carries one.
    pub fn index(&self) -> Option<usize> {
        self.0.split_once('#')?.1.parse().ok()
    }
}
