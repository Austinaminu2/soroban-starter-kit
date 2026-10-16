// `#[contracterror]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WrappedTokenError {
    /// `initialize` was called on an already-initialized contract.
    AlreadyInitialized = 1,
    /// An operation was attempted before the contract was initialized.
    NotInitialized = 2,
    /// Caller is not the admin.
    Unauthorized = 3,
    /// Amount is zero or negative.
    InvalidAmount = 4,
    /// Insufficient wrapped token balance to burn.
    InsufficientBalance = 5,
    /// Insufficient XLM in reserve to unwrap.
    InsufficientReserve = 6,
    /// Wrapping this amount would exceed the per-address cap set at `initialize`.
    MaxWrapExceeded = 7,
    /// An i128 supply or amount calculation overflowed.
    Overflow = 8,
    /// A fee is greater than 10_000 bps, or a non-zero fee was configured without a `fee_recipient`.
    InvalidFee = 9,
    /// A batch call was given an empty recipient list.
    EmptyBatch = 10,
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::WrappedTokenError;
    use std::format;
    use std::string::String;

    #[allow(clippy::as_conversions)]
    fn render_error_code_snapshot() -> String {
        format!(
            "\
WrappedTokenError::AlreadyInitialized = {}\n\
WrappedTokenError::NotInitialized = {}\n\
WrappedTokenError::Unauthorized = {}\n\
WrappedTokenError::InvalidAmount = {}\n\
WrappedTokenError::InsufficientBalance = {}\n\
WrappedTokenError::InsufficientReserve = {}\n\
WrappedTokenError::MaxWrapExceeded = {}\n",
            WrappedTokenError::AlreadyInitialized as u32,
            WrappedTokenError::NotInitialized as u32,
            WrappedTokenError::Unauthorized as u32,
            WrappedTokenError::InvalidAmount as u32,
            WrappedTokenError::InsufficientBalance as u32,
            WrappedTokenError::InsufficientReserve as u32,
            WrappedTokenError::MaxWrapExceeded as u32,
        )
    }

    #[test]
    fn wrapped_token_error_codes_match_snapshot() {
        assert_eq!(
            render_error_code_snapshot(),
            include_str!("../snapshots/error_codes.snap")
        );
    }
}
