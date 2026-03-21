use serde::{Deserialize, Serialize};

macro_rules! i64_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
        #[serde(transparent)]
        pub struct $name(pub i64);

        impl From<i64> for $name {
            fn from(value: i64) -> Self {
                Self(value)
            }
        }

        impl From<$name> for i64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

i64_id!(ThreadId);
i64_id!(TurnId);
i64_id!(ItemId);
i64_id!(CallId);
i64_id!(MessageId);
i64_id!(PartId);
